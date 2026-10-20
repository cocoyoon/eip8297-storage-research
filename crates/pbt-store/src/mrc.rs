//! LRU miss-ratio curve by stack distance (Mattson et al., 1970).
//!
//! LRU has the inclusion property: at any moment a cache of capacity C
//! holds exactly the C most recently used entries. One pass that records
//! each access's STACK DISTANCE -- the number of distinct other entries
//! touched since this entry was last touched -- therefore yields the miss
//! count of every capacity at once: an access is a hit iff distance < C.
//! Sweeping a bounded cache this way costs one instance instead of one per
//! cache size, and the curve is exact rather than sampled.
//!
//! Removal needs care. Deleting an entry from a real cache leaves a free
//! slot; it does not bring back whatever that cache evicted earlier. So a
//! removed entry stays on the stack as a HOLE that still occupies its
//! position, and the hole is used up by the next entry that arrives from
//! below it or from outside. Closing the gap instead would count evicted
//! entries as cached.
//!
//! Counter model only: nothing here does IO.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hasher;

/// Stable 64-bit identity for a record or page id. `DefaultHasher::new()`
/// is SipHash with fixed keys, so the value is the same in every run.
pub fn id64(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(bytes);
    h.finish()
}

pub struct StackProfile {
    /// entry -> its slot on the recency timeline (1-based).
    slot: HashMap<u64, u32>,
    /// Slots left behind by removed entries (see module docs).
    holes: BTreeSet<u32>,
    /// Fenwick tree over timeline slots; a 1 marks an occupied position,
    /// which is an entry's latest slot or a hole.
    fen: Vec<u32>,
    /// Last slot handed out.
    now: u32,
    /// hist[d] = counted accesses at stack distance exactly d.
    hist: Vec<u64>,
    /// Counted accesses to an entry that was not on the stack: never placed
    /// since profiling began, or removed. A miss at every capacity short of
    /// holding everything.
    cold: u64,
    /// Stack size at each counted cold access, in the order they happened.
    /// The stack never shrinks, so this is sorted. Everything on the stack
    /// was touched after the cold entry last was, so its true distance is
    /// at least that size: a certain miss for any capacity up to it.
    cold_sizes: Vec<u64>,
    accesses: u64,
    /// Bytes a miss would read, summed over counted accesses.
    read_bytes: u64,
    /// Stack size when counting last (re)started. A cold access after that
    /// point is to an entry untouched for at least this many distinct
    /// entries, so the curve is exact for capacities up to this size and an
    /// upper bound on misses above it.
    exact_upto: u64,
    /// The same counts for the TAGGED accesses only (execution reads), so a
    /// budget's misses can be split by when the store needs them.
    tag_hist: Vec<u64>,
    tag_cold_sizes: Vec<u64>,
    tag_accesses: u64,
}

impl Default for StackProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl StackProfile {
    pub fn new() -> Self {
        StackProfile {
            slot: HashMap::new(),
            holes: BTreeSet::new(),
            fen: vec![0; 1025],
            now: 0,
            hist: Vec::new(),
            cold: 0,
            cold_sizes: Vec::new(),
            accesses: 0,
            read_bytes: 0,
            exact_upto: 0,
            tag_hist: Vec::new(),
            tag_cold_sizes: Vec::new(),
            tag_accesses: 0,
        }
    }

    pub fn len(&self) -> u64 {
        self.slot.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.slot.is_empty()
    }

    pub fn contains(&self, id: u64) -> bool {
        self.slot.contains_key(&id)
    }

    fn add(&mut self, mut i: u32, up: bool) {
        let n = self.fen.len() as u32;
        while i < n {
            if up {
                self.fen[i as usize] += 1;
            } else {
                self.fen[i as usize] -= 1;
            }
            i += i & i.wrapping_neg();
        }
    }

    fn prefix(&self, mut i: u32) -> u64 {
        let mut s = 0u64;
        while i > 0 {
            s += self.fen[i as usize] as u64;
            i -= i & i.wrapping_neg();
        }
        s
    }

    /// Renumber occupied positions onto slots 1..=n in recency order and
    /// size the timeline to twice that, so memory tracks live entries
    /// rather than total accesses.
    fn compact(&mut self) {
        let mut order: Vec<(u32, Option<u64>)> =
            self.slot.iter().map(|(id, s)| (*s, Some(*id))).collect();
        order.extend(self.holes.iter().map(|s| (*s, None)));
        order.sort_unstable();
        let n = order.len();
        let cap = (2 * n).max(1024);
        self.fen = vec![0; cap + 1];
        self.holes.clear();
        for (i, (_, id)) in order.into_iter().enumerate() {
            let s = (i + 1) as u32;
            match id {
                Some(id) => {
                    self.slot.insert(id, s);
                }
                None => {
                    self.holes.insert(s);
                }
            }
            self.fen[s as usize] = 1;
        }
        // Linear Fenwick build.
        for i in 1..=cap {
            let j = i + (i & i.wrapping_neg());
            if j <= cap {
                self.fen[j] += self.fen[i];
            }
        }
        self.now = n as u32;
    }

    fn next_slot(&mut self) -> u32 {
        if self.now as usize + 1 >= self.fen.len() {
            self.compact();
        }
        self.now += 1;
        self.now
    }

    /// Positions above `id` on the stack (entries and holes), if it is on it.
    fn distance(&self, id: u64) -> Option<u64> {
        let s = *self.slot.get(&id)?;
        Some((self.slot.len() + self.holes.len()) as u64 - self.prefix(s))
    }

    /// Put `id` on top of the stack without counting an access: the entry
    /// is already in hand (just created, or just written in full).
    pub fn place(&mut self, id: u64) {
        let old = self.slot.remove(&id);
        let top_hole = self.holes.iter().next_back().copied();
        match (old, top_hole) {
            // Arriving from below a hole: every cache that could see the
            // hole but not this entry fills its free slot, so the hole is
            // used up; caches that held both keep a free slot, now at the
            // position this entry vacates.
            (Some(s), Some(h)) if h > s => {
                self.holes.remove(&h);
                self.add(h, false);
                self.holes.insert(s);
            }
            (Some(s), _) => self.add(s, false),
            // Arriving from outside uses up the shallowest free slot.
            (None, Some(h)) => {
                self.holes.remove(&h);
                self.add(h, false);
            }
            (None, None) => {}
        }
        let s = self.next_slot();
        self.slot.insert(id, s);
        self.add(s, true);
    }

    /// A counted access: the entry's previous contents are needed. Records
    /// its stack distance (or a cold miss), then moves it to the top.
    /// `miss_bytes` is what a miss would read from disk.
    pub fn access(&mut self, id: u64, miss_bytes: u64) -> Option<u64> {
        let d = self.distance(id);
        match d {
            Some(d) => {
                let d = d as usize;
                if self.hist.len() <= d {
                    self.hist.resize(d + 1, 0);
                }
                self.hist[d] += 1;
            }
            None => {
                self.cold += 1;
                self.cold_sizes
                    .push((self.slot.len() + self.holes.len()) as u64);
            }
        }
        self.accesses += 1;
        self.read_bytes += miss_bytes;
        self.place(id);
        d
    }

    /// `access`, and counted again among the tagged accesses.
    pub fn access_tagged(&mut self, id: u64, miss_bytes: u64) -> Option<u64> {
        let size = (self.slot.len() + self.holes.len()) as u64;
        let d = self.access(id, miss_bytes);
        match d {
            Some(d) => {
                let d = d as usize;
                if self.tag_hist.len() <= d {
                    self.tag_hist.resize(d + 1, 0);
                }
                self.tag_hist[d] += 1;
            }
            None => self.tag_cold_sizes.push(size),
        }
        self.tag_accesses += 1;
        d
    }

    /// The entry no longer exists. Its position stays occupied by a hole.
    pub fn remove(&mut self, id: u64) {
        if let Some(s) = self.slot.remove(&id) {
            self.holes.insert(s);
        }
    }

    /// Forget the counts but keep the stack: call at the end of a warm-up.
    pub fn restart_counts(&mut self) {
        self.hist.clear();
        self.cold = 0;
        self.cold_sizes.clear();
        self.accesses = 0;
        self.read_bytes = 0;
        self.exact_upto = self.slot.len() as u64;
        self.tag_hist.clear();
        self.tag_cold_sizes.clear();
        self.tag_accesses = 0;
    }

    /// Freeze into a curve. `entry_bytes` is DRAM per cached entry,
    /// `fixed_dram_bytes` what the discipline holds before caching anything,
    /// `cacheable` how many entries exist to be cached.
    pub fn curve(&self, entry_bytes: u64, fixed_dram_bytes: u64, cacheable: u64) -> CacheProfile {
        self.curve_indexed(entry_bytes, fixed_dram_bytes, cacheable, 0, 0.0)
    }

    /// As `curve`, for a discipline whose fixed part includes an index of
    /// `index_entries` entries at `index_entry_bytes` each. Kept separate so
    /// the index assumption can be varied after the run.
    pub fn curve_indexed(
        &self,
        entry_bytes: u64,
        fixed_dram_bytes: u64,
        cacheable: u64,
        index_entries: u64,
        index_entry_bytes: f64,
    ) -> CacheProfile {
        // suffix[c] = counted warm accesses with distance >= c.
        let mut suffix = vec![0u64; self.hist.len() + 1];
        for d in (0..self.hist.len()).rev() {
            suffix[d] = suffix[d + 1] + self.hist[d];
        }
        let mut tag_suffix = vec![0u64; self.tag_hist.len() + 1];
        for d in (0..self.tag_hist.len()).rev() {
            tag_suffix[d] = tag_suffix[d + 1] + self.tag_hist[d];
        }
        CacheProfile {
            tag_suffix,
            tag_cold_sizes: self.tag_cold_sizes.clone(),
            tag_accesses: self.tag_accesses,
            entry_bytes,
            fixed_dram_bytes,
            cacheable_entries: cacheable,
            exact_upto: self.exact_upto,
            suffix,
            cold: self.cold,
            cold_sizes: self.cold_sizes.clone(),
            accesses: self.accesses,
            read_bytes: self.read_bytes,
            index_entries,
            index_entry_bytes,
            uncharged_elided_touches: 0,
            uncharged_leaf_changes: 0,
            liveness_lookups: 0,
            relocations: 0,
            value_leaf_page_writes: 0,
            value_leaf_pages: 0,
            value_bbn_writes: 0,
            value_bbns: 0,
            state_keys: 0,
            displaced_leaf_splits: 0,
            leaf_fetches: 0,
            appended_bytes: 0,
        }
    }
}

/// One discipline's bounded-cache behaviour over the measured window: what
/// an LRU of any capacity would have missed on the write path.
#[derive(Clone)]
pub struct CacheProfile {
    tag_suffix: Vec<u64>,
    tag_cold_sizes: Vec<u64>,
    /// Tagged (execution-read) accesses among `accesses`.
    pub tag_accesses: u64,
    /// DRAM one cached entry occupies, bookkeeping included.
    pub entry_bytes: u64,
    /// DRAM held before any cache: index and resident top for the log
    /// store, resident pages for the page store.
    pub fixed_dram_bytes: u64,
    /// Entries that exist to be cached at the end of the run.
    pub cacheable_entries: u64,
    /// The curve is exact for capacities up to this many entries; above it
    /// cold-start accesses are counted as misses, so it is an upper bound.
    pub exact_upto: u64,
    suffix: Vec<u64>,
    pub cold: u64,
    /// Stack size at each cold access, ascending (see `StackProfile`).
    cold_sizes: Vec<u64>,
    /// Counted accesses = reads a store with no cache performs.
    pub accesses: u64,
    /// Bytes those reads move.
    pub read_bytes: u64,
    /// Index entries inside `fixed_dram_bytes`, and the bytes assumed for
    /// each. Zero for a discipline that addresses without an index.
    pub index_entries: u64,
    pub index_entry_bytes: f64,
    /// Page store only, DIAGNOSTIC: pages touched on the write path while
    /// below the elision threshold. The model charges them nothing, though
    /// the read path charges one leaf-store read to rebuild such a page.
    pub uncharged_elided_touches: u64,
    /// Page store only, DIAGNOSTIC: leaf records created, rewritten or
    /// deleted on the write path. A page slot holds a 32-byte node hash, so
    /// the leaf's key and value have to be persisted somewhere else, and the
    /// model charges nothing for that.
    pub uncharged_leaf_changes: u64,
    /// Log store only, MEASURED: records whose previous location has to be
    /// looked up for a reason other than reading them back, namely
    /// overwritten or deleted leaves and deleted branches, so the log can
    /// mark the old copy dead. Free while the index is in DRAM; an index
    /// lookup each once it is not.
    pub liveness_lookups: u64,
    /// Log store only, MEASURED: records superseded in the window. Under
    /// the amortized compaction model one live record is relocated per
    /// superseded one, and each relocation is one more index update.
    pub relocations: u64,
    /// Page store with value store only, MEASURED on a model of the value
    /// store's leaf layer: distinct leaf pages dirtied, summed over blocks.
    /// Each is one copy-on-write 4 KB page write per commit.
    pub value_leaf_page_writes: u64,
    /// Leaf pages the value store holds at the end of the run.
    pub value_leaf_pages: u64,
    /// As `value_leaf_page_writes`, for the bottom branch nodes above those
    /// leaf pages: distinct nodes with a dirtied leaf under them.
    pub value_bbn_writes: u64,
    /// Bottom branch nodes at the end of the run. All are resident.
    pub value_bbns: u64,
    /// Log store only: live leaves, i.e. state keys, at the end of the run.
    pub state_keys: u64,
    /// Log store only, MEASURED: new branches made by splitting a leaf the
    /// block did not otherwise touch. Each needs that leaf's record fetched.
    pub displaced_leaf_splits: u64,
    /// Log store only, MEASURED, and only when old-value reads are charged:
    /// the accesses among `accesses` that fetch the previous copy of an
    /// overwritten or deleted leaf rather than a branch.
    pub leaf_fetches: u64,
    /// Log store only, MEASURED: bytes appended by commits in the window.
    /// Compaction rewrites are not in it.
    pub appended_bytes: u64,
}

impl CacheProfile {
    /// Upper bound on the misses of an LRU holding `capacity` entries:
    /// every cold access counted as a miss. Exact when `capacity` is within
    /// `exact_upto`.
    pub fn misses_at(&self, capacity: u64) -> u64 {
        self.bounds_at(capacity).1
    }

    /// (lower, upper) bound on misses at `capacity`. A cold access is to an
    /// entry untouched since profiling began, so its true distance is at
    /// least the size the stack had when it happened: a certain miss for a
    /// capacity up to that size, unknown above it. The lower bound counts
    /// only the unknowns as hits, which is what a cache that held them from
    /// the start would do. Every cold access in the window happened at a
    /// size of `exact_upto` or more, so within that range the two agree.
    pub fn bounds_at(&self, capacity: u64) -> (u64, u64) {
        let warm = self.suffix.get(capacity as usize).copied().unwrap_or(0);
        let unknown = self.cold_sizes.partition_point(|s| *s < capacity) as u64;
        (self.cold - unknown + warm, self.cold + warm)
    }

    /// `bounds_at` for the tagged accesses alone.
    pub fn tag_bounds_at(&self, capacity: u64) -> (u64, u64) {
        let warm = self.tag_suffix.get(capacity as usize).copied().unwrap_or(0);
        let cold = self.tag_cold_sizes.len() as u64;
        let unknown = self.tag_cold_sizes.partition_point(|s| *s < capacity) as u64;
        (cold - unknown + warm, cold + warm)
    }

    /// The same curve under a different index size per entry.
    pub fn with_index_entry_bytes(&self, bytes: f64) -> CacheProfile {
        let mut c = self.clone();
        let old = (self.index_entries as f64 * self.index_entry_bytes) as u64;
        let new = (self.index_entries as f64 * bytes) as u64;
        c.fixed_dram_bytes = self.fixed_dram_bytes.saturating_sub(old) + new;
        c.index_entry_bytes = bytes;
        c
    }

    /// The same curve for a state with a different number of index
    /// entries, at the same bytes per entry. A replay indexes only the
    /// records it touches; the store it stands for indexes all of them.
    pub fn with_index_entries(&self, entries: u64) -> CacheProfile {
        let mut c = self.clone();
        let old = (self.index_entries as f64 * self.index_entry_bytes) as u64;
        let new = (entries as f64 * self.index_entry_bytes) as u64;
        c.fixed_dram_bytes = self.fixed_dram_bytes.saturating_sub(old) + new;
        c.index_entries = entries;
        c
    }

    /// The same curve with a different number of entries available to
    /// cache, for the same reason.
    pub fn with_cacheable(&self, entries: u64) -> CacheProfile {
        let mut c = self.clone();
        c.cacheable_entries = entries;
        c
    }

    /// The same curve with more DRAM held before any cache.
    pub fn with_fixed_extra(&self, bytes: u64) -> CacheProfile {
        let mut c = self.clone();
        c.fixed_dram_bytes += bytes;
        c
    }

    /// The same curve under a different DRAM cost per cached entry.
    pub fn with_entry_bytes(&self, bytes: u64) -> CacheProfile {
        let mut c = self.clone();
        c.entry_bytes = bytes;
        c
    }

    /// Entries a DRAM budget buys after the fixed part is paid; None when
    /// the fixed part alone does not fit.
    pub fn capacity_for(&self, dram_bytes: u64) -> Option<u64> {
        if dram_bytes < self.fixed_dram_bytes {
            return None;
        }
        Some(((dram_bytes - self.fixed_dram_bytes) / self.entry_bytes).min(self.cacheable_entries))
    }

    /// DRAM at which every cacheable entry is resident.
    pub fn full_dram_bytes(&self) -> u64 {
        self.fixed_dram_bytes + self.cacheable_entries * self.entry_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
    }

    #[derive(Clone, Copy)]
    enum Op {
        Access(u64),
        Place(u64),
        Remove(u64),
    }

    /// Reference: an explicit LRU of fixed capacity, MRU at the back.
    fn naive_misses(ops: &[Op], capacity: usize) -> u64 {
        let mut lru: Vec<u64> = Vec::new();
        let mut misses = 0u64;
        for op in ops {
            match *op {
                Op::Access(id) => {
                    if let Some(i) = lru.iter().position(|x| *x == id) {
                        lru.remove(i);
                    } else {
                        misses += 1;
                    }
                    lru.push(id);
                }
                Op::Place(id) => {
                    if let Some(i) = lru.iter().position(|x| *x == id) {
                        lru.remove(i);
                    }
                    lru.push(id);
                }
                Op::Remove(id) => {
                    if let Some(i) = lru.iter().position(|x| *x == id) {
                        lru.remove(i);
                    }
                }
            }
            while lru.len() > capacity {
                lru.remove(0);
            }
        }
        misses
    }

    fn stream(seed: u64, n: usize, universe: u64) -> Vec<Op> {
        let mut rng = Rng(seed);
        (0..n)
            .map(|_| {
                // Skewed ids so short and long distances both occur.
                let a = rng.next() % universe;
                let b = rng.next() % universe;
                let id = a.min(b);
                match rng.next() % 10 {
                    0 => Op::Place(id),
                    1 => Op::Remove(id),
                    _ => Op::Access(id),
                }
            })
            .collect()
    }

    /// The one-pass curve must agree with an explicit LRU at every capacity,
    /// across accesses, in-hand placements and removals.
    #[test]
    fn curve_matches_explicit_lru_at_every_capacity() {
        for seed in [1u64, 7, 99] {
            let ops = stream(seed, 6000, 400);
            let mut p = StackProfile::new();
            for op in &ops {
                match *op {
                    Op::Access(id) => {
                        p.access(id, 512);
                    }
                    Op::Place(id) => p.place(id),
                    Op::Remove(id) => p.remove(id),
                }
            }
            // `cacheable` set past the universe so the full-residency
            // shortcut never fires; an explicit LRU starts empty.
            let c = p.curve(1, 0, u64::MAX);
            for cap in [0usize, 1, 2, 3, 5, 8, 13, 50, 100, 200, 399, 400, 1000] {
                assert_eq!(
                    c.misses_at(cap as u64),
                    naive_misses(&ops, cap),
                    "seed {seed} capacity {cap}"
                );
            }
            assert_eq!(c.misses_at(0), c.accesses);
        }
    }

    /// Timeline compaction must not disturb distances.
    #[test]
    fn compaction_preserves_distances() {
        let ops = stream(5, 40_000, 300); // far more accesses than slots
        let mut p = StackProfile::new();
        for op in &ops {
            match *op {
                Op::Access(id) => {
                    p.access(id, 512);
                }
                Op::Place(id) => p.place(id),
                Op::Remove(id) => p.remove(id),
            }
        }
        let c = p.curve(1, 0, u64::MAX);
        for cap in [1usize, 10, 100, 250] {
            assert_eq!(c.misses_at(cap as u64), naive_misses(&ops, cap));
        }
    }

    /// Restarting counts keeps the stack: what was warm stays warm.
    #[test]
    fn restart_keeps_the_stack_and_sets_the_exact_range() {
        let mut p = StackProfile::new();
        for id in 0..10u64 {
            p.access(id, 512);
        }
        p.restart_counts();
        assert_eq!(p.exact_upto, 10);
        p.access(9, 512); // most recent: distance 0
        p.access(0, 512); // oldest: 9 others touched since
        let c = p.curve(1, 0, u64::MAX);
        assert_eq!(c.cold, 0);
        assert_eq!(c.misses_at(1), 1);
        assert_eq!(c.misses_at(9), 1);
        assert_eq!(c.misses_at(10), 0);
    }

    #[test]
    fn budget_arithmetic() {
        let mut p = StackProfile::new();
        p.access(1, 512);
        let c = p.curve(100, 1000, 50);
        assert_eq!(c.capacity_for(999), None);
        assert_eq!(c.capacity_for(1000), Some(0));
        assert_eq!(c.capacity_for(1250), Some(2));
        assert_eq!(c.capacity_for(1_000_000), Some(50));
        assert_eq!(c.full_dram_bytes(), 6000);
    }

    /// Above the exact range a cold access is an unknown, not a miss: the
    /// lower bound drops it, the upper bound keeps it.
    #[test]
    fn cold_accesses_bound_the_curve_above_the_exact_range() {
        let mut p = StackProfile::new();
        for id in 0..4u64 {
            p.place(id);
        }
        p.restart_counts(); // exact up to 4 entries
        p.access(100, 512); // never seen: cold
        p.access(0, 512); // warm, 4 others above it (1, 2, 3, 100)
        let c = p.curve(1, 0, 1000);
        assert_eq!(c.cold, 1);
        assert_eq!(c.bounds_at(4), (2, 2)); // inside the exact range
        assert_eq!(c.bounds_at(5), (0, 1)); // above it: the cold one is unknown
    }

    /// A cold access is a certain miss for every capacity up to the stack
    /// size it happened at, not only up to the size at the restart.
    #[test]
    fn a_cold_access_stays_a_miss_until_capacity_passes_the_stack_it_met() {
        let mut p = StackProfile::new();
        for id in 0..4u64 {
            p.place(id);
        }
        p.restart_counts();
        p.access(100, 512); // cold, met a stack of 4
        for id in 4..10u64 {
            p.place(id);
        }
        p.access(200, 512); // cold, met a stack of 11
        let c = p.curve(1, 0, 1000);
        assert_eq!(c.bounds_at(4), (2, 2));
        assert_eq!(c.bounds_at(5), (1, 2)); // the second is still certain
        assert_eq!(c.bounds_at(11), (1, 2));
        assert_eq!(c.bounds_at(12), (0, 2));
    }

    #[test]
    fn index_assumption_can_be_varied_after_the_run() {
        let mut p = StackProfile::new();
        p.access(1, 512);
        // fixed 1000 = 500 resident + 50 index entries * 10 B.
        let c = p.curve_indexed(100, 1000, 50, 50, 10.0);
        let wider = c.with_index_entry_bytes(20.0);
        assert_eq!(wider.fixed_dram_bytes, 1500);
        assert_eq!(wider.capacity_for(1499), None);
        assert_eq!(c.with_entry_bytes(200).capacity_for(1400), Some(2));
    }
}
