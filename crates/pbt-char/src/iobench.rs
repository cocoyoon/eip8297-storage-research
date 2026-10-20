//! Timed IO on a real device for one per-block IO mix.
//!
//! The replay reduces each store at a DRAM budget to per-block counts:
//! random reads (and their size), random 4 KB page writes, sequential
//! bytes written and read, fsyncs. The cost model prices those counts with
//! an ASSUMED device profile. This replays the counts against a file of
//! full-state size with the page cache bypassed and times them, so the
//! profile can be checked on the device a node would run.
//!
//! It times the IO a store would issue, not a store: offsets are uniform
//! random over the file (the model's misses are of cold records spread
//! over the whole state); a block's reads, then its page writes, are issued
//! by `threads` workers in parallel; the log append and fsyncs close it.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct Mix {
    pub rand_reads: u64,
    pub read_size: u64,
    /// A second class of random reads, e.g. 512 B records beside 4 KB pages.
    pub rand_reads2: u64,
    pub read_size2: u64,
    pub rand_writes: u64,
    pub seq_write_bytes: u64,
    pub seq_read_bytes: u64,
    pub fsyncs: u64,
}

struct Buf {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl Buf {
    fn new(len: usize) -> Buf {
        let layout = std::alloc::Layout::from_size_align(len.max(4096), 4096).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        Buf { ptr, layout }
    }
    fn slice(&mut self, len: usize) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, len) }
    }
}

impl Drop for Buf {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr, self.layout) }
    }
}

fn open_direct(path: &str, len: u64) -> (File, bool) {
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(0x4000); // O_DIRECT
    }
    let f = o.open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    #[cfg(target_os = "macos")]
    let direct = {
        use std::os::fd::AsRawFd;
        extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        const F_NOCACHE: i32 = 48;
        unsafe { fcntl(f.as_raw_fd(), F_NOCACHE, 1) == 0 }
    };
    #[cfg(not(target_os = "macos"))]
    let direct = cfg!(target_os = "linux");
    if f.metadata().unwrap().len() < len {
        f.set_len(len).unwrap();
    }
    (f, direct)
}

/// Write the whole file once so reads hit allocated blocks, not holes.
pub fn fill(path: &str, len: u64) {
    let (f, _) = open_direct(path, len);
    let chunk = 8 << 20;
    let mut b = Buf::new(chunk);
    for (i, x) in b.slice(chunk).iter_mut().enumerate() {
        *x = (i * 31 + 7) as u8;
    }
    let mut off = 0;
    let t = Instant::now();
    while off < len {
        let n = chunk.min((len - off) as usize);
        f.write_all_at(b.slice(n), off).unwrap();
        off += n as u64;
        if off % (16 << 30) < chunk as u64 {
            eprintln!(
                "[iobench] filled {} GB ({:.0}s)",
                off >> 30,
                t.elapsed().as_secs_f64()
            );
        }
    }
    f.sync_all().unwrap();
}

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

pub struct Timing {
    pub direct: bool,
    pub per_block_us: Vec<f64>,
    pub read_us: f64,
    pub write_us: f64,
}

pub fn run(
    path: &str,
    len: u64,
    log_path: &str,
    mix: &Mix,
    blocks: u64,
    threads: usize,
    seed: u64,
) -> Timing {
    let (f, direct) = open_direct(path, len);
    let (log, _) = open_direct(log_path, 0);
    let read_size = mix.read_size.max(512).div_ceil(512) * 512;
    let slots = len / read_size;
    let read_size2 = mix.read_size2.max(512).div_ceil(512) * 512;
    let slots2 = len / read_size2;
    let pages = len / 4096;
    let seq_w = mix.seq_write_bytes.div_ceil(4096) * 4096;
    let mut wbuf = Buf::new(seq_w.max(4096) as usize);
    let mut sbuf = Buf::new(1 << 20);
    let mut log_off = 0u64;
    let mut seq_r_off = 0u64;
    let (mut per_block, mut rsum, mut wsum) = (Vec::new(), 0.0, 0.0);
    for b in 0..blocks {
        let t0 = Instant::now();
        // Reads: split across workers, each its own offsets.
        let next = AtomicU64::new(0);
        std::thread::scope(|sc| {
            for w in 0..threads {
                let (f, next) = (&f, &next);
                sc.spawn(move || {
                    let mut buf = Buf::new(read_size.max(read_size2) as usize);
                    let mut s = seed ^ (b << 20) ^ (w as u64 + 1) * 0x9e37_79b9_7f4a_7c15;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let (size, n) = if i < mix.rand_reads {
                            (read_size, slots)
                        } else if i < mix.rand_reads + mix.rand_reads2 {
                            (read_size2, slots2)
                        } else {
                            break;
                        };
                        let off = (rng(&mut s) % n) * size;
                        f.read_exact_at(buf.slice(size as usize), off).unwrap();
                    }
                });
            }
        });
        let t1 = Instant::now();
        let mut left = mix.seq_read_bytes;
        while left > 0 {
            let n = left.min(1 << 20).div_ceil(4096) * 4096;
            if seq_r_off + n > len {
                seq_r_off = 0;
            }
            f.read_exact_at(sbuf.slice(n as usize), seq_r_off).unwrap();
            seq_r_off += n;
            left = left.saturating_sub(n);
        }
        // Page writes of one commit are independent: same workers.
        let next = AtomicU64::new(0);
        std::thread::scope(|sc| {
            for w in 0..threads {
                let (f, next) = (&f, &next);
                sc.spawn(move || {
                    let mut buf = Buf::new(4096);
                    let mut s = seed ^ (b << 21) ^ (w as u64 + 7) * 0x2545_f491_4f6c_dd1d;
                    while next.fetch_add(1, Ordering::Relaxed) < mix.rand_writes {
                        let off = (rng(&mut s) % pages) * 4096;
                        f.write_all_at(buf.slice(4096), off).unwrap();
                    }
                });
            }
        });
        if seq_w > 0 {
            log.write_all_at(wbuf.slice(seq_w as usize), log_off)
                .unwrap();
            log_off += seq_w;
            if log_off > (64 << 30) {
                log_off = 0; // keep the log file bounded
            }
        }
        for _ in 0..mix.fsyncs {
            f.sync_data().unwrap();
            log.sync_data().unwrap();
        }
        let t2 = Instant::now();
        rsum += (t1 - t0).as_secs_f64() * 1e6;
        wsum += (t2 - t1).as_secs_f64() * 1e6;
        per_block.push((t2 - t0).as_secs_f64() * 1e6);
    }
    let n = blocks.max(1) as f64;
    Timing {
        direct,
        per_block_us: per_block,
        read_us: rsum / n,
        write_us: wsum / n,
    }
}
