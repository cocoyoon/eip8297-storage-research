//! Physical storage backends under a fixed PBT commitment (Phase 2).
//!
//! One interface, several physical organizations. The tree (pbt-ref) stays
//! the in-memory working representation; at each block commit it emits the
//! set of records whose committed bytes changed (`CommitSink`), and every
//! backend applies the same batch its own way, counting IO.
//!
//! Realism levels are deliberately mixed and documented:
//! - Write path of `append` and `paged`: REAL file writes + fsync per block
//!   (wall time meaningful) plus logical counters.
//! - Read path and append-compaction: logical counters only (deterministic,
//!   no page cache noise).
//! - `kv` (libmdbx): fully real, reads included -- the production control.

pub mod append;
pub mod btree;
pub mod kv;
pub mod lsm;
pub mod mrc;
pub mod paged;
pub mod shape;
pub mod split;

use pbt_ref::tree::{CommitSink, ReadPath};

pub const PAGE: u64 = 4096;

#[derive(Default, Clone)]
pub struct Metrics {
    /// Sequential bytes appended (log writes, compaction rewrites).
    pub seq_bytes_written: u64,
    /// Sequential bytes read (compaction input).
    pub seq_bytes_read: u64,
    /// 4KB random page writes (paged stores, spills).
    pub rand_page_writes: u64,
    /// 4KB random page reads (read-modify-write inputs, lookups).
    pub page_reads: u64,
    /// Bytes actually transferred to serve reads. A per-IO price ignores
    /// this; a bandwidth price does not, and the two disagree because a
    /// page store moves 4 KB per hop while a log store moves one sector.
    pub read_bytes: u64,
    pub fsyncs: u64,
    pub live_bytes: u64,
    pub garbage_bytes: u64,
    pub compactions: u64,
    pub record_writes: u64,
    pub record_deletes: u64,
    pub reads_served: u64,
    pub wall_ns: u64,
    /// Backend-specific: on-disk footprint if measurable.
    pub disk_bytes: u64,
    /// DRAM the configuration requires to hold its resident tier.
    pub dram_bytes: u64,
}

/// Relative IO cost model. Units: cost of one sequential 4KB write = 1.
/// These are ASSUMPTION profiles, reported side by side so the d* argmin's
/// robustness to the model is visible.
#[derive(Clone, Copy)]
pub struct DeviceProfile {
    pub name: &'static str,
    pub c_seq4k_write: f64,
    pub c_seq4k_read: f64,
    pub c_rand_write: f64,
    pub c_rand_read: f64,
    /// A durability barrier is not free: designs here differ 4:1 in
    /// fsyncs per block, so the model prices them.
    pub c_fsync: f64,
    /// Opportunity cost of holding one MB resident, per block. Without a
    /// DRAM term a design can buy unlimited speed with unlimited memory.
    pub c_dram_mb: f64,
    /// Bandwidth-aware read price: a fixed per-IO overhead plus a per-MB
    /// transfer term, calibrated so a 4 KB random read costs the same as
    /// `c_rand_read` under the per-IO model.
    pub c_read_io_fixed: f64,
    pub c_read_mb: f64,
}

pub const PROFILES: [DeviceProfile; 2] = [
    DeviceProfile {
        name: "nvme-optimistic",
        c_seq4k_write: 1.0,
        c_seq4k_read: 0.8,
        c_rand_write: 4.0,
        c_rand_read: 3.0,
        c_fsync: 20.0,
        c_dram_mb: 0.5,
        c_read_io_fixed: 1.5,
        c_read_mb: 384.0,
    },
    DeviceProfile {
        name: "commodity-ssd",
        c_seq4k_write: 1.0,
        c_seq4k_read: 0.8,
        c_rand_write: 16.0,
        c_rand_read: 10.0,
        c_fsync: 60.0,
        c_dram_mb: 2.0,
        c_read_io_fixed: 5.0,
        c_read_mb: 1280.0,
    },
];

impl Metrics {
    pub fn cost(&self, p: &DeviceProfile) -> f64 {
        let seq_w = self.seq_bytes_written as f64 / PAGE as f64;
        let seq_r = self.seq_bytes_read as f64 / PAGE as f64;
        seq_w * p.c_seq4k_write
            + seq_r * p.c_seq4k_read
            + self.rand_page_writes as f64 * p.c_rand_write
            + self.page_reads as f64 * p.c_rand_read
            + self.fsyncs as f64 * p.c_fsync
    }

    /// Per-block cost including the DRAM the configuration holds resident.
    pub fn cost_per_block(&self, p: &DeviceProfile, blocks: u64) -> f64 {
        let b = blocks.max(1) as f64;
        self.cost(p) / b + (self.dram_bytes as f64 / (1 << 20) as f64) * p.c_dram_mb
    }

    /// Same, but reads priced by bytes moved rather than per IO.
    pub fn cost_per_block_bw(&self, p: &DeviceProfile, blocks: u64) -> f64 {
        let b = blocks.max(1) as f64;
        let seq_w = self.seq_bytes_written as f64 / PAGE as f64;
        let seq_r = self.seq_bytes_read as f64 / PAGE as f64;
        let reads = self.page_reads as f64 * p.c_read_io_fixed
            + (self.read_bytes as f64 / (1 << 20) as f64) * p.c_read_mb;
        let io = seq_w * p.c_seq4k_write
            + seq_r * p.c_seq4k_read
            + self.rand_page_writes as f64 * p.c_rand_write
            + reads
            + self.fsyncs as f64 * p.c_fsync;
        io / b + (self.dram_bytes as f64 / (1 << 20) as f64) * p.c_dram_mb
    }

    /// True when the configuration fits a DRAM ceiling.
    pub fn fits_dram(&self, ceiling: u64) -> bool {
        self.dram_bytes <= ceiling
    }
}

pub trait Backend {
    fn name(&self) -> String;
    /// Apply one block's record batch, then reach a durability point.
    fn commit_block(&mut self, batch: &CommitSink);
    /// Serve one state lookup as a WITNESS/PROOF walk: every record on the
    /// path must be fetched.
    fn serve_read(&mut self, rp: &ReadPath);

    /// Serve a whole block's lookups as ONE batched witness walk (a
    /// multiproof), which is what a real witness generator emits: records
    /// and pages shared by several paths are fetched ONCE for the batch,
    /// not once per key. The default is the unbatched per-key path, so a
    /// backend that has no notion of sharing keeps its old behaviour.
    fn serve_read_batch(&mut self, rps: &[ReadPath]) {
        for rp in rps {
            self.serve_read(rp);
        }
    }

    /// Serve one state lookup as an EVM VALUE read. This is what SLOAD does
    /// and it is much cheaper than a proof walk for both disciplines: the
    /// append store resolves the key through its RAM index and fetches one
    /// record; the page store resolves it through its value store (NOMT's
    /// beatree) without walking merkle pages at all.
    fn serve_value_read(&mut self, rp: &ReadPath);
    fn metrics(&self) -> Metrics;
    /// Final flush + any end-of-run accounting.
    fn finalize(&mut self);

    /// Called BEFORE genesis when a cache sweep is going to run, so a
    /// backend can build side models that have to see every record from the
    /// first one. Default: nothing.
    fn prepare_cache_sweep(&mut self) {}
    /// As `cache_profile`, for a page store with its value store included:
    /// one cache over merkle pages and value-store leaf pages together.
    fn value_store_profile(&self) -> Option<mrc::CacheProfile> {
        None
    }
    /// Start recording what a bounded cache would have missed on the write
    /// path (O7). Called after genesis, so the stack starts empty and an
    /// entry untouched since genesis counts as cold. Default: not profiled.
    fn enable_cache_profile(&mut self) {}
    /// The measured window starts here: drop warm-up counts, keep state.
    fn begin_measurement(&mut self) {}
    /// Charge the read of a value's previous copy when it is overwritten or
    /// deleted. A store that already reads what it overwrites ignores this.
    fn charge_old_value_reads(&mut self) {}
    /// Serve one execution read of `key`, which the block reads but does
    /// not change, through the same bounded cache as the write path.
    /// `present` is false when the key does not exist (the read returned
    /// zero). Default: not modeled.
    fn serve_state_read(&mut self, _key: &[u8], _present: bool) {}
    /// Start keeping the ids of index updates, in order (stores that keep
    /// one: the log store's index entries, the page store's pages).
    fn record_index_stream(&mut self) {}
    /// Distinct ids per power-of-two window of the index updates since
    /// `record_index_stream`, and how many updates there were.
    fn index_curve(&self) -> Option<(Vec<(u64, f64)>, u64)> {
        None
    }
    /// The bounded-cache curve over the measured window, if profiled.
    fn cache_profile(&self) -> Option<mrc::CacheProfile> {
        None
    }
    /// Backend-specific report section (markdown).
    fn extra_report(&self) -> String {
        String::new()
    }
}
