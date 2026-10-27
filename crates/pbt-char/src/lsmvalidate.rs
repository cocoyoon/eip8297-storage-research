//! Feed a replay's record stream (`stream.rs`) to a real leveled merge
//! tree, RocksDB, and set what it writes beside what the report-time model
//! (`pbt_store::lsm`) predicts for the same stream and engine constants.
//!
//! The starting state is loaded first and compacted into the last level,
//! as a long-running node's state would sit. The blocks are then applied
//! one write batch each, and the engine's own statistics (bytes written by
//! flushes, compactions and the WAL, bytes read by compactions) are taken
//! at the start of the measured window and again after the engine has
//! finished what the window left pending. The model runs on the same
//! stream with the live entries of this database in place of the full
//! state's, so the two see the same key population and the same level
//! structure.
//!
//! Values are pseudo-random bytes of the record's serialized size less the
//! key, so an entry is the record plus the engine's own overhead, as the
//! model assumes; keys are the records' tree positions, laid out either as
//! paths (pre-order, as a geth or Nethermind node keys its trie) or as
//! (page, position in page) with six-level pages.

use crate::stream::{Event, StreamReader};
use pbt_store::lsm::{cost_from_curve, cost_with_levels, Buffer, DistinctCounter, LsmConfig};
use pbt_store::mrc::id64;
use rocksdb::{
    BlockBasedOptions, Cache, DBCompactionStyle, DBCompressionType, Options, WriteBatch,
    WriteOptions, DB,
};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyLayout {
    Path,
    Page,
    /// A 128-bit hash of the position: no locality at all, as a store keyed
    /// by record id would have.
    Hash,
}

pub struct Config {
    pub stream: String,
    pub db: String,
    pub label: String,
    pub out_dir: String,
    pub buffer_bytes: u64,
    pub l0_trigger: i32,
    pub level_base_bytes: u64,
    pub multiplier: f64,
    pub target_file_bytes: u64,
    pub key: KeyLayout,
    pub compression: DBCompressionType,
    pub compression_name: String,
    pub sync: bool,
    /// Blocks applied before the measured window opens. Default: half.
    pub measure_from: Option<u64>,
    /// Stop after this many blocks of the stream.
    pub max_blocks: Option<u64>,
    pub threads: i32,
    pub bloom_bits: f64,
    pub block_cache_bytes: u64,
    /// Walk the whole database after the starting state is loaded and
    /// count its keys, a check on the tracked live set (slow).
    pub count_keys: bool,
    /// Extra copies of the starting state, each under its key with one
    /// more byte so that it sorts beside the original, never updated: cold
    /// ballast that gives the tree the levels a bigger state would have,
    /// interleaved with the stream's keys as the untouched state is.
    pub ballast: u32,
    /// Replay the stream's blocks this many times over (the starting state
    /// once): a longer run than the stream, for levels that take longer
    /// than the stream to fill. Block numbers run on.
    pub repeat: u32,
    /// Index mode: the stream as the disk index of a log store sees it,
    /// 16-byte hashed keys with an 8-byte value (the log position), and
    /// `relocations` further puts per update to live entries chosen at
    /// random, ballast copies included: the records the log's garbage
    /// collection moves, whose index entries change. Forces hash keys.
    pub index_mode: bool,
    pub relocations: f64,
}

/// Microseconds since the UNIX epoch, the clock of the engine's event log.
fn unix_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

/// The engine key of a record at position `pos` (`encode_bit_prefix`).
///
/// Path: the position's bits in groups of eight bytes, each group followed
/// by a marker, 0xFF when more bits follow and otherwise the count of valid
/// bits in the group. Markers sit at the same offsets in every key, so a
/// node sorts before everything below it and before the subtree to its
/// right: pre-order, the order a geth or Nethermind node keeps its trie
/// in, with no padding to the longest position (a storage leaf's position
/// runs past 512 bits, a branch's rarely past 48).
///
/// Page: the same encoding of the bits of the six-level page the node
/// belongs to, then the page's depth and the node's position within the
/// page, so that a page's records are contiguous and pages are in
/// pre-order.
pub fn encode_key(pos: &[u8], layout: KeyLayout, out: &mut Vec<u8>) {
    out.clear();
    let bits = u16::from_be_bytes([pos[0], pos[1]]) as usize;
    let packed = &pos[2..];
    match layout {
        KeyLayout::Hash => hash_key(id64(pos), pos.len(), out),
        KeyLayout::Path => grouped(packed, bits, out),
        KeyLayout::Page => {
            // Depths 1..=6 lie in the first page below the root, 7..=12 in
            // the next, and so on; the root alone is depth 0.
            let page = bits.saturating_sub(1) / 6;
            let prefix_bits = page * 6;
            grouped(packed, prefix_bits, out);
            out.push(page as u8);
            let mut p = 1u8;
            for i in prefix_bits..bits {
                p = (p << 1) | ((packed[i / 8] >> (7 - i % 8)) & 1);
            }
            out.push(p);
        }
    }
}

/// The 16-byte hashed key of a position of `len` bytes whose `id64` is `h1`.
fn hash_key(h1: u64, len: usize, out: &mut Vec<u8>) {
    out.clear();
    let h2 = h1.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(32) ^ (len as u64);
    out.extend_from_slice(&h1.to_be_bytes());
    out.extend_from_slice(&h2.to_be_bytes());
}

fn xorshift(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}

/// `bits` leading bits of `packed`, in groups of eight bytes with markers.
fn grouped(packed: &[u8], bits: usize, out: &mut Vec<u8>) {
    let mut done = 0usize;
    loop {
        let start = done / 8;
        for i in 0..8 {
            let b = packed.get(start + i).copied().unwrap_or(0);
            // Bits past `bits` are not part of the position.
            let keep = bits.saturating_sub((start + i) * 8).min(8);
            out.push(if keep == 8 {
                b
            } else if keep == 0 {
                0
            } else {
                b & (0xFFu8 << (8 - keep))
            });
        }
        let valid = (bits - done).min(64);
        done += valid;
        if done < bits {
            out.push(0xFF);
        } else {
            out.push(valid as u8);
            break;
        }
    }
}

/// Entries flushed per update by a memtable of `updates` updates: the
/// distinct share of its window.
fn distinct_share(curve: &[(u64, f64)], updates: f64, _l: &pbt_store::lsm::LsmLevel) -> f64 {
    pbt_store::lsm::distinct_at(curve, updates) / updates
}

fn fill_value(buf: &mut Vec<u8>, len: usize, seed: u64) {
    buf.clear();
    let mut x = seed | 1;
    while buf.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf.truncate(len);
}

/// The engine's tickers, by name.
fn tickers(opts: &Options) -> HashMap<String, u64> {
    let s = opts.get_statistics().unwrap_or_default();
    let mut m = HashMap::new();
    for line in s.lines() {
        if let Some((name, rest)) = line.split_once(" COUNT : ") {
            if let Some(v) = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
            {
                m.insert(name.trim().to_string(), v);
            }
        }
    }
    m
}

/// Per level: (read GB, write GB, compactions, keys in, keys dropped).
type LevelStats = HashMap<String, (f64, f64, f64, f64, f64)>;

/// Per level from the engine's compaction table, cumulative since the
/// database was opened: (read GB, write GB, compactions, keys in, keys
/// dropped). Columns are found by name; the size column takes two tokens.
fn level_table(db: &DB) -> (String, LevelStats) {
    let stats = db
        .property_value("rocksdb.stats")
        .ok()
        .flatten()
        .unwrap_or_default();
    let table: Vec<&str> = stats
        .lines()
        .skip_while(|l| !l.starts_with("** Compaction Stats"))
        .skip(1)
        .take_while(|l| !l.is_empty())
        .collect();
    let mut m = HashMap::new();
    let Some(header) = table.first() else {
        return (String::new(), m);
    };
    let h: Vec<&str> = header.split_whitespace().collect();
    let col = |name: &str| h.iter().position(|c| *c == name).map(|i| i + 1);
    let (Some(ir), Some(iw), Some(ic), Some(iki), Some(ikd)) = (
        col("Read(GB)"),
        col("Write(GB)"),
        col("Comp(cnt)"),
        col("KeyIn"),
        col("KeyDrop"),
    ) else {
        return (table.join("\n"), m);
    };
    let num = |s: &str| -> f64 {
        let (v, mul) = match s.chars().last() {
            Some('K') => (&s[..s.len() - 1], 1e3),
            Some('M') => (&s[..s.len() - 1], 1e6),
            Some('G') => (&s[..s.len() - 1], 1e9),
            _ => (s, 1.0),
        };
        v.parse::<f64>().unwrap_or(0.0) * mul
    };
    for l in &table[2..] {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() <= ikd
            || !(f[0].starts_with('L') && f[0][1..].chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        m.insert(
            f[0].to_string(),
            (num(f[ir]), num(f[iw]), num(f[ic]), num(f[iki]), num(f[ikd])),
        );
    }
    (table.join("\n"), m)
}

fn prop(db: &DB, name: &str) -> u64 {
    db.property_int_value(name).ok().flatten().unwrap_or(0)
}

/// Block until no flush or compaction is running or pending.
fn wait_idle(db: &DB, what: &str) {
    let t = Instant::now();
    loop {
        if prop(db, "rocksdb.compaction-pending") == 0
            && prop(db, "rocksdb.num-running-compactions") == 0
            && prop(db, "rocksdb.num-running-flushes") == 0
            && prop(db, "rocksdb.mem-table-flush-pending") == 0
        {
            break;
        }
        if t.elapsed() > Duration::from_secs(3 * 3600) {
            eprintln!("[lsm-validate] {what}: still compacting after three hours, going on");
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if t.elapsed() > Duration::from_secs(1) {
        eprintln!("[lsm-validate] {what}: settled in {:.1?}", t.elapsed());
    }
}

#[derive(Clone, Copy, Default)]
struct Acc {
    puts: u64,
    deletes: u64,
    /// Index mode: puts to live entries chosen at random.
    relocs: u64,
    /// Key and value bytes handed to the engine.
    user_bytes: u64,
    /// Serialized record bytes of the puts, as the replay reports them.
    record_bytes: u64,
    /// Keys the replayed blocks touched for the first time, puts and
    /// deletes: entries new to every level, which sink to the last one.
    first_touches: u64,
}

impl Acc {
    fn updates(&self) -> u64 {
        self.puts + self.deletes + self.relocs
    }
}

const TICKERS: [(&str, &str); 7] = [
    ("WAL", "rocksdb.wal.bytes"),
    ("flush", "rocksdb.flush.write.bytes"),
    ("compaction write", "rocksdb.compact.write.bytes"),
    ("compaction read", "rocksdb.compact.read.bytes"),
    ("user bytes (engine's count)", "rocksdb.bytes.written"),
    ("keys written", "rocksdb.number.keys.written"),
    ("stall micros", "rocksdb.stall.micros"),
];

fn delta(a: &HashMap<String, u64>, b: &HashMap<String, u64>, name: &str) -> u64 {
    b.get(name).copied().unwrap_or(0) - a.get(name).copied().unwrap_or(0)
}

pub fn run(cfg: Config) {
    let t_all = Instant::now();
    let dbp = std::path::Path::new(&cfg.db);
    if dbp.exists() {
        assert!(
            dbp.join("CURRENT").exists(),
            "{} exists and is not a RocksDB directory",
            cfg.db
        );
        DB::destroy(&Options::default(), &cfg.db).expect("destroy the old database");
    }
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_compaction_style(DBCompactionStyle::Level);
    opts.set_level_compaction_dynamic_level_bytes(true);
    opts.set_write_buffer_size(cfg.buffer_bytes as usize);
    opts.set_max_write_buffer_number(3);
    opts.set_min_write_buffer_number_to_merge(1);
    opts.set_level_zero_file_num_compaction_trigger(cfg.l0_trigger);
    // Stalls change timing, not bytes: never block the writer.
    opts.set_level_zero_slowdown_writes_trigger(1 << 20);
    opts.set_level_zero_stop_writes_trigger(1 << 20);
    opts.set_soft_pending_compaction_bytes_limit(0);
    opts.set_hard_pending_compaction_bytes_limit(0);
    opts.set_max_bytes_for_level_base(cfg.level_base_bytes);
    opts.set_max_bytes_for_level_multiplier(cfg.multiplier);
    opts.set_target_file_size_base(cfg.target_file_bytes);
    opts.set_target_file_size_multiplier(1);
    opts.set_compression_type(cfg.compression);
    opts.set_max_background_jobs(cfg.threads);
    opts.increase_parallelism(cfg.threads);
    opts.set_max_subcompactions(4);
    // Off while the starting state loads; one manual compaction settles it.
    opts.set_disable_auto_compactions(true);
    opts.enable_statistics();
    let mut bbo = BlockBasedOptions::default();
    bbo.set_block_size(4096);
    bbo.set_block_cache(&Cache::new_lru_cache(cfg.block_cache_bytes as usize));
    bbo.set_bloom_filter(cfg.bloom_bits, false);
    bbo.set_cache_index_and_filter_blocks(false);
    opts.set_block_based_table_factory(&bbo);
    let db = DB::open(&opts, &cfg.db).expect("open the database");
    // The window opens after this many blocks; without a block limit the
    // stream's length is unknown, so it has to be given.
    let measure_from = cfg
        .measure_from
        .or(cfg.max_blocks.map(|m| m / 2))
        .expect("--measure-from-block N (or --blocks N, which measures its second half)");
    let mut wo = WriteOptions::default();
    wo.set_sync(cfg.sync);

    let layout = if cfg.index_mode {
        KeyLayout::Hash
    } else {
        cfg.key
    };
    // Index mode: the stream's live keys, to pick relocations from.
    let mut pool: Vec<(u64, u8)> = Vec::new();
    let mut pool_at: HashMap<u64, u32> = HashMap::new();
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut block_updates: u64 = 0;
    let mut reloc_debt: f64 = 0.0;
    let mut rd = StreamReader::open(&cfg.stream).expect("open the stream");
    let mut key = Vec::with_capacity(64);
    let mut val = Vec::with_capacity(512);
    let mut live: HashSet<u64> = HashSet::new();
    // Ids touched by the replayed blocks.
    let mut touched: HashSet<u64> = HashSet::new();
    // A commit lists its records before its deletes, and a delete may name
    // the position a record of the same commit now occupies (a leaf split
    // into a branch, or a branch merged into a leaf). That delete refers to
    // the record the put replaced and must not land after the put.
    let mut put_in_sink: HashSet<u64> = HashSet::new();
    let mut last_was_delete = false;
    let mut deletes_dropped: u64 = 0;
    // The engine's memtable, sampled: bytes per entry it holds.
    let (mut mem_bytes, mut mem_entries) = (0u64, 0u64);
    let mut counter = DistinctCounter::new(4);
    // Position lengths seen, in bytes: the longest, and a histogram by 8.
    let mut max_pos: usize = 0;
    let mut pos_hist = [0u64; 16];
    let mut batch = WriteBatch::default();
    let mut started = false;
    let mut cur_block: Option<u32> = None;
    let mut blocks_done: u64 = 0;
    let mut first_block: u32 = 0;
    let mut last_first_touches: u64 = 0;
    let mut total = Acc::default();
    let mut genesis = Acc::default();
    let mut genesis_sst: u64 = 0;
    let mut genesis_wall = Duration::ZERO;
    let mut genesis_live: u64 = 0;
    let mut ballast_puts: u64 = 0;
    let mut base: Option<(HashMap<String, u64>, Acc, u64)> = None;
    let mut base_levels: Option<LevelStats> = None;
    let mut start_tickers: Option<HashMap<String, u64>> = None;
    let mut seq: u64 = 0;
    let t_replay = Instant::now();
    let mut stopped = false;
    let mut passes_left = cfg.repeat.max(1) - 1;
    let mut window_opened_micros: u64 = 0;
    loop {
        let ev = match rd.next().expect("read the stream") {
            Some(ev) => ev,
            None => {
                if passes_left == 0 {
                    break;
                }
                passes_left -= 1;
                rd = StreamReader::open(&cfg.stream).expect("reopen the stream");
                // Skip the starting state: this pass replays the blocks again.
                loop {
                    match rd.next().expect("read the stream") {
                        Some(Event::Start) => break,
                        Some(_) => {}
                        None => panic!("the stream has no start marker"),
                    }
                }
                eprintln!(
                    "[lsm-validate] the stream's blocks replayed again (pass {} of {}) after {blocks_done} blocks",
                    cfg.repeat - passes_left,
                    cfg.repeat
                );
                continue;
            }
        };
        match ev {
            Event::Genesis => {}
            Event::Start => {
                if !batch.is_empty() {
                    db.write_opt(std::mem::take(&mut batch), &wo).unwrap();
                }
                let t = Instant::now();
                db.flush().unwrap();
                db.compact_range::<&[u8], &[u8]>(None, None);
                wait_idle(&db, "starting state");
                db.set_options(&[("disable_auto_compactions", "false")])
                    .unwrap();
                genesis_sst = prop(&db, "rocksdb.live-sst-files-size");
                genesis_wall = t.elapsed();
                genesis_live = live.len() as u64;
                put_in_sink.clear();
                last_was_delete = false;
                if cfg.count_keys {
                    let t = Instant::now();
                    let n = db.iterator(rocksdb::IteratorMode::Start).count();
                    eprintln!("[lsm-validate] the database holds {n} keys after the starting state ({:.1?}); tracked live {}", t.elapsed(), live.len());
                }
                eprintln!(
                    "[lsm-validate] position lengths: longest {max_pos} B; by 8 B: {:?}",
                    pos_hist
                );
                eprintln!(
                    "[lsm-validate] starting state: {} puts, {} live entries ({} estimated by the engine), {:.1} MB handed in, {:.1} MB on disk after compaction ({:.1?})",
                    genesis.puts,
                    genesis_live,
                    prop(&db, "rocksdb.estimate-num-keys"),
                    genesis.user_bytes as f64 / 1e6,
                    genesis_sst as f64 / 1e6,
                    genesis_wall
                );
                start_tickers = Some(tickers(&opts));
                started = true;
            }
            Event::Block(i) => {
                put_in_sink.clear();
                last_was_delete = false;
                if cur_block.is_some() {
                    if cfg.relocations > 0.0 && !pool.is_empty() {
                        let n = block_updates as f64 * cfg.relocations + reloc_debt;
                        let whole = n.floor();
                        reloc_debt = n - whole;
                        for _ in 0..whole as u64 {
                            let (h1, len) = pool[(xorshift(&mut rng) % pool.len() as u64) as usize];
                            hash_key(h1, len as usize, &mut key);
                            let c = xorshift(&mut rng) % (cfg.ballast as u64 + 1);
                            if c > 0 {
                                key.push(c as u8);
                            }
                            fill_value(&mut val, 8, xorshift(&mut rng));
                            batch.put(&key, &val);
                            total.relocs += 1;
                            total.user_bytes += (key.len() + val.len()) as u64;
                        }
                    }
                    block_updates = 0;
                    if !batch.is_empty() {
                        db.write_opt(std::mem::take(&mut batch), &wo).unwrap();
                    }
                    blocks_done += 1;
                    if blocks_done % 100 == 0 {
                        let e = prop(&db, "rocksdb.num-entries-active-mem-table");
                        if e > 10_000 {
                            mem_entries += e;
                            mem_bytes += prop(&db, "rocksdb.cur-size-active-mem-table");
                        }
                    }
                    if blocks_done % 1000 == 0 {
                        let tk = tickers(&opts);
                        let since = start_tickers.as_ref().unwrap();
                        eprintln!(
                            "[lsm-validate] block {blocks_done} ({:.1?}): {} updates, flush {:.2} GB, compaction {:.2} GB written, pending {:.2} GB, live {} entries, first touches {} ({} in the last 1000 blocks)",
                            t_replay.elapsed(),
                            total.updates(),
                            delta(since, &tk, "rocksdb.flush.write.bytes") as f64 / 1e9,
                            delta(since, &tk, "rocksdb.compact.write.bytes") as f64 / 1e9,
                            prop(&db, "rocksdb.estimate-pending-compaction-bytes") as f64 / 1e9,
                            live.len(),
                            total.first_touches,
                            total.first_touches - last_first_touches
                        );
                        last_first_touches = total.first_touches;
                    }
                    if cfg.max_blocks.is_some_and(|m| blocks_done >= m) {
                        stopped = true;
                        break;
                    }
                } else {
                    first_block = i;
                }
                if blocks_done == measure_from {
                    base = Some((
                        tickers(&opts),
                        total,
                        prop(&db, "rocksdb.live-sst-files-size"),
                    ));
                    base_levels = Some(level_table(&db).1);
                    window_opened_micros = unix_micros();
                    eprintln!(
                        "[lsm-validate] measured window opens at block {blocks_done} of the stream, unix micros {window_opened_micros}"
                    );
                }
                cur_block = Some(i);
            }
            Event::Put { bytes, .. } => {
                let k = rd.key();
                max_pos = max_pos.max(k.len());
                pos_hist[(k.len() / 8).min(15)] += 1;
                encode_key(k, layout, &mut key);
                let vlen = if cfg.index_mode {
                    8
                } else {
                    (bytes as usize).saturating_sub(k.len()).max(1)
                };
                let bytes = if cfg.index_mode {
                    (key.len() + vlen) as u32
                } else {
                    bytes
                };
                seq += 1;
                fill_value(
                    &mut val,
                    vlen,
                    id64(k) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15),
                );
                batch.put(&key, &val);
                let h = id64(k);
                if last_was_delete {
                    put_in_sink.clear();
                    last_was_delete = false;
                }
                put_in_sink.insert(h);
                live.insert(h);
                if cfg.relocations > 0.0 && !pool_at.contains_key(&h) {
                    pool_at.insert(h, pool.len() as u32);
                    pool.push((h, k.len() as u8));
                }
                if started {
                    if touched.insert(h) {
                        total.first_touches += 1;
                    }
                    block_updates += 1;
                }
                let acc = if started { &mut total } else { &mut genesis };
                acc.puts += 1;
                acc.user_bytes += (key.len() + vlen) as u64;
                acc.record_bytes += bytes as u64;
                if started {
                    counter.push(h);
                } else {
                    // Ballast: the same record under its key with one more
                    // byte, so the copies sort beside the original and a
                    // compaction over the stream's keys rewrites them, as
                    // it would the untouched state on mainnet.
                    for c in 1..=cfg.ballast {
                        key.push(c as u8);
                        fill_value(&mut val, vlen, h ^ (c as u64) << 56);
                        batch.put(&key, &val);
                        live.insert(id64(&key));
                        ballast_puts += 1;
                        key.pop();
                    }
                    if genesis.puts % 2_000_000 == 0 {
                        db.write_opt(std::mem::take(&mut batch), &wo).unwrap();
                    }
                }
            }
            Event::Delete => {
                let k = rd.key();
                let h = id64(k);
                last_was_delete = true;
                if put_in_sink.contains(&h) {
                    deletes_dropped += 1;
                    continue;
                }
                encode_key(k, layout, &mut key);
                batch.delete(&key);
                live.remove(&h);
                if let Some(i) = pool_at.remove(&h) {
                    let last = pool.pop().expect("pool holds the deleted key");
                    if (i as usize) < pool.len() {
                        pool[i as usize] = last;
                        pool_at.insert(last.0, i);
                    }
                }
                let acc = if started { &mut total } else { &mut genesis };
                acc.deletes += 1;
                acc.user_bytes += key.len() as u64;
                if started {
                    if touched.insert(h) {
                        total.first_touches += 1;
                    }
                    counter.push(h);
                    block_updates += 1;
                }
            }
        }
    }
    if !stopped && cur_block.is_some() {
        if !batch.is_empty() {
            db.write_opt(std::mem::take(&mut batch), &wo).unwrap();
        }
        blocks_done += 1;
    }
    assert!(started, "the stream has no replayed blocks");
    let (base_tk, base_acc, base_sst) = base.unwrap_or_else(|| {
        panic!("the measured window never opened: --measure-from-block {measure_from} is past the {blocks_done} blocks applied")
    });
    // Let the engine finish what the window started before the end
    // figures are read, as the model charges every update its full cost.
    let t = Instant::now();
    db.flush().unwrap();
    wait_idle(&db, "end of the window");
    let settle = t.elapsed();
    let window_closed_micros = unix_micros();
    eprintln!("[lsm-validate] window closed at unix micros {window_closed_micros}");
    let end_tk = tickers(&opts);
    let start_tk = start_tickers.unwrap();
    let end_sst = prop(&db, "rocksdb.live-sst-files-size");
    let est_keys = prop(&db, "rocksdb.estimate-num-keys");
    let levelstats = db
        .property_value("rocksdb.levelstats")
        .ok()
        .flatten()
        .unwrap_or_default();
    // The engine's own compaction table, cumulative since the database was
    // opened: per level, what compactions read and wrote and how often.
    let (compaction_table, end_levels) = level_table(&db);
    let base_levels = base_levels.unwrap_or_default();
    let flushes =
        end_levels.get("L0").map_or(0.0, |e| e.2) - base_levels.get("L0").map_or(0.0, |b| b.2);
    // The engine's levels on disk, smallest first, as entries: what the
    // model is given for the like-for-like run.
    let mut engine_caps: Vec<(String, f64)> = levelstats
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() == 3 {
                if let (Ok(level), Ok(mb)) = (f[0].parse::<u32>(), f[2].parse::<f64>()) {
                    if level > 0 && mb > 0.0 {
                        return Some((format!("L{level}"), mb * 1e6));
                    }
                }
            }
            None
        })
        .collect();
    engine_caps.sort_by(|a, b| a.0.cmp(&b.0));

    // The window's updates.
    let win = Acc {
        puts: total.puts - base_acc.puts,
        deletes: total.deletes - base_acc.deletes,
        relocs: total.relocs - base_acc.relocs,
        user_bytes: total.user_bytes - base_acc.user_bytes,
        record_bytes: total.record_bytes - base_acc.record_bytes,
        first_touches: total.first_touches - base_acc.first_touches,
    };
    let win_blocks = blocks_done - measure_from;
    let entry_bytes_model = win.record_bytes as f64 / win.puts.max(1) as f64 + 8.0;
    let (curve, len) = (counter.curve(), counter.len());
    // Relocations per stream update, the model's share of never-absorbed
    // updates; the curve counts the stream's updates alone.
    let reloc_share = win.relocs as f64 / (win.puts + win.deletes).max(1) as f64;
    let model_at = |buffer_bytes: u64| {
        cost_from_curve(
            &curve,
            len,
            reloc_share,
            LsmConfig {
                entries: live.len() as u64,
                entry_bytes: entry_bytes_model,
                buffer_bytes,
                ratio: cfg.multiplier,
            },
        )
    };
    let model = model_at(cfg.buffer_bytes);
    // The engine's memtable spends more on an update than the record, and
    // holds every update, not every distinct entry: the same buffer holds
    // fewer, and flushes what its updates touched. The model with the
    // engine's own levels and that memtable is the like-for-like test of
    // its mechanism.
    let mem_per_entry = if mem_entries > 0 {
        mem_bytes as f64 / mem_entries as f64
    } else {
        entry_bytes_model
    };
    let eff_entries = cfg.buffer_bytes as f64 / mem_per_entry;

    let mut md = String::new();
    if cfg.index_mode {
        let _ = writeln!(md, "# LSM of index entries on RocksDB: {}\n", cfg.label);
        let _ = writeln!(md, "> MEASURED on RocksDB {} fed the record stream `{}` as the disk index of a log store sees it: every record update is a put of its 16-byte hashed key with an 8-byte value (the log position), a delete removes the key, and every block adds {:.2} puts per update to live entries chosen at random, ballast copies included (the records the log's garbage collection moves). The model is `pbt_store::lsm::cost_from_curve` on the same stream with that relocation share, with this database's live entries as the full state.\n", rocksdb_version(), cfg.stream, cfg.relocations);
    } else {
        let _ = writeln!(md, "# LSM of records on RocksDB: {}\n", cfg.label);
        let _ = writeln!(md, "> MEASURED on RocksDB {} fed the record stream `{}`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as {:?}.\n", rocksdb_version(), cfg.stream, layout);
    }
    let _ = writeln!(md, "| setting | value |\n|---|---|");
    let _ = writeln!(md, "| write buffer | {} MB |", cfg.buffer_bytes >> 20);
    let _ = writeln!(
        md,
        "| L0 files before compaction into L1 | {} |",
        cfg.l0_trigger
    );
    let _ = writeln!(
        md,
        "| level base (`max_bytes_for_level_base`, dynamic level bytes) | {} MB |",
        cfg.level_base_bytes >> 20
    );
    let _ = writeln!(md, "| level multiplier | {} |", cfg.multiplier);
    let _ = writeln!(
        md,
        "| target file size | {} MB |",
        cfg.target_file_bytes >> 20
    );
    let _ = writeln!(md, "| compression | {} |", cfg.compression_name);
    let _ = writeln!(md, "| bloom filter | {} bits per key |", cfg.bloom_bits);
    let _ = writeln!(md, "| block cache | {} MB |", cfg.block_cache_bytes >> 20);
    let _ = writeln!(md, "| WAL sync per block | {} |", cfg.sync);
    let _ = writeln!(md, "| background jobs | {} |", cfg.threads);
    let _ = writeln!(md, "| key layout | {:?} |", layout);
    if cfg.index_mode {
        let _ = writeln!(
            md,
            "| index mode | 16 B hashed keys, 8 B values, {:.2} relocations per update |",
            cfg.relocations
        );
    }
    let _ = writeln!(md);

    let _ = writeln!(md, "## Starting state\n");
    let _ = writeln!(
        md,
        "- {} puts and {} deletes handed in ({:.1} MB of keys and values, {:.1} B per put), plus {} ballast puts ({} copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); {} live entries once loaded, {:.1} MB on disk once compacted into the last level ({:.1} B per live entry), loaded and compacted in {:.0?}",
        genesis.puts,
        genesis.deletes,
        genesis.user_bytes as f64 / 1e6,
        genesis.user_bytes as f64 / genesis.puts.max(1) as f64,
        ballast_puts,
        cfg.ballast,
        genesis_live,
        genesis_sst as f64 / 1e6,
        genesis_sst as f64 / genesis_live.max(1) as f64,
        genesis_wall
    );

    let _ = writeln!(md, "\n## Replay\n");
    let _ = writeln!(
        md,
        "- {} blocks applied (the stream's block {} onward), {} updates in all; measured window: blocks {}..{} of the stream, {} blocks, {} updates ({} puts, {} deletes, {} relocations), {:.1} MB of keys and values, {:.1} B per put as a record, {:.1} B per entry with the engine's 8 B of overhead",
        blocks_done, first_block, total.updates(), measure_from, blocks_done, win_blocks,
        win.updates(), win.puts, win.deletes, win.relocs, win.user_bytes as f64 / 1e6,
        win.record_bytes as f64 / win.puts.max(1) as f64, entry_bytes_model
    );
    if cfg.repeat > 1 {
        let _ = writeln!(
            md,
            "- the stream's blocks were replayed {} times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)",
            cfg.repeat
        );
    }
    let _ = writeln!(
        md,
        "- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros {window_opened_micros} and {window_closed_micros} (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)"
    );
    let disk_per_entry = end_sst as f64 / live.len().max(1) as f64;
    let caps_entries: Vec<f64> = engine_caps
        .iter()
        .map(|(_, b)| b / disk_per_entry)
        .collect();
    let model_eff = cost_with_levels(
        &curve,
        len,
        reloc_share,
        eff_entries,
        &caps_entries,
        Buffer::Memtable {
            updates: eff_entries,
            l0_files: cfg.l0_trigger as f64,
        },
    );
    let _ = writeln!(
        md,
        "- at the end: {} live entries tracked ({} estimated by the engine), {:.1} MB on disk ({:.1} B per live entry, against {:.1} B handed in per put: prefix compression and {} at work); {} deletes dropped as superseded within their own commit; the replayed blocks touched {} distinct ids, {} of them first inside the window ({:.3} per update: entries new to every level); the window's last flush and compactions settled in {:.1?}; replay {:.0?}, whole run {:.0?}",
        live.len(), est_keys, end_sst as f64 / 1e6, disk_per_entry,
        win.user_bytes as f64 / win.updates().max(1) as f64, cfg.compression_name, deletes_dropped, touched.len(), win.first_touches, win.first_touches as f64 / win.updates().max(1) as f64, settle,
        t_replay.elapsed(), t_all.elapsed()
    );
    let _ = writeln!(
        md,
        "- the engine's memtable held {:.1} B per entry (sampled), so the {} MB buffer holds about {:.0} entries, against the {:.0} entries the model gives a buffer of that size at {:.1} B per entry",
        mem_per_entry, cfg.buffer_bytes >> 20, eff_entries, cfg.buffer_bytes as f64 / entry_bytes_model, entry_bytes_model
    );

    let _ = writeln!(md, "\n### Engine, measured window\n");
    let _ = writeln!(
        md,
        "| figure | bytes | per update | per user byte | per block |\n|---|---|---|---|---|"
    );
    let ub = win.user_bytes.max(1) as f64;
    let up = win.updates().max(1) as f64;
    let bl = win_blocks.max(1) as f64;
    for (label, name) in TICKERS {
        let d = delta(&base_tk, &end_tk, name);
        if name.ends_with("bytes") {
            let _ = writeln!(
                md,
                "| {label} | {d} | {:.1} | {:.2} | {:.0} |",
                d as f64 / up,
                d as f64 / ub,
                d as f64 / bl
            );
        } else {
            let _ = writeln!(
                md,
                "| {label} | {d} | {:.2} | - | {:.0} |",
                d as f64 / up,
                d as f64 / bl
            );
        }
    }
    let written = delta(&base_tk, &end_tk, "rocksdb.flush.write.bytes")
        + delta(&base_tk, &end_tk, "rocksdb.compact.write.bytes");
    let read = delta(&base_tk, &end_tk, "rocksdb.compact.read.bytes");
    let amp_user = written as f64 / ub;
    let amp_entries = written as f64 / (up * disk_per_entry);
    let _ = writeln!(
        md,
        "| **flush + compaction write** | {written} | {:.1} | **{:.2}** | {:.0} |",
        written as f64 / up,
        amp_user,
        written as f64 / bl
    );
    let _ = writeln!(md, "\n- write amplification, engine: **{amp_user:.2}** bytes written per user byte, **{amp_entries:.2}** entries of {disk_per_entry:.1} B (the on-disk entry) per update; compaction reads {:.2} per user byte, {:.2} entries per update. Disk grew by {:.1} MB over the window.", read as f64 / ub, read as f64 / (up * disk_per_entry), (end_sst as f64 - base_sst as f64) / 1e6);
    let whole_written = delta(&start_tk, &end_tk, "rocksdb.flush.write.bytes")
        + delta(&start_tk, &end_tk, "rocksdb.compact.write.bytes");
    let _ = writeln!(
        md,
        "- whole replay, levels filling included: {:.2} per user byte ({} updates, {:.1} MB).",
        whole_written as f64 / total.user_bytes.max(1) as f64,
        total.updates(),
        total.user_bytes as f64 / 1e6
    );

    let _ = writeln!(md, "\n### Model, same stream and constants\n");
    let _ = writeln!(
        md,
        "- write amplification, model at the nominal buffer: **{:.2}** entries written per update (flushes and compactions, the WAL apart), compaction reads {:.2}; {} entries, {:.1} B each, buffer {} MB = {:.0} entries, ratio {}; the stream's {} updates, longest measured window {}",
        model.amplification, model.read_amplification, live.len(), entry_bytes_model, cfg.buffer_bytes >> 20, cfg.buffer_bytes as f64 / entry_bytes_model, cfg.multiplier, model.stream_updates, model.longest_window
    );
    let _ = writeln!(
        md,
        "- write amplification, model with the engine's levels ({}) and an append-only memtable of {:.0} updates merged {} flushes at a time: **{:.2}** entries written per update, compaction reads {:.2}",
        engine_caps.iter().map(|(n, b)| format!("{n} {:.0} MB", b / 1e6)).collect::<Vec<_>>().join(", "),
        eff_entries, cfg.l0_trigger, model_eff.amplification, model_eff.read_amplification
    );
    for (title, m) in [
        ("nominal buffer, the report-time model", &model),
        ("engine's levels and memtable", &model_eff),
    ] {
        let _ = writeln!(md, "\nModel levels, {title}:\n\n| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |\n|---|---|---|---|---|---|---|");
        for (k, l) in m.levels.iter().enumerate() {
            let name = if k == 0 {
                "buffer".to_string()
            } else {
                format!("L{k}")
            };
            let _ = writeln!(
                md,
                "| {name} | {:.0} | {} | {} | {:.3} | {:.3} | {:.3} |",
                l.capacity,
                l.window.map_or("-".to_string(), |w| format!("{w:.0}")),
                if l.window.is_none() {
                    "-".to_string()
                } else {
                    l.measured.to_string()
                },
                l.outflow,
                l.written,
                l.read
            );
        }
    }
    // The engine per level over the window, beside the model's level that
    // receives the same merges: the engine books a merge at the level it
    // writes into, the model at the level the entries leave.
    let _ = writeln!(md, "\nEngine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):\n\n| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |\n|---|---|---|---|---|---|---|---|---|---|");
    let mut names: Vec<&String> = end_levels.keys().filter(|k| k.starts_with('L')).collect();
    names.sort();
    for (i, name) in names.iter().enumerate() {
        let e = end_levels[*name];
        let b = base_levels
            .get(*name)
            .copied()
            .unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0));
        let (r, w, ki, kd) = (e.0 - b.0, e.1 - b.1, e.3 - b.3, e.4 - b.4);
        if w <= 0.0 && r <= 0.0 {
            continue;
        }
        // The stats table prints GiB.
        let ew = w * 1024.0 * 1024.0 * 1024.0 / (up * disk_per_entry);
        // L0 receives flushes (the model's buffer flush); the next engine
        // level receives the buffer's merge, and so on.
        let (mname, mw) = if i == 0 {
            (
                "buffer (flush)".to_string(),
                model_eff
                    .levels
                    .first()
                    .map_or(0.0, |l| distinct_share(&curve, eff_entries, l)),
            )
        } else {
            let k = i - 1;
            (
                if k == 0 {
                    "buffer (merge)".to_string()
                } else {
                    format!("L{k}")
                },
                model_eff.levels.get(k).map_or(0.0, |l| {
                    if k == 0 {
                        l.written - distinct_share(&curve, eff_entries, l)
                    } else {
                        l.written
                    }
                }),
            )
        };
        let out = if ki > 0.0 {
            format!("{:.2}", (ki - kd) / up)
        } else {
            "-".to_string()
        };
        let _ = writeln!(md, "| {name} | {w:.2} | {r:.2} | {ew:.2} | {ki:.0} | {kd:.0} | {out} | {mname} | {mw:.2} | {:.2} |", if mw > 0.0 { ew / mw } else { 0.0 });
    }
    let model_bytes_blk = up / bl * entry_bytes_model * model.amplification;
    let model_eff_disk_blk = up / bl * disk_per_entry * model_eff.amplification;
    let _ = writeln!(
        md,
        "\n### Side by side\n\nThe report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.\n\n| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |\n|---|---|---|---|---|---|\n| entries written per update | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |\n| bytes written per block by flushes and compactions | {:.0} | {:.0} | {:.0} | {:.2} | {:.2} |\n| entries read back per update by compactions | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
        amp_entries, model.amplification, model_eff.amplification,
        amp_entries / model.amplification.max(1e-9), amp_entries / model_eff.amplification.max(1e-9),
        written as f64 / bl, model_bytes_blk, model_eff_disk_blk,
        written as f64 / bl / model_bytes_blk.max(1.0), written as f64 / bl / model_eff_disk_blk.max(1.0),
        read as f64 / (up * disk_per_entry), model.read_amplification, model_eff.read_amplification,
        read as f64 / (up * disk_per_entry) / model.read_amplification.max(1e-9), read as f64 / (up * disk_per_entry) / model_eff.read_amplification.max(1e-9)
    );
    let _ = writeln!(
        md,
        "\n### Engine levels at the end\n\n```\n{}```",
        levelstats
    );
    let _ = writeln!(md, "\n{:.0} flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):\n\n```\n{}\n```", flushes, compaction_table);
    let out = format!("{}/{}.md", cfg.out_dir, cfg.label);
    std::fs::create_dir_all(&cfg.out_dir).unwrap();
    std::fs::write(&out, &md).unwrap();
    print!("{md}");
    eprintln!("[lsm-validate] wrote {out}");
}

/// The bundled library, as the `rocksdb` crate pins it.
fn rocksdb_version() -> &'static str {
    "11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19)"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(bits: &[u8]) -> Vec<u8> {
        pbt_ref::bits::encode_bit_prefix(bits)
    }

    #[test]
    fn path_keys_sort_in_pre_order() {
        let enc = |bits: &[u8]| {
            let mut k = Vec::new();
            encode_key(&pos(bits), KeyLayout::Path, &mut k);
            k
        };
        let a = enc(&[0, 1, 0, 1]);
        let b = enc(&[0, 1, 0, 1, 0]);
        let c = enc(&[0, 1, 0, 1, 1]);
        assert!(a < b && b < c);
        // The right subtree of a node sorts after the whole left subtree,
        // however deep the left one goes.
        let deep_left = enc(&[
            0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        ]);
        assert!(deep_left < c);
        assert!(a < deep_left);
        // A key ends where its bits do: nine bytes per group of 64 bits.
        assert_eq!(a.len(), 9);
        assert_eq!(a[8], 4);
        let long = enc(&[1; 70]);
        assert_eq!(long.len(), 18);
        assert_eq!(long[8], 0xFF);
        assert_eq!(long[17], 6);
        let exact = enc(&[1; 64]);
        assert_eq!(exact.len(), 9);
        assert_eq!(exact[8], 64);
        assert!(exact < long);
        // Different positions never share a key, even past 64 bits.
        let mut x = vec![1u8; 70];
        x[66] = 0;
        assert_ne!(enc(&x), long);
        let root = enc(&[]);
        assert_eq!(root, vec![0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(root < a);
    }

    #[test]
    fn page_keys_group_six_levels() {
        let enc = |bits: &[u8]| {
            let mut k = Vec::new();
            encode_key(&pos(bits), KeyLayout::Page, &mut k);
            k
        };
        let k3 = enc(&[1, 0, 1]);
        assert_eq!(
            k3[..9],
            [0, 0, 0, 0, 0, 0, 0, 0, 0],
            "no page prefix in the first page"
        );
        assert_eq!(k3[9], 0, "depth 3 is in the first page");
        assert_eq!(k3[10], 0b1101);
        let k6 = enc(&[1, 0, 1, 1, 1, 1]);
        assert_eq!(k6[9], 0, "depth 6 still");
        let k7 = enc(&[1, 0, 1, 1, 1, 1, 0]);
        assert_eq!(k7[9], 1, "depth 7 opens the next page");
        assert_eq!(k7[0], 0b1011_1100, "six prefix bits kept");
        assert_eq!(k7[8], 6, "six valid bits in the group");
        assert_eq!(k7[10], 0b10);
        assert!(k3 < k6 && k6 < k7);
    }
}
