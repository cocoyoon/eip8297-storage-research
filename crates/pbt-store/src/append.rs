//! Append-only backend: every changed record is appended to a log; a
//! compact in-memory index maps record identity to its latest location;
//! superseded and deleted records become garbage reclaimed by compaction.
//! WiscKey-lineage value log with QMDB-style identity management (pattern,
//! not code -- draft SS6.2). Write path does real file IO; reads and
//! compaction are counter-modeled.

use crate::mrc::{id64, CacheProfile, StackProfile};
use crate::{Backend, Metrics};
use pbt_ref::tree::{CommitSink, ReadPath, RecordKind, RecordOp};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::Instant;

/// Record header on the log: 1B kind + 3B len.
const HEADER: u64 = 4;

/// A branch record commits BOTH children's hashes:
/// `H(0x01 || prefix || left_hash || right_hash)`. When one child changes,
/// recomputing the parent needs the OTHER child's hash, and that hash lives
/// in the parent's own previous record. So a rewritten branch is a genuine
/// read-modify-write for a log store, exactly as it is for a page store.
/// The harness previously charged this to neither, because the reference
/// tree is fully memory-resident; that is free at test scale and impossible
/// at mainnet scale, so it is now a modeled choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BranchHashPolicy {
    /// Read the previous branch record back to recover the sibling hash.
    /// Costs one random read per rewritten non-resident branch.
    ReadBack,
    /// Keep every live branch's two child hashes in DRAM beside the index.
    /// Costs 64 bytes per live branch and no read.
    CacheInDram,
    /// Charge neither. Only correct if the whole tree is memory-resident,
    /// which is the assumption this policy exists to make visible.
    Free,
}

/// Bytes held per live branch under `CacheInDram`.
///
/// Not 64. The branch preimage is `H(0x01 || encode_bit_prefix(prefix) ||
/// left || right)`, so the prefix is needed to recompute the hash at all,
/// and the index key of a child at depth d+1 cannot be derived without the
/// depth-d prefix either. The cache therefore has to hold the whole branch
/// record body, measured at a mean of 70.2 bytes on this workload, not the
/// two digests alone.
const BRANCH_HASH_BYTES: u64 = 70;

/// Bookkeeping per entry of a BOUNDED branch cache, on top of the record:
/// two 8-byte recency links. ASSUMPTION, not a measurement. A cache that
/// holds everything needs none of it, which is why `CacheInDram` charges
/// the bare record.
const BRANCH_CACHE_OVERHEAD: u64 = 16;

/// True when bit path `a` is a proper prefix of bit path `b`, both in
/// `encode_bit_prefix` form (2-byte big-endian bit count, bits MSB first).
fn is_bit_prefix(a: &[u8], b: &[u8]) -> bool {
    let la = u16::from_be_bytes([a[0], a[1]]) as usize;
    let lb = u16::from_be_bytes([b[0], b[1]]) as usize;
    if la >= lb {
        return false;
    }
    let full = la / 8;
    if a[2..2 + full] != b[2..2 + full] {
        return false;
    }
    let rem = la % 8;
    if rem == 0 {
        return true;
    }
    let mask = 0xFFu8 << (8 - rem);
    (a[2 + full] & mask) == (b[2 + full] & mask)
}

/// Node-tree position of a child of `parent`, in the same packing
/// `pos_key` uses: 2-byte big-endian bit count, then the bits packed MSB
/// first. DIAGNOSTIC helper only.
fn child_npos(parent: &[u8], bit: u8) -> Vec<u8> {
    let l = u16::from_be_bytes([parent[0], parent[1]]) as usize;
    let nl = l + 1;
    let nbytes = nl.div_ceil(8);
    let mut out = Vec::with_capacity(2 + nbytes);
    out.extend_from_slice(&(nl as u16).to_be_bytes());
    out.extend_from_slice(&parent[2..]);
    out.resize(2 + nbytes, 0);
    if bit == 1 {
        out[2 + l / 8] |= 1 << (7 - (l % 8));
    }
    out
}

pub struct AppendCore {
    /// branch records keyed by position; leaf records keyed by leaf key.
    ///
    /// PHASE 3 DEBT (draft §6.5): this index is memory-resident and never
    /// persisted, so a restart rebuilds it by scanning the whole log --
    /// ~25 MB of index from ~140 MB of log at the measured state size,
    /// growing linearly. With the page tier dropped this is the dominant
    /// unmodeled recovery cost, and it is priced in no table.
    /// Keyed by `id64` of the record identity: 8 bytes an entry instead of
    /// the identity itself. A collision among the ~10^8 records a run holds
    /// has odds near 10^-4 and would only miscount one entry.
    pub index: HashMap<u64, u32>,
    pub m: Metrics,
    compact_ratio: f64,
    #[allow(dead_code)]
    compact_floor: u64,
    /// Garbage created since the last amortized compaction charge.
    pending_garbage: u64,
    /// Cumulative bytes reclaimed through the amortized charge.
    pub compaction_debt_bytes: u64,
    /// Records shallower than this are held in DRAM and read free, exactly
    /// the grant the paged discipline gets. Without it the comparison hands
    /// one side a free cache and charges the other for the same bytes.
    pub resident_depth: u32,
    /// Bytes of those resident records (counted toward the DRAM budget).
    resident_bytes: u64,
    resident_ids: std::collections::HashSet<u64>,
    pub branch_policy: BranchHashPolicy,
    /// DIAGNOSTIC: exact index-entry census by kind, counted at insert/remove
    /// of the index key itself rather than from the record's op flag.
    pub idx_branches: i64,
    pub idx_leaves: i64,
    /// Read-modify-write reads charged on the write path.
    pub rmw_reads: u64,
    /// DIAGNOSTIC (charges nothing): of those RMW reads, how many are
    /// AVOIDABLE because BOTH children of the rewritten branch are
    /// themselves in this block's batch -- their new hashes are already in
    /// hand, so no sibling lookup is needed. `ReadBack` as modeled charges
    /// every rewritten branch, so this counter bounds its pessimism.
    pub rmw_sibling_dirty: u64,
    /// Bounded-cache profile of the write-path branch reads (O7). Only
    /// meaningful under `ReadBack`, whose charged reads are exactly the
    /// accesses a cache would be asked to serve.
    pub profile: Option<StackProfile>,
    /// Every index update after genesis, in order: the id of each record
    /// written and of each record deleted. Kept when asked for, to model an
    /// index held on disk (`crate::lsm`).
    pub index_stream: Option<crate::lsm::DistinctCounter>,
    /// Branch ids inside the resident grant (never cached: already free).
    resident_branches: u64,
    /// DIAGNOSTIC (charges nothing): new branches created by splitting a
    /// leaf that this block did not otherwise touch. Building such a branch
    /// needs the displaced leaf's KEY to find the diverging bit, and the
    /// parent holds only its hash, so the leaf record has to be fetched.
    /// No backend in this crate charges that read. A page store pays the
    /// same fetch from its value store, so it is not an asymmetry between
    /// disciplines, but it is missing from every absolute cost here.
    pub displaced_leaf_splits: u64,
    /// MEASURED while profiling: see `CacheProfile::liveness_lookups`.
    liveness_lookups: u64,
    /// MEASURED while profiling: see `CacheProfile::relocations`.
    superseded: u64,
    /// Charge the read of a leaf's previous copy when it is overwritten or
    /// deleted, and let those copies share the branch cache. Off by
    /// default: the commit itself writes leaves blind. On when the page
    /// store it is compared with is charged the read of the value-store
    /// page it overwrites, because both reads are the same event: the
    /// executing transaction looks at a value before it changes it.
    pub old_value_reads: bool,
    /// MEASURED while profiling: see `CacheProfile::leaf_fetches`.
    leaf_fetches: u64,
    /// DRAM those fetched leaves would occupy as cache entries, summed.
    leaf_fetch_entry_bytes: u64,
    /// MEASURED while profiling: see `CacheProfile::appended_bytes`.
    appended_bytes: u64,
}

/// Cache identity of a leaf record, kept apart from branch positions.
fn leaf_cache_id(key: &[u8]) -> u64 {
    id64(key) ^ 0x9E37_79B9_7F4A_7C15
}

impl AppendCore {
    pub fn new() -> Self {
        Self::with_resident_depth(0)
    }

    pub fn with_resident_depth(resident_depth: u32) -> Self {
        Self::with_policy(resident_depth, BranchHashPolicy::ReadBack)
    }

    pub fn with_policy(resident_depth: u32, branch_policy: BranchHashPolicy) -> Self {
        AppendCore {
            index: HashMap::new(),
            m: Metrics::default(),
            compact_ratio: 0.5,
            compact_floor: 16 << 20,
            pending_garbage: 0,
            compaction_debt_bytes: 0,
            resident_depth,
            resident_bytes: 0,
            resident_ids: std::collections::HashSet::new(),
            branch_policy,
            idx_branches: 0,
            idx_leaves: 0,
            rmw_reads: 0,
            rmw_sibling_dirty: 0,
            profile: None,
            index_stream: None,
            resident_branches: 0,
            displaced_leaf_splits: 0,
            liveness_lookups: 0,
            superseded: 0,
            old_value_reads: false,
            leaf_fetches: 0,
            leaf_fetch_entry_bytes: 0,
            appended_bytes: 0,
        }
    }

    /// Bounded-cache curve for this store's write-path branch reads.
    ///
    /// The fixed part is what `ReadBack` already holds: the compact index
    /// and the resident top. Whatever a DRAM budget leaves after that buys
    /// cache entries of one branch record plus recency bookkeeping.
    pub fn cache_profile(&self) -> Option<CacheProfile> {
        let p = self.profile.as_ref()?;
        let fixed = (self.index.len() as u64 * 154) / 10 + self.resident_bytes;
        let cacheable = self
            .live_branch_count()
            .saturating_sub(self.resident_branches);
        // One entry size for the whole cache. With leaves in it, the mean
        // over what the window actually asked the cache for.
        let branch_entry = BRANCH_HASH_BYTES + BRANCH_CACHE_OVERHEAD;
        let window = p.curve(1, 0, 0);
        let entry = if self.leaf_fetches > 0 {
            ((window.accesses - self.leaf_fetches) * branch_entry + self.leaf_fetch_entry_bytes)
                .div_ceil(window.accesses.max(1))
        } else {
            branch_entry
        };
        let cacheable = if self.old_value_reads {
            self.index.len() as u64
        } else {
            cacheable
        };
        let mut c = p.curve_indexed(entry, fixed, cacheable, self.index.len() as u64, 15.4);
        c.leaf_fetches = self.leaf_fetches;
        c.appended_bytes = self.appended_bytes;
        c.liveness_lookups = self.liveness_lookups;
        c.relocations = self.superseded;
        c.state_keys = self.idx_leaves.max(0) as u64;
        c.displaced_leaf_splits = self.displaced_leaf_splits;
        Some(c)
    }

    /// DRAM this discipline requires: the compact index (QMDB's measured
    /// ~15.4 B/key in-memory indexer) plus the resident top-of-tree records.
    pub fn dram_bytes(&self) -> u64 {
        let base = (self.index.len() as u64 * 154) / 10 + self.resident_bytes;
        match self.branch_policy {
            BranchHashPolicy::CacheInDram => base + self.live_branch_count() * BRANCH_HASH_BYTES,
            _ => base,
        }
    }

    /// Live branch records (the `CacheInDram` DRAM term's multiplicand),
    /// from index membership. The trie is strictly binary with two
    /// non-optional children, so the true count is live leaves minus one.
    ///
    /// This counter only APPROXIMATES that. When a delete merges a parent
    /// away and the surviving child is a branch, the tree reports the
    /// survivor's rewrite and the parent's delete at the same position;
    /// `apply` inserts then removes that key, so the index loses the live
    /// position and keeps the survivor's old one. The count stays right at
    /// that moment and drifts when either position is touched again. It has
    /// been observed a few hundred off in either direction at ~1.7M
    /// branches (under 0.03%). No test asserts the identity. Fixing it
    /// needs the tree to report the survivor's OLD position.
    pub fn live_branch_count(&self) -> u64 {
        self.idx_branches.max(0) as u64
    }

    /// Apply the records selected by `take(node_depth)`; returns bytes appended.
    pub fn apply(&mut self, batch: &CommitSink, take: impl Fn(u32) -> bool) -> u64 {
        // DIAGNOSTIC set: node-tree positions of every record in this batch.
        let dirty_npos: std::collections::HashSet<&[u8]> = batch
            .records
            .iter()
            .filter(|r| take(r.node_depth))
            .map(|r| r.node_pos.as_slice())
            .collect();
        // Bounded-cache profile, two batch-level facts it needs up front.
        //
        // `split_heads`: a key that diverges INSIDE an existing branch's
        // prefix puts a brand-new head branch at that branch's position and
        // demotes the old one to a new position. The demoted record arrives
        // as a rewrite of an id nobody has seen, but what the engine had to
        // consult was the old record at the head's position.
        //
        // `written_branches`: when a delete merges a parent away and its
        // surviving child is a branch, the survivor is rewritten AT the
        // parent's position and the delete event names that same position.
        // The position is still occupied, so it must not leave the cache.
        let (split_heads, written_branches): (Vec<&[u8]>, HashSet<u64>) = match &self.profile {
            Some(p) => {
                let branches = || {
                    batch
                        .records
                        .iter()
                        .filter(|r| take(r.node_depth) && r.kind == RecordKind::Branch)
                };
                (
                    branches()
                        .filter(|r| r.op == RecordOp::Create && p.contains(id64(&r.pos)))
                        .map(|r| r.pos.as_slice())
                        .collect(),
                    branches().map(|r| id64(&r.pos)).collect(),
                )
            }
            None => (Vec::new(), HashSet::new()),
        };
        let mut appended = 0u64;
        for r in &batch.records {
            if !take(r.node_depth) {
                continue;
            }
            let id = match r.kind {
                RecordKind::Leaf => r.leaf_key.as_ref().unwrap().to_vec(),
                RecordKind::Branch => r.pos.clone(),
            };
            let is_branch = r.kind == RecordKind::Branch;
            if is_branch && r.op == RecordOp::Create {
                let l = dirty_npos.contains(child_npos(&r.node_pos, 0).as_slice());
                let rt = dirty_npos.contains(child_npos(&r.node_pos, 1).as_slice());
                if l != rt {
                    self.displaced_leaf_splits += 1;
                }
            }
            if is_branch && r.node_depth >= self.resident_depth {
                if let Some(p) = self.profile.as_mut() {
                    let h = id64(&r.pos);
                    match r.op {
                        // In hand: goes to the top of the cache unread.
                        RecordOp::Create => p.place(h),
                        // The same condition `ReadBack` charges a read on.
                        RecordOp::Rewrite => {
                            let miss_bytes = (r.bytes as u64).div_ceil(512) * 512;
                            let head = if p.contains(h) {
                                None
                            } else {
                                split_heads
                                    .iter()
                                    .filter(|s| is_bit_prefix(s, &r.pos))
                                    .max_by_key(|s| u16::from_be_bytes([s[0], s[1]]))
                            };
                            match head {
                                Some(head) => {
                                    // Records are emitted children first, so
                                    // the head has not been placed yet and
                                    // still sits where the old record was.
                                    p.access(id64(head), miss_bytes);
                                    p.place(h);
                                }
                                None => {
                                    p.access(h, miss_bytes);
                                }
                            }
                        }
                    }
                }
            }
            if is_branch
                && r.op == RecordOp::Rewrite
                && self.branch_policy == BranchHashPolicy::ReadBack
                && r.node_depth >= self.resident_depth
            {
                // Recover the unchanged child's hash from the old record.
                self.m.page_reads += 1;
                self.m.read_bytes += (r.bytes as u64).div_ceil(512) * 512;
                self.rmw_reads += 1;
                if dirty_npos.contains(child_npos(&r.node_pos, 0).as_slice())
                    && dirty_npos.contains(child_npos(&r.node_pos, 1).as_slice())
                {
                    // Both children changed in this batch, so their new
                    // hashes are already in hand and no recovery read is
                    // really needed. Measured under 3% and shrinking.
                    self.rmw_sibling_dirty += 1;
                }
            }
            let sz = r.bytes as u64 + HEADER;
            let hid = id64(&id);
            if r.node_depth < self.resident_depth && self.resident_ids.insert(hid) {
                self.resident_bytes += r.bytes as u64;
                if is_branch {
                    self.resident_branches += 1;
                }
            }
            // Census keyed off actual index membership. A batch carries
            // deletes for nodes created and merged away inside one block
            // that were never stored, so counting record ops instead runs
            // ~6% low against the structural truth (branches = leaves - 1).
            let stream_id = self.index_stream.as_ref().map(|_| hid);
            if let Some(old) = self.index.insert(hid, r.bytes) {
                self.m.garbage_bytes += old as u64 + HEADER;
                self.pending_garbage += old as u64 + HEADER;
                self.m.live_bytes -= old as u64 + HEADER;
                if !is_branch && self.old_value_reads {
                    // The value was read before it was changed: one fetch
                    // of the old copy, which also tells where it is.
                    let miss_bytes = (old as u64 + HEADER).div_ceil(512) * 512;
                    self.m.page_reads += 1;
                    self.m.read_bytes += miss_bytes;
                    if let Some(p) = self.profile.as_mut() {
                        p.access(leaf_cache_id(r.leaf_key.as_ref().unwrap()), miss_bytes);
                        self.leaf_fetches += 1;
                        self.leaf_fetch_entry_bytes += r.bytes as u64 + BRANCH_CACHE_OVERHEAD;
                    }
                }
                if self.profile.is_some() {
                    self.superseded += 1;
                    if !is_branch && !self.old_value_reads {
                        // An overwritten leaf is written blind; only marking
                        // its old copy dead needs the old location.
                        self.liveness_lookups += 1;
                    }
                }
            } else if is_branch {
                self.idx_branches += 1;
            } else {
                self.idx_leaves += 1;
                if self.old_value_reads {
                    // A new leaf is in hand: top of the cache, unread.
                    if let Some(p) = self.profile.as_mut() {
                        p.place(leaf_cache_id(r.leaf_key.as_ref().unwrap()));
                    }
                }
            }
            self.m.live_bytes += sz;
            appended += sz;
            self.m.record_writes += 1;
            if let (Some(s), Some(h)) = (self.index_stream.as_mut(), stream_id) {
                s.push(h);
            }
        }
        for d in &batch.deletes {
            if !take(d.node_depth) {
                continue;
            }
            let id = match d.leaf_key {
                Some(ref k) => k.to_vec(),
                None => d.pos.clone(),
            };
            let hid = id64(&id);
            if self.resident_ids.remove(&hid) {
                if let Some(sz) = self.index.get(&hid) {
                    self.resident_bytes = self.resident_bytes.saturating_sub(*sz as u64);
                }
                if !d.was_leaf {
                    self.resident_branches = self.resident_branches.saturating_sub(1);
                }
            }
            if !d.was_leaf {
                if let Some(p) = self.profile.as_mut() {
                    let h = id64(&d.pos);
                    if !written_branches.contains(&h) {
                        p.remove(h);
                    }
                }
            }
            if let Some(old) = self.index.remove(&hid) {
                self.m.garbage_bytes += old as u64 + HEADER;
                self.pending_garbage += old as u64 + HEADER;
                self.m.live_bytes -= old as u64 + HEADER;
                if d.was_leaf && self.old_value_reads {
                    // Deleting a value is a change to it like any other.
                    let miss_bytes = (old as u64 + HEADER).div_ceil(512) * 512;
                    self.m.page_reads += 1;
                    self.m.read_bytes += miss_bytes;
                    if let Some(p) = self.profile.as_mut() {
                        let h = leaf_cache_id(&id);
                        p.access(h, miss_bytes);
                        p.remove(h);
                        self.leaf_fetches += 1;
                        self.leaf_fetch_entry_bytes += old as u64 + BRANCH_CACHE_OVERHEAD;
                    }
                }
                if self.profile.is_some() {
                    self.superseded += 1;
                    // A position rewritten in this batch was already looked
                    // up (or cached) for the rewrite; so was a leaf whose
                    // old copy has just been fetched.
                    let fetched = d.was_leaf && self.old_value_reads;
                    if !fetched && (d.was_leaf || !written_branches.contains(&id64(&d.pos))) {
                        self.liveness_lookups += 1;
                    }
                }
                if d.was_leaf {
                    self.idx_leaves -= 1;
                } else {
                    self.idx_branches -= 1;
                }
            }
            if !d.was_leaf {}
            self.m.record_deletes += 1;
            if let Some(s) = self.index_stream.as_mut() {
                s.push(id64(&id));
            }
        }
        self.m.seq_bytes_written += appended;
        self.appended_bytes += appended;
        appended
    }

    /// Steady-state compaction charge, applied per block rather than as a
    /// one-shot debt at the end.
    ///
    /// A threshold-triggered model made a run's score depend on where the
    /// trigger happened to land relative to the last block: at large state
    /// and short runs a single end-of-run compaction was 96% of all
    /// sequential writes, which corrupted every cross-scale comparison.
    ///
    /// Amortized instead: under a `compact_ratio` garbage trigger, a
    /// compaction reads (live + garbage) and rewrites live, reclaiming
    /// `garbage`. With garbage ~= live at the trigger, each byte of garbage
    /// created eventually costs ~2 bytes read and ~1 byte written. Charging
    /// that when the garbage is created makes the cost scale-independent
    /// and run-length-independent, which is what the comparison needs.
    pub fn maintain(&mut self) {
        let g = self.pending_garbage;
        if g == 0 {
            return;
        }
        self.pending_garbage = 0;
        let denom = (1.0 - self.compact_ratio).max(0.05);
        let read_factor = (1.0 / denom) as u64; // 2 at ratio 0.5
        self.m.seq_bytes_read += g * read_factor;
        self.m.seq_bytes_written += g;
        self.m.garbage_bytes = self.m.garbage_bytes.saturating_sub(g);
        self.compaction_debt_bytes += g;
    }

    /// Kept for API compatibility; the amortized model owes nothing at the
    /// end of a run.
    pub fn settle(&mut self) {}

    /// Index lookups are RAM; each selected non-resident record costs one
    /// random read. Resident (shallow) records are served from DRAM, the
    /// same grant the paged discipline receives.
    ///
    /// Served INDEPENDENTLY: no dedup, so a branch record every key on the
    /// block shares is fetched once per key.
    pub fn read(&mut self, rp: &ReadPath, take: impl Fn(u32) -> bool) {
        self.read_into(rp, &take, &mut None);
    }

    /// One multiproof over a block's whole read set: a record shared by
    /// several paths -- every upper-trie branch is -- is fetched ONCE for
    /// the batch, matching how a witness generator actually assembles a
    /// block's proof.
    pub fn read_batch(&mut self, rps: &[ReadPath], take: impl Fn(u32) -> bool) {
        let mut seen = Some(std::collections::HashSet::new());
        for rp in rps {
            self.read_into(rp, &take, &mut seen);
        }
    }

    /// `seen` = None means per-key service; Some(set) dedups across a batch.
    /// Record identity is the same one the index uses: the packed bit
    /// position for a branch, the complete key for a leaf.
    fn read_into<F: Fn(u32) -> bool>(
        &mut self,
        rp: &ReadPath,
        take: &F,
        seen: &mut Option<std::collections::HashSet<Vec<u8>>>,
    ) {
        const SECTOR: u64 = 512;
        // Under CacheInDram the live branch set is held in DRAM and paid for
        // in dram_bytes(), so a path descent is served from memory and only
        // the leaf touches disk. Charging the branches here as well would
        // bill the cache twice and credit it once.
        let branches_cached = self.branch_policy == BranchHashPolicy::CacheInDram;
        for (i, (_sb, pos, _npos, bytes)) in rp.branches.iter().enumerate() {
            let d = i as u32;
            if take(d) && d >= self.resident_depth && !branches_cached {
                if let Some(set) = seen.as_mut() {
                    if !set.insert(pos.clone()) {
                        continue;
                    }
                }
                self.m.page_reads += 1;
                self.m.read_bytes += (*bytes as u64).div_ceil(SECTOR) * SECTOR;
            }
        }
        let d = rp.branches.len() as u32;
        if let Some((k, bytes)) = &rp.leaf {
            if take(d) && d >= self.resident_depth {
                if let Some(set) = seen.as_mut() {
                    if !set.insert(k.to_vec()) {
                        return;
                    }
                }
                self.m.page_reads += 1;
                self.m.read_bytes += (*bytes as u64).div_ceil(SECTOR) * SECTOR;
            }
        }
    }
}

pub struct AppendBackend {
    core: AppendCore,
    file: Option<File>,
    buf: Vec<u8>,
}

impl AppendBackend {
    pub fn new(path: Option<&str>, resident_depth: u32) -> Self {
        Self::with_policy(path, resident_depth, BranchHashPolicy::ReadBack)
    }

    pub fn with_policy(path: Option<&str>, resident_depth: u32, policy: BranchHashPolicy) -> Self {
        let file = path.map(|p| {
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(p)
                .expect("log file")
        });
        AppendBackend {
            core: AppendCore::with_policy(resident_depth, policy),
            file,
            buf: Vec::new(),
        }
    }
}

impl Backend for AppendBackend {
    fn name(&self) -> String {
        let p = match self.core.branch_policy {
            BranchHashPolicy::ReadBack => "readback",
            BranchHashPolicy::CacheInDram => "hashcache",
            BranchHashPolicy::Free => "free",
        };
        format!("append-{p}(res={})", self.core.resident_depth)
    }

    fn commit_block(&mut self, batch: &CommitSink) {
        let t0 = Instant::now();
        let appended = self.core.apply(batch, |_| true);
        if let Some(f) = &mut self.file {
            // Real sequential write of the batch (placeholder payload).
            self.buf.clear();
            self.buf.resize(appended as usize, 0xAB);
            f.write_all(&self.buf).unwrap();
            f.sync_data().unwrap();
        }
        self.core.m.fsyncs += 1;
        self.core.maintain();
        self.core.m.wall_ns += t0.elapsed().as_nanos() as u64;
    }

    fn serve_read(&mut self, rp: &ReadPath) {
        self.core.m.reads_served += 1;
        self.core.read(rp, |_| true);
    }

    fn serve_read_batch(&mut self, rps: &[ReadPath]) {
        self.core.m.reads_served += rps.len() as u64;
        self.core.read_batch(rps, |_| true);
    }

    fn serve_value_read(&mut self, rp: &ReadPath) {
        self.core.m.reads_served += 1;
        // Index hit is RAM; one record fetch, one sector.
        if let Some((_k, bytes)) = &rp.leaf {
            self.core.m.page_reads += 1;
            self.core.m.read_bytes += (*bytes as u64).div_ceil(512) * 512;
        }
    }

    fn metrics(&self) -> Metrics {
        let mut m = self.core.m.clone();
        m.dram_bytes = self.core.dram_bytes();
        m
    }

    fn finalize(&mut self) {
        self.core.settle();
        if let Some(f) = &mut self.file {
            f.sync_all().unwrap();
            self.core.m.disk_bytes = f.metadata().unwrap().len();
        }
    }

    fn enable_cache_profile(&mut self) {
        // Only the read-back store performs the reads a cache would save.
        if self.core.branch_policy == BranchHashPolicy::ReadBack {
            self.core.profile = Some(StackProfile::new());
        }
    }

    fn begin_measurement(&mut self) {
        if let Some(p) = self.core.profile.as_mut() {
            p.restart_counts();
        }
        self.core.displaced_leaf_splits = 0;
        self.core.liveness_lookups = 0;
        self.core.superseded = 0;
        self.core.leaf_fetches = 0;
        self.core.leaf_fetch_entry_bytes = 0;
        self.core.appended_bytes = 0;
    }

    fn charge_old_value_reads(&mut self) {
        self.core.old_value_reads = true;
    }

    fn record_index_stream(&mut self) {
        // One id in 16 tracked: the estimate is unbiased and the memory
        // stays a sixteenth of the distinct ids.
        self.core.index_stream = Some(crate::lsm::DistinctCounter::new(4));
    }

    fn index_curve(&self) -> Option<(Vec<(u64, f64)>, u64)> {
        self.core
            .index_stream
            .as_ref()
            .map(|c| (c.curve(), c.len()))
    }

    fn serve_state_read(&mut self, key: &[u8], present: bool) {
        self.core.m.reads_served += 1;
        // An absent key costs no fetch: the index (or, with the index on
        // disk, its per-entry filter in DRAM) already says it is not there.
        if !present {
            return;
        }
        let bytes = pbt_ref::tree::leaf_record_bytes(key.len()) as u64;
        let miss_bytes = (bytes + HEADER).div_ceil(512) * 512;
        self.core.m.page_reads += 1;
        self.core.m.read_bytes += miss_bytes;
        if let Some(p) = self.core.profile.as_mut() {
            p.access_tagged(leaf_cache_id(key), miss_bytes);
            self.core.leaf_fetches += 1;
            self.core.leaf_fetch_entry_bytes += bytes + BRANCH_CACHE_OVERHEAD;
        }
    }

    fn cache_profile(&self) -> Option<CacheProfile> {
        self.core.cache_profile()
    }

    fn extra_report(&self) -> String {
        format!(
            "- {}: live {} MB, amortized compaction reclaim {} MB, live records {}, live branches {}, idx_branches {}, idx_leaves {}, write-path RMW reads {} (sibling-dirty {}), uncharged displaced-leaf splits {}, dram bytes {}\n",
            self.name(),
            self.core.m.live_bytes >> 20,
            self.core.compaction_debt_bytes >> 20,
            self.core.index.len(),
            self.core.live_branch_count(),
            self.core.idx_branches,
            self.core.idx_leaves,
            self.core.rmw_reads,
            self.core.rmw_sibling_dirty,
            self.core.displaced_leaf_splits,
            self.core.dram_bytes()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(branch_pos: &[&[u8]], leaf: &[u8]) -> ReadPath {
        ReadPath {
            branches: branch_pos
                .iter()
                .enumerate()
                .map(|(i, p)| (i as u32, p.to_vec(), p.to_vec(), 100u32))
                .collect(),
            leaf: Some((leaf.to_vec().into_boxed_slice(), 100)),
        }
    }

    /// Two paths sharing their first branch record. Served independently the
    /// shared record is fetched twice; batched into one multiproof it is
    /// fetched once.
    #[test]
    fn batched_reads_charge_a_shared_record_once() {
        let a = path(&[b"root", b"left"], b"key-a");
        let b = path(&[b"root", b"right"], b"key-b");

        let mut per_key = AppendCore::with_resident_depth(0);
        per_key.read(&a, |_| true);
        per_key.read(&b, |_| true);
        assert_eq!(per_key.m.page_reads, 6); // 2 branches + 1 leaf, twice

        let mut batched = AppendCore::with_resident_depth(0);
        batched.read_batch(&[a, b], |_| true);
        assert_eq!(batched.m.page_reads, 5); // `root` fetched once
        assert!(batched.m.read_bytes < per_key.m.read_bytes);
    }

    /// The same key read twice in a block needs one witness, not two.
    #[test]
    fn batched_reads_dedup_a_repeated_key() {
        let a = path(&[b"root"], b"key-a");
        let mut one = AppendCore::with_resident_depth(0);
        one.read(&a, |_| true);
        let mut twice = AppendCore::with_resident_depth(0);
        twice.read_batch(
            &[path(&[b"root"], b"key-a"), path(&[b"root"], b"key-a")],
            |_| true,
        );
        assert_eq!(twice.m.page_reads, one.m.page_reads);
        assert_eq!(twice.m.read_bytes, one.m.read_bytes);
    }
}
