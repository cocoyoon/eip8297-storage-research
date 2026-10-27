//! A copy-on-write B+tree of node records keyed by tree position: the way
//! a client stores trie nodes in MDBX today (reth's `AccountsTrie` and
//! `StoragesTrie` tables map a nibble path to a branch node), applied to
//! PBT's records. MODELED, not run: no B+tree is built. The full state's
//! shape says which 4 KB page each record sits in, and a commit is charged
//! the pages it would read and copy.
//!
//! The model:
//! - Entries are every node record of the full state, branches and
//!   leaves, in key order. A position sorts before everything under it, so
//!   the order is a pre-order walk of the tree. A record's pre-order rank
//!   comes from the shape (`rank_along`), its leaf page is `rank / E`, and
//!   the interior page above it at level j is `leaf_page / F^j`.
//! - The page map is static: inserts and deletes during the replay neither
//!   split nor merge pages, and the fill stays at `FILL`.
//! - A commit descends to every leaf page it dirties, reading each page on
//!   the way once per block (a cache miss is one 4 KB read), then writes
//!   every dirtied leaf and interior page to a fresh location (copy on
//!   write), the freelist records of the pages it frees, and the meta
//!   page, and syncs twice: data, then meta (MDBX_DURABLE).
//! - One LRU over leaf and interior pages together stands for the OS page
//!   cache a memory-mapped engine runs on. A second curve over leaf pages
//!   alone is kept for a reader that wants data blocks only.
//! - Residency follows the page store's rule: a page that ever holds a
//!   record above the resident depth is held in DRAM for good, written but
//!   never read.
//!
//! Every constant below is ASSUMED and stated as such in the report.

use crate::mrc::{id64, CacheProfile, StackProfile};
use crate::paged::PAGE_CACHE_OVERHEAD;
use crate::shape::{SharedShape, StateShape};
use crate::{Backend, Metrics, PAGE};
use pbt_ref::bits::bit;
use pbt_ref::tree::{CommitSink, ReadPath, RecordKind, RecordOp};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

/// Bytes of a 4 KB page left for entries after the page header.
pub const PAGE_BODY: u64 = 4076;
/// Mean fill of a leaf page under random inserts and in-place updates.
pub const FILL: f64 = 0.8;
/// Mean bytes of one entry. Key: 2 B length plus packed position bits,
/// 6 B for an account-zone branch and about 38 B for a branch inside a
/// contract (start bit 264 or more); 34 or 66 B for a leaf. Value: 64 B of
/// child hashes for a branch, 32 B for a leaf. MDBX adds an 8 B node
/// header and a 2 B slot pointer. Weighted by zone share (accounts 27%,
/// storage 57%, code 16%) that is about 96 B. reth's nibble-string keys
/// are longer than packed bits, which would put this nearer 130 B.
pub const MEAN_ENTRY_BYTES: u64 = 96;
/// Entries per leaf page: `PAGE_BODY * FILL / MEAN_ENTRY_BYTES`.
pub const LEAF_ENTRIES: u64 = 33;
/// Children per interior page: a separator is a full key (about 50 B on
/// average) plus a page number and node header, at the same fill.
pub const FANOUT: u64 = 60;
/// Freed page numbers one freelist (GC) page records.
pub const GC_PER_PAGE: u64 = 1000;
/// Syncs per commit: the data pages, then the meta page.
pub const FSYNCS_PER_COMMIT: u64 = 2;
/// Leaf ranks remembered across blocks for execution reads; the memo is
/// cleared when it grows past this.
const READ_MEMO_CAP: usize = 1 << 22;

/// Pages per level of the tree over the full state: `levels[0]` leaf
/// pages, each level above `FANOUT` times smaller, down to one root page.
pub struct Geometry {
    pub levels: Vec<u64>,
}

impl Geometry {
    pub fn for_records(records: u64) -> Geometry {
        let mut levels = vec![records.div_ceil(LEAF_ENTRIES).max(1)];
        while *levels.last().unwrap() > 1 {
            let n = levels.last().unwrap().div_ceil(FANOUT);
            levels.push(n);
        }
        Geometry { levels }
    }

    pub fn height(&self) -> usize {
        self.levels.len()
    }

    pub fn total_pages(&self) -> u64 {
        self.levels.iter().sum()
    }

    /// The interior pages above a leaf page, root first: (level, page).
    pub fn ancestors(&self, leaf_page: u64) -> Vec<(u32, u64)> {
        (1..self.levels.len())
            .rev()
            .map(|j| (j as u32, leaf_page / FANOUT.pow(j as u32)))
            .collect()
    }
}

/// Pre-order rank of the node entered after each prefix of a path, in the
/// full tree. `counts[i]` is the number of leaves under the first `i` bits
/// of the path (the shape's `keys_along`), `bit_at(i)` the path's bit `i`.
///
/// At a level where keys sit on the other side (`off > 0`) the full tree
/// has a branch, which precedes the node (+1); taking the 1-side also
/// passes the whole 0-side subtree, `off` leaves and `off - 1` branches.
/// Bits inside a compressed prefix (`off == 0`) add nothing, so no
/// knowledge of where the branches are is needed. Below the point where
/// the path leaves the modeled keys (`on == 0`), the branch the path's own
/// key makes against the keys it left is still counted: a written key is
/// in the tree whether or not the shape models it.
pub fn rank_along(counts: &[u64], bit_at: impl Fn(usize) -> u8) -> Vec<u64> {
    let n = counts.len().saturating_sub(1);
    let mut out = Vec::with_capacity(n + 1);
    let mut r = 0u64;
    out.push(0);
    for i in 0..n {
        let on = counts[i + 1];
        let off = counts[i].saturating_sub(on);
        if off > 0 {
            r += 1;
            if bit_at(i) == 1 {
                r += 2 * off - 1;
            }
        }
        out.push(r);
    }
    out
}

/// A whole key as a bit path in `encode_bit_prefix` form.
pub fn key_path(key: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(2 + key.len());
    p.extend_from_slice(&((key.len() * 8) as u16).to_be_bytes());
    p.extend_from_slice(key);
    p
}

fn page_cache_id(level: u32, page: u64) -> u64 {
    let mut b = [0u8; 16];
    b[..4].copy_from_slice(b"btpg");
    b[4..8].copy_from_slice(&level.to_le_bytes());
    b[8..].copy_from_slice(&page.to_le_bytes());
    id64(&b)
}

/// Compare the first `nbits` bits of `key` with `packed`, the bits of a
/// position in `encode_bit_prefix` form without its length. A key shorter
/// than the prefix sorts first.
fn prefix_cmp(key: &[u8], packed: &[u8], nbits: usize) -> Ordering {
    let full = nbits / 8;
    if key.len() < full {
        return match key.cmp(&packed[..key.len()]) {
            Ordering::Equal => Ordering::Less,
            o => o,
        };
    }
    match key[..full].cmp(&packed[..full]) {
        Ordering::Equal => {}
        o => return o,
    }
    if nbits % 8 == 0 {
        return Ordering::Equal;
    }
    if key.len() == full {
        return Ordering::Less;
    }
    let mask = 0xFFu8 << (8 - nbits % 8);
    (key[full] & mask).cmp(&packed[full])
}

/// Prefix ranks along the paths of the leaves one block changes. Every
/// branch a block writes sits on the path of one of those leaves, except a
/// branch demoted by a split, which is looked up on its own.
struct BlockRanks {
    keys: Vec<Box<[u8]>>,
    ranks: Vec<Vec<u64>>,
}

impl BlockRanks {
    fn build(shape: &dyn StateShape, keys: &BTreeSet<&[u8]>) -> BlockRanks {
        let mut ks = Vec::with_capacity(keys.len());
        let mut rs = Vec::with_capacity(keys.len());
        for k in keys {
            let counts = shape.keys_along(&key_path(k));
            rs.push(rank_along(&counts, |i| bit(k, i)));
            ks.push(Box::from(*k));
        }
        BlockRanks {
            keys: ks,
            ranks: rs,
        }
    }

    fn len(&self) -> usize {
        self.keys.len()
    }

    /// The rank of a leaf whose key the block changed.
    fn rank_of_key(&self, key: &[u8]) -> Option<u64> {
        let i = self.keys.binary_search_by(|k| k.as_ref().cmp(key)).ok()?;
        self.ranks[i].last().copied()
    }

    /// The rank of the node entered at a position, if one of the block's
    /// leaves lies under it.
    fn rank_of_prefix(&self, pos: &[u8]) -> Option<u64> {
        let nbits = u16::from_be_bytes([pos[0], pos[1]]) as usize;
        let packed = &pos[2..];
        let i = self
            .keys
            .partition_point(|k| prefix_cmp(k, packed, nbits) == Ordering::Less);
        let k = self.keys.get(i)?;
        if prefix_cmp(k, packed, nbits) != Ordering::Equal {
            return None;
        }
        self.ranks[i].get(nbits).copied()
    }
}

pub struct BtreeBackend {
    shape: SharedShape,
    geometry: Geometry,
    resident_levels: u32,
    /// Records above this node depth are in the resident grant.
    resident_depth: u32,
    m: Metrics,
    /// Leaf pages that ever held a resident record: in DRAM for good.
    resident_pages: HashSet<u64>,
    resident_page_bytes: u64,
    /// One cache over leaf and interior pages, and one over leaf pages alone.
    profile_tree: Option<StackProfile>,
    profile_leaf: Option<StackProfile>,
    /// Leaf rank by key hash, for execution reads.
    read_memo: HashMap<u64, u64>,
    creates: u64,
    rewrites: u64,
    deletes: u64,
    leaf_changes: u64,
    // Measured window.
    leaf_page_writes: u64,
    interior_page_writes: u64,
    gc_pages: u64,
    walks: u64,
    prefix_hits: u64,
    fallbacks: u64,
    memo_hits: u64,
}

impl BtreeBackend {
    /// `records` is the number of node records in the full state, which
    /// sets the page count and height; the shape places each record.
    pub fn new(shape: SharedShape, records: u64, resident_levels: u32) -> Self {
        BtreeBackend {
            shape,
            geometry: Geometry::for_records(records),
            resident_levels,
            resident_depth: resident_levels * crate::paged::PAGE_DEPTH,
            m: Metrics::default(),
            resident_pages: HashSet::new(),
            resident_page_bytes: 0,
            profile_tree: None,
            profile_leaf: None,
            read_memo: HashMap::new(),
            creates: 0,
            rewrites: 0,
            deletes: 0,
            leaf_changes: 0,
            leaf_page_writes: 0,
            interior_page_writes: 0,
            gc_pages: 0,
            walks: 0,
            prefix_hits: 0,
            fallbacks: 0,
            memo_hits: 0,
        }
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn walk_pos(&self, pos: &[u8]) -> u64 {
        let counts = self.shape.keys_along(pos);
        *rank_along(&counts, |i| bit(&pos[2..], i)).last().unwrap()
    }

    fn walk_key(&self, key: &[u8]) -> u64 {
        let counts = self.shape.keys_along(&key_path(key));
        *rank_along(&counts, |i| bit(key, i)).last().unwrap()
    }

    fn remember(&mut self, key_id: u64, rank: u64) {
        if self.read_memo.len() >= READ_MEMO_CAP {
            self.read_memo.clear();
        }
        self.read_memo.insert(key_id, rank);
    }

    fn rank_of(
        &mut self,
        ranks: &BlockRanks,
        is_leaf: bool,
        leaf_key: Option<&[u8]>,
        pos: &[u8],
    ) -> u64 {
        if is_leaf {
            if let Some(r) = leaf_key.and_then(|k| ranks.rank_of_key(k)) {
                return r;
            }
        }
        if let Some(r) = ranks.rank_of_prefix(pos) {
            self.prefix_hits += 1;
            return r;
        }
        self.fallbacks += 1;
        self.walks += 1;
        self.walk_pos(pos)
    }

    /// One page read on a descent: counted, and fed to the curves.
    fn touch(&mut self, level: u32, page: u64, tagged: bool) {
        let h = page_cache_id(level, page);
        let feed = |p: &mut StackProfile| {
            if tagged {
                p.access_tagged(h, PAGE);
            } else {
                p.access(h, PAGE);
            }
        };
        if let Some(p) = self.profile_tree.as_mut() {
            feed(p);
        }
        if level == 0 {
            if let Some(p) = self.profile_leaf.as_mut() {
                feed(p);
            }
        }
        self.m.page_reads += 1;
        self.m.read_bytes += PAGE;
    }

    /// Descend to a leaf page: its ancestors root first, then the page
    /// itself unless resident. `seen` keeps a block from reading an
    /// interior page twice.
    fn descend(&mut self, page: u64, seen: &mut BTreeSet<(u32, u64)>, tagged: bool) {
        for (level, id) in self.geometry.ancestors(page) {
            if seen.insert((level, id)) {
                self.touch(level, id, tagged);
            }
        }
        if !self.resident_pages.contains(&page) {
            self.touch(0, page, tagged);
        }
    }

    fn apply_pages(&mut self, batch: &CommitSink) {
        let mut keys: BTreeSet<&[u8]> = BTreeSet::new();
        for r in &batch.records {
            if let Some(k) = &r.leaf_key {
                keys.insert(k);
            }
        }
        for d in &batch.deletes {
            if let Some(k) = &d.leaf_key {
                keys.insert(k);
            }
        }
        let ranks = BlockRanks::build(&*self.shape, &keys);
        self.walks += ranks.len() as u64;
        for (k, r) in ranks.keys.iter().zip(&ranks.ranks) {
            let rank = *r.last().unwrap();
            self.remember(id64(k), rank);
        }
        let mut dirty: BTreeSet<u64> = BTreeSet::new();
        for r in &batch.records {
            let rank = self.rank_of(
                &ranks,
                r.kind == RecordKind::Leaf,
                r.leaf_key.as_deref(),
                &r.pos,
            );
            let page = rank / LEAF_ENTRIES;
            if r.node_depth < self.resident_depth && self.resident_pages.insert(page) {
                self.resident_page_bytes += PAGE;
            }
            dirty.insert(page);
        }
        for d in &batch.deletes {
            let rank = self.rank_of(&ranks, d.was_leaf, d.leaf_key.as_deref(), &d.pos);
            let page = rank / LEAF_ENTRIES;
            if d.node_depth < self.resident_depth && self.resident_pages.insert(page) {
                self.resident_page_bytes += PAGE;
            }
            dirty.insert(page);
        }
        // The order a cursor over a sorted batch of puts touches pages in.
        let mut seen: BTreeSet<(u32, u64)> = BTreeSet::new();
        for &page in &dirty {
            self.descend(page, &mut seen, false);
        }
        // Every dirtied page is copied to a fresh location, the pages it
        // frees go into the freelist, and the meta page is rewritten.
        let freed = dirty.len() as u64 + seen.len() as u64;
        let gc = freed.div_ceil(GC_PER_PAGE);
        self.m.rand_page_writes += freed + gc + 1;
        self.leaf_page_writes += dirty.len() as u64;
        self.interior_page_writes += seen.len() as u64;
        self.gc_pages += gc;
    }

    fn curve_of(&self, p: &StackProfile, cacheable: u64) -> CacheProfile {
        let mut c = p.curve(
            PAGE + PAGE_CACHE_OVERHEAD,
            self.resident_page_bytes,
            cacheable,
        );
        c.btree_leaf_page_writes = self.leaf_page_writes;
        c.btree_interior_page_writes = self.interior_page_writes;
        c
    }
}

impl Backend for BtreeBackend {
    fn name(&self) -> String {
        format!("btree-cow(res={})", self.resident_levels)
    }

    fn commit_block(&mut self, batch: &CommitSink) {
        let t0 = Instant::now();
        for r in &batch.records {
            self.m.record_writes += 1;
            match r.op {
                RecordOp::Create => self.creates += 1,
                RecordOp::Rewrite => self.rewrites += 1,
            }
            if r.kind == RecordKind::Leaf {
                self.leaf_changes += 1;
            }
        }
        for d in &batch.deletes {
            self.m.record_deletes += 1;
            self.deletes += 1;
            if d.was_leaf {
                self.leaf_changes += 1;
            }
        }
        // Genesis loads the tree in bulk; its page map is not costed.
        if self.profile_tree.is_some() {
            self.apply_pages(batch);
        }
        self.m.fsyncs += FSYNCS_PER_COMMIT;
        self.m.wall_ns += t0.elapsed().as_nanos() as u64;
    }

    fn serve_read(&mut self, rp: &ReadPath) {
        self.m.reads_served += 1;
        let mut pages: BTreeSet<u64> = BTreeSet::new();
        for (_, pos, _, _) in &rp.branches {
            pages.insert(self.walk_pos(pos) / LEAF_ENTRIES);
        }
        if let Some((k, _)) = &rp.leaf {
            pages.insert(self.walk_key(k) / LEAF_ENTRIES);
        }
        let mut seen = BTreeSet::new();
        for page in pages {
            self.descend(page, &mut seen, false);
        }
    }

    fn serve_value_read(&mut self, rp: &ReadPath) {
        self.m.reads_served += 1;
        // A point lookup descends to the leaf page whose range covers the
        // key, present or not.
        let page = match (&rp.leaf, rp.branches.last()) {
            (Some((k, _)), _) => self.walk_key(k) / LEAF_ENTRIES,
            (None, Some((_, pos, _, _))) => self.walk_pos(pos) / LEAF_ENTRIES,
            (None, None) => return,
        };
        let mut seen = BTreeSet::new();
        self.descend(page, &mut seen, false);
    }

    fn serve_state_read(&mut self, key: &[u8], _present: bool) {
        self.m.reads_served += 1;
        if self.profile_tree.is_none() {
            return;
        }
        let kid = id64(key);
        let rank = match self.read_memo.get(&kid) {
            Some(&r) => {
                self.memo_hits += 1;
                r
            }
            None => {
                self.walks += 1;
                let r = self.walk_key(key);
                self.remember(kid, r);
                r
            }
        };
        // Absent or present, the lookup reads the page the key would be in;
        // a B+tree has no filter to answer an absent key without it.
        let mut seen = BTreeSet::new();
        self.descend(rank / LEAF_ENTRIES, &mut seen, true);
    }

    fn metrics(&self) -> Metrics {
        let mut m = self.m.clone();
        m.live_bytes = self.geometry.total_pages() * PAGE;
        m.disk_bytes = m.live_bytes;
        m.dram_bytes = self.resident_page_bytes;
        m
    }

    fn finalize(&mut self) {}

    fn enable_cache_profile(&mut self) {
        self.profile_tree = Some(StackProfile::new());
        self.profile_leaf = Some(StackProfile::new());
    }

    fn begin_measurement(&mut self) {
        for p in [self.profile_tree.as_mut(), self.profile_leaf.as_mut()]
            .into_iter()
            .flatten()
        {
            p.restart_counts();
        }
        self.leaf_page_writes = 0;
        self.interior_page_writes = 0;
        self.gc_pages = 0;
        self.walks = 0;
        self.prefix_hits = 0;
        self.fallbacks = 0;
        self.memo_hits = 0;
    }

    /// The copy-on-write pages are read before they are copied, which is
    /// also the read of the value a transaction is about to change.
    fn charge_old_value_reads(&mut self) {}

    fn cache_profile(&self) -> Option<CacheProfile> {
        let p = self.profile_tree.as_ref()?;
        Some(self.curve_of(p, self.geometry.total_pages()))
    }

    /// The leaf-page curve alone, under the page store's slot for a second
    /// curve: this backend has no value store.
    fn value_store_profile(&self) -> Option<CacheProfile> {
        let p = self.profile_leaf.as_ref()?;
        Some(self.curve_of(p, self.geometry.levels[0]))
    }

    fn extra_report(&self) -> String {
        let g = &self.geometry;
        let mut s = String::new();
        s.push_str(&format!(
            "- btree-cow MODELED: {} entries per 4 KB leaf page ({} B mean entry at fill {}), fanout {}, height {}, pages per level {:?} ({:.1} GB)\n",
            LEAF_ENTRIES,
            MEAN_ENTRY_BYTES,
            FILL,
            FANOUT,
            g.height(),
            g.levels,
            g.total_pages() as f64 * PAGE as f64 / 1e9
        ));
        s.push_str(&format!(
            "- btree-cow: resident leaf pages {} ({:.1} MB); records created {}, rewritten {}, deleted {}, leaf changes {}\n",
            self.resident_pages.len(),
            self.resident_page_bytes as f64 / 1e6,
            self.creates,
            self.rewrites,
            self.deletes,
            self.leaf_changes
        ));
        s.push_str(&format!(
            "- btree-cow, measured window: leaf pages copied {}, interior pages copied {}, freelist pages {}; key walks {}, prefix hits {}, fallback walks {}, memo hits {}\n",
            self.leaf_page_writes,
            self.interior_page_writes,
            self.gc_pages,
            self.walks,
            self.prefix_hits,
            self.fallbacks,
            self.memo_hits
        ));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbt_ref::bits::encode_bit_prefix;
    use pbt_ref::keys::{
        get_tree_key_for_basic_data, get_tree_key_for_storage_slot, slot_from_u64,
    };
    use pbt_ref::tree::{CommitSink, PbtTree, RewriteStats};
    use std::sync::Arc;

    /// Every prefix of length at most `depth` holds a complete subtree.
    struct CompleteShape {
        depth: usize,
    }

    impl StateShape for CompleteShape {
        fn keys_under(&self, path: &[u8]) -> u64 {
            let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
            if nbits > self.depth {
                0
            } else {
                1 << (self.depth - nbits)
            }
        }
        fn value_leaf_page(&self, _key: &[u8]) -> u64 {
            0
        }
        fn value_bbn(&self, _key: &[u8]) -> u64 {
            0
        }
    }

    /// Exactly the given keys.
    struct ExplicitShape {
        keys: Vec<Vec<u8>>,
    }

    impl StateShape for ExplicitShape {
        fn keys_under(&self, path: &[u8]) -> u64 {
            let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
            self.keys
                .iter()
                .filter(|k| {
                    k.len() * 8 >= nbits && (0..nbits).all(|i| bit(k, i) == bit(&path[2..], i))
                })
                .count() as u64
        }
        fn value_leaf_page(&self, _key: &[u8]) -> u64 {
            0
        }
        fn value_bbn(&self, _key: &[u8]) -> u64 {
            0
        }
    }

    fn bits_of(path: &[u8]) -> Vec<u8> {
        let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
        (0..nbits).map(|i| bit(&path[2..], i)).collect()
    }

    fn some_keys(n: u64) -> Vec<Vec<u8>> {
        let mut keys = Vec::new();
        for i in 0..n {
            let mut addr = [0u8; 32];
            addr[24..].copy_from_slice(&i.to_be_bytes());
            keys.push(get_tree_key_for_basic_data(&addr));
            if i % 2 == 0 {
                keys.push(get_tree_key_for_storage_slot(&addr, &slot_from_u64(i)));
                keys.push(get_tree_key_for_storage_slot(
                    &addr,
                    &slot_from_u64(i + 1000),
                ));
            }
        }
        keys
    }

    fn committed(keys: &[Vec<u8>]) -> (PbtTree, CommitSink) {
        let mut tree = PbtTree::new();
        for (i, k) in keys.iter().enumerate() {
            let mut v = [0u8; 32];
            v[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
            tree.write(k, &v);
        }
        let mut sink = CommitSink::default();
        tree.commit(&mut RewriteStats::default(), Some(&mut sink));
        (tree, sink)
    }

    #[test]
    fn leaf_entries_follow_the_constants() {
        assert_eq!(
            LEAF_ENTRIES,
            (PAGE_BODY as f64 * FILL / MEAN_ENTRY_BYTES as f64) as u64
        );
    }

    #[test]
    fn complete_tree_ranks_are_preorder_indices() {
        let depth = 6;
        let shape = CompleteShape { depth };
        // Pre-order index of every node of a complete binary tree of that
        // depth, by walking it.
        fn walk(prefix: &mut Vec<u8>, depth: usize, next: &mut u64, out: &mut Vec<(Vec<u8>, u64)>) {
            out.push((prefix.clone(), *next));
            *next += 1;
            if prefix.len() == depth {
                return;
            }
            for b in [0u8, 1] {
                prefix.push(b);
                walk(prefix, depth, next, out);
                prefix.pop();
            }
        }
        let mut expect = Vec::new();
        walk(&mut Vec::new(), depth, &mut 0, &mut expect);
        for (bits, want) in expect {
            let path = encode_bit_prefix(&bits);
            let counts = shape.keys_along(&path);
            let ranks = rank_along(&counts, |i| bits[i]);
            assert_eq!(ranks[bits.len()], want, "prefix {bits:?}");
            // Ranks never decrease along a path.
            assert!(ranks.windows(2).all(|w| w[0] <= w[1]));
        }
    }

    #[test]
    fn real_tree_ranks_are_preorder_and_agree_between_key_and_pos() {
        let keys = some_keys(40);
        let shape = ExplicitShape { keys: keys.clone() };
        let (_, sink) = committed(&keys);
        assert_eq!(sink.records.len(), 2 * keys.len() - 1, "a full binary tree");
        // Pre-order = records sorted by their bit path, a prefix first.
        let mut recs: Vec<(Vec<u8>, u64, u64)> = sink
            .records
            .iter()
            .map(|r| {
                let pos_rank = *rank_along(&shape.keys_along(&r.pos), |i| bit(&r.pos[2..], i))
                    .last()
                    .unwrap();
                let (bits, rank) = match &r.leaf_key {
                    Some(k) => {
                        let key_rank = *rank_along(&shape.keys_along(&key_path(k)), |i| bit(k, i))
                            .last()
                            .unwrap();
                        assert_eq!(
                            key_rank, pos_rank,
                            "leaf rank by key equals rank by position"
                        );
                        (bits_of(&key_path(k)), key_rank)
                    }
                    None => (bits_of(&r.pos), pos_rank),
                };
                (bits, rank, r.node_depth as u64)
            })
            .collect();
        recs.sort();
        for (i, (bits, rank, _)) in recs.iter().enumerate() {
            assert_eq!(*rank, i as u64, "record {i} at {bits:?}");
        }
        // A branch and its leftmost descendant differ by their depth
        // difference, so they share a page when that is small.
        for w in recs.windows(2) {
            if w[1].0.starts_with(&w[0].0) {
                assert_eq!(w[1].1 - w[0].1, w[1].2 - w[0].2);
            }
        }
    }

    #[test]
    fn geometry_chain() {
        let g = Geometry::for_records(5_000_000_000);
        assert_eq!(g.levels[0], 5_000_000_000u64.div_ceil(LEAF_ENTRIES));
        assert_eq!(*g.levels.last().unwrap(), 1);
        for w in g.levels.windows(2) {
            assert_eq!(w[1], w[0].div_ceil(FANOUT));
        }
        let anc = g.ancestors(123_456_789);
        assert_eq!(anc.len(), g.height() - 1);
        assert_eq!(anc[0], ((g.height() - 1) as u32, 0));
        assert_eq!(anc.last().unwrap(), &(1, 123_456_789 / FANOUT));
    }

    #[test]
    fn commit_counts_pages_reads_and_syncs() {
        let keys = some_keys(40);
        let shape: SharedShape = Arc::new(ExplicitShape { keys: keys.clone() });
        let (mut tree, genesis) = committed(&keys);
        let mut b = BtreeBackend::new(shape, 2 * keys.len() as u64, 0);
        b.commit_block(&genesis);
        b.enable_cache_profile();
        b.begin_measurement();
        // Rewrite four leaves far apart.
        let mut sink = CommitSink::default();
        for i in [0usize, 10, 20, 30] {
            tree.write(&keys[i], &[0xAB; 32]);
        }
        tree.commit(&mut RewriteStats::default(), Some(&mut sink));
        let before = b.metrics();
        b.commit_block(&sink);
        let m = b.metrics();
        let tree_curve = b.cache_profile().unwrap();
        let leaf_curve = b.value_store_profile().unwrap();
        assert_eq!(m.fsyncs - before.fsyncs, FSYNCS_PER_COMMIT);
        assert_eq!(m.page_reads - before.page_reads, tree_curve.accesses);
        assert_eq!(m.read_bytes - before.read_bytes, tree_curve.accesses * PAGE);
        assert_eq!(leaf_curve.accesses, b.leaf_page_writes);
        assert!(b.leaf_page_writes >= 1 && b.interior_page_writes >= 1);
        let freed = b.leaf_page_writes + b.interior_page_writes;
        assert_eq!(freed, tree_curve.accesses);
        assert_eq!(
            m.rand_page_writes - before.rand_page_writes,
            freed + freed.div_ceil(GC_PER_PAGE) + 1
        );
        assert_eq!(b.fallbacks, 0, "every branch sits above a changed leaf");
        assert_eq!(tree_curve.btree_leaf_page_writes, b.leaf_page_writes);
        assert_eq!(
            tree_curve.btree_interior_page_writes,
            b.interior_page_writes
        );
    }

    #[test]
    fn resident_pages_are_written_but_never_read() {
        let keys = some_keys(8);
        let shape: SharedShape = Arc::new(ExplicitShape { keys: keys.clone() });
        let (mut tree, genesis) = committed(&keys);
        // Every record of this small tree is above depth 12.
        let mut b = BtreeBackend::new(shape, 2 * keys.len() as u64, 2);
        b.commit_block(&genesis);
        b.enable_cache_profile();
        b.begin_measurement();
        let mut sink = CommitSink::default();
        tree.write(&keys[3], &[0xCD; 32]);
        tree.commit(&mut RewriteStats::default(), Some(&mut sink));
        b.commit_block(&sink);
        let m = b.metrics();
        assert_eq!(b.cache_profile().unwrap().accesses, b.interior_page_writes);
        assert_eq!(b.value_store_profile().unwrap().accesses, 0);
        assert!(m.rand_page_writes > 0);
        assert_eq!(m.dram_bytes, b.resident_pages.len() as u64 * PAGE);
        assert!(m.dram_bytes > 0);
    }

    #[test]
    fn prefix_lookup_matches_a_direct_walk() {
        let keys = some_keys(40);
        let shape = ExplicitShape { keys: keys.clone() };
        let (_, sink) = committed(&keys);
        let set: BTreeSet<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
        let ranks = BlockRanks::build(&shape, &set);
        for r in &sink.records {
            let direct = *rank_along(&shape.keys_along(&r.pos), |i| bit(&r.pos[2..], i))
                .last()
                .unwrap();
            assert_eq!(ranks.rank_of_prefix(&r.pos), Some(direct));
        }
        // A position no changed leaf lies under is not found.
        let foreign = encode_bit_prefix(&[1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(ranks.rank_of_prefix(&foreign), None);
    }

    #[test]
    fn absent_and_present_state_reads_cost_the_same() {
        let keys = some_keys(40);
        let shape: SharedShape = Arc::new(ExplicitShape { keys: keys.clone() });
        let (_, genesis) = committed(&keys);
        let mut b = BtreeBackend::new(shape, 2 * keys.len() as u64, 0);
        b.commit_block(&genesis);
        b.enable_cache_profile();
        b.begin_measurement();
        let h = b.geometry.height() as u64;
        b.serve_state_read(&keys[5], true);
        let c1 = b.cache_profile().unwrap();
        assert_eq!(c1.accesses, h);
        assert_eq!(c1.tag_accesses, h);
        let mut absent = keys[5].clone();
        absent[20] ^= 0x55;
        b.serve_state_read(&absent, false);
        let c2 = b.cache_profile().unwrap();
        assert_eq!(c2.accesses, 2 * h);
        assert_eq!(c2.tag_accesses, 2 * h);
        assert_eq!(b.memo_hits, 0);
        b.serve_state_read(&keys[5], true);
        assert_eq!(b.memo_hits, 1);
    }
}
