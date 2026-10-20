//! MEASUREMENT CONTROL, not a layout candidate. The page-packed branch
//! tier was dropped from the committed layout in draft Appendix D; this
//! backend is retained because §8.1 requires a page-packed baseline to
//! measure the append discipline against, and because Appendix D's
//! reinstatement trigger must stay runnable.
//!
//! Page-packed backend, modeled on NOMT's bitbox as read from source
//! (thrumdev/nomt, 2026-08-25). Faithful to the mechanism, not the code:
//!
//! - 4KB pages, each holding a ROOTLESS depth-6 subtrie = 126 node slots
//!   (2+4+8+16+32+64), so a page covers 6 tree levels and has 64 children.
//! - A page's location is hash-addressed (`hash(page_id) % buckets`) with
//!   triangular probing over a preallocated table; an existing page keeps
//!   its bucket, so updates are IN PLACE, not copy-on-write.
//! - Crash consistency: per-commit WAL of node-granularity redo diffs
//!   (page_id 32B + diff bitmap 16B + changed nodes 32B each + elided 8B
//!   + bucket 8B), fsynced; then the manifest; then the 4KB pages in place.
//!   Three fsyncs per commit in the modeled path.
//! - Deletion sets a tombstone byte in the meta map: no page rewrite.

use crate::mrc::{id64, CacheProfile, StackProfile};
use crate::shape::SharedShape;
use crate::{Backend, Metrics, PAGE};
use pbt_ref::tree::{CommitSink, ReadPath, RecordKind, RecordOp};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::Instant;

pub const PAGE_DEPTH: u32 = 6;
/// Rootless depth-6 subtrie: 2+4+8+16+32+64 = 126 slots. A page holds
/// absolute depths [6L+1, 6L+6]; the subtree's own root at depth 6L lives
/// in the PARENT page, which is what makes the packing rootless.
pub const NODES_PER_PAGE: u32 = (1 << (PAGE_DEPTH + 1)) - 2; // 126

/// Page level owning a record at `depth`. Depth 0 (the tree root) has no
/// parent page and is modeled as living in level 0 alongside depths 1..6.
fn level_of(depth: u32) -> u32 {
    depth.saturating_sub(1) / PAGE_DEPTH
}
const NODE_BYTES: u64 = 32;
/// WAL per-page overhead: tag(4)+page_id(32)+diff(16)+elided(8)+bucket(8).
const WAL_PAGE_OVERHEAD: u64 = 68;

/// NOMT elides a page whose subtree holds fewer than this many leaves: the
/// page is not stored, the parent marks it in an 8-byte elided-children
/// bitfield, and the subtree is rebuilt from the leaf store when needed.
/// This is NOMT's own answer to page sparsity, so a fair comparison under
/// PBT must include it (merkle/mod.rs PAGE_ELISION_THRESHOLD = 20).
pub const PAGE_ELISION_THRESHOLD: u32 = 20;

/// Bookkeeping per cached page: the same two recency links the log store's
/// bounded branch cache is charged, so neither side gets its cache free.
const PAGE_CACHE_OVERHEAD: u64 = 16;

/// Entries in one value-store leaf page. NOMT's Beatree leaf has a
/// 4094-byte body and a PBT leaf entry is a 34-byte key plus a 32-byte
/// value, so 62 fit (nomt/src/beatree, read at c3c0e55).
pub const VALUE_LEAF_ENTRIES: usize = 62;

/// Leaf pages one bottom branch node (BBN) of the value store can hold.
/// ASSUMED from NOMT's node format (4086-byte body, about 8 bytes per entry
/// once separator keys are shortened); the true figure depends on how far
/// separators compress. Nodes split at the midpoint, so typical fan-out is
/// about 0.7 of this. Beatree keeps every such node in memory.
pub const VALUE_BBN_MAX: usize = 510;

/// The leaf layer of a B+tree value store and the bottom branch nodes above
/// it, modeled so that the pages a block dirties are MEASURED rather than
/// assumed.
///
/// This is the part of a NOMT-style store the merkle pages do not cover: a
/// page slot holds a 32-byte node hash, and the leaf's key and value live
/// in the value store. As read from NOMT's source (c3c0e55): every touched
/// leaf page is read if uncached and rewritten copy-on-write to a new page
/// number on every commit, with no log and no batching across commits, and
/// the bottom branch node above a rewritten leaf is rewritten too.
///
/// Simplifications, each a few percent on the page counts and in opposite
/// directions: no merge of underfull neighbours (a page is dropped only
/// when it empties); every split is at the midpoint, where NOMT bulk-splits
/// sorted input to three-quarters full.
pub struct ValueLeaves {
    /// first key of each page -> (stable page id, its keys in order). One
    /// page keyed by the empty key always exists and takes anything below
    /// every other first key.
    pages: BTreeMap<Vec<u8>, (u64, Vec<Box<[u8]>>)>,
    next_id: u64,
    /// first key of each bottom branch node -> (stable id, leaf pages under
    /// it). Ranges partition the key space the same way pages do.
    bbns: BTreeMap<Vec<u8>, (u64, usize)>,
    next_bbn: u64,
}

/// What one key change did to the value store.
pub struct LeafTouch {
    /// The page the key belongs to.
    pub page: u64,
    /// A page created by splitting it, if the insert overflowed.
    pub split_off: Option<u64>,
    /// The page emptied and was dropped.
    pub dropped: bool,
    /// Bottom branch nodes whose contents changed or that sit above a
    /// rewritten leaf: the key's own, and the split-off page's if different.
    pub bbns: [Option<u64>; 2],
}

impl Default for ValueLeaves {
    fn default() -> Self {
        Self::new()
    }
}

fn floor_key<V>(map: &BTreeMap<Vec<u8>, V>, key: &[u8]) -> Vec<u8> {
    map.range::<[u8], _>((std::ops::Bound::Unbounded, std::ops::Bound::Included(key)))
        .next_back()
        .map(|(k, _)| k.clone())
        .unwrap_or_default()
}

impl ValueLeaves {
    pub fn new() -> Self {
        let mut pages = BTreeMap::new();
        pages.insert(Vec::new(), (0u64, Vec::new()));
        let mut bbns = BTreeMap::new();
        bbns.insert(Vec::new(), (0u64, 1usize));
        ValueLeaves {
            pages,
            next_id: 1,
            bbns,
            next_bbn: 1,
        }
    }

    pub fn page_count(&self) -> u64 {
        self.pages.len() as u64
    }

    pub fn bbn_count(&self) -> u64 {
        self.bbns.len() as u64
    }

    fn bbn_id(&self, key: &[u8]) -> u64 {
        self.bbns[&floor_key(&self.bbns, key)].0
    }

    /// A leaf page keyed `first` came into being: count it under its node
    /// and split the node at the midpoint if it overflowed.
    fn bbn_add_page(&mut self, first: &[u8]) {
        let bfirst = floor_key(&self.bbns, first);
        let entry = self.bbns.get_mut(&bfirst).expect("node exists");
        entry.1 += 1;
        let count = entry.1;
        if count <= VALUE_BBN_MAX {
            return;
        }
        let keep = count / 2;
        let median = self
            .pages
            .range::<[u8], _>((
                std::ops::Bound::Included(bfirst.as_slice()),
                std::ops::Bound::Unbounded,
            ))
            .nth(keep)
            .map(|(k, _)| k.clone())
            .expect("node holds more pages than it keeps");
        self.bbns.get_mut(&bfirst).expect("node exists").1 = keep;
        let id = self.next_bbn;
        self.next_bbn += 1;
        self.bbns.insert(median, (id, count - keep));
    }

    /// Insert or overwrite `key`.
    pub fn upsert(&mut self, key: &[u8]) -> LeafTouch {
        let first = floor_key(&self.pages, key);
        let (id, keys) = self.pages.get_mut(&first).expect("page exists");
        let id = *id;
        if let Err(i) = keys.binary_search_by(|k| (**k).cmp(key)) {
            keys.insert(i, key.into());
        }
        let mut split_off = None;
        let mut split_first = None;
        if keys.len() > VALUE_LEAF_ENTRIES {
            let right = keys.split_off(keys.len() / 2);
            let new_id = self.next_id;
            self.next_id += 1;
            let right_first = right[0].to_vec();
            self.pages.insert(right_first.clone(), (new_id, right));
            self.bbn_add_page(&right_first);
            split_off = Some(new_id);
            split_first = Some(right_first);
        }
        let own = self.bbn_id(&first);
        let other = split_first.map(|k| self.bbn_id(&k)).filter(|b| *b != own);
        LeafTouch {
            page: id,
            split_off,
            dropped: false,
            bbns: [Some(own), other],
        }
    }

    /// Remove `key` if present; None if it was not there.
    pub fn remove(&mut self, key: &[u8]) -> Option<LeafTouch> {
        let first = floor_key(&self.pages, key);
        let (id, keys) = self.pages.get_mut(&first).expect("page exists");
        let id = *id;
        let i = keys.binary_search_by(|k| (**k).cmp(key)).ok()?;
        keys.remove(i);
        let dropped = keys.is_empty() && !first.is_empty();
        let bfirst = floor_key(&self.bbns, &first);
        let own = self.bbns[&bfirst].0;
        if dropped {
            self.pages.remove(&first);
            let entry = self.bbns.get_mut(&bfirst).expect("node exists");
            entry.1 = entry.1.saturating_sub(1);
            if entry.1 == 0 && !bfirst.is_empty() {
                self.bbns.remove(&bfirst);
            }
        }
        Some(LeafTouch {
            page: id,
            split_off: None,
            dropped,
            bbns: [Some(own), None],
        })
    }
}

fn value_leaf_id(page: u64) -> u64 {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(b"vleafpg\0");
    b[8..].copy_from_slice(&page.to_le_bytes());
    id64(&b)
}

/// How a record's page is chosen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Packing {
    /// NOMT's own scheme: a page covers BIT levels [6k, 6k+6). Faithful to
    /// bitbox, but PBT compresses shared bit-runs away, so most bit levels
    /// hold no node and pages come out sparse. This is RQ5's naive port.
    Positional,
    /// Adaptation: a page covers NODE-RECORD levels [6k, 6k+6) of the node
    /// tree, ignoring compressed prefix bits. Dense by construction, at the
    /// cost of losing the key-derived page address (a page's location can
    /// no longer be computed from the key alone without the node path).
    Structural,
}

/// Page identity from a packed bit path, truncated EXACTLY at the page
/// boundary. Byte-granular truncation would keep up to 7 extra bits and
/// split one page into as many as 128, so the trailing byte is masked.
fn page_id_from(path_pos: &[u8], level: u32) -> Vec<u8> {
    let page_start = (level * PAGE_DEPTH) as usize;
    let keep = (page_start + 7) / 8;
    let mut id = Vec::with_capacity(2 + keep);
    id.extend_from_slice(&(page_start as u16).to_le_bytes());
    if path_pos.len() > 2 && keep > 0 {
        let body = &path_pos[2..];
        let n = keep.min(body.len());
        id.extend_from_slice(&body[..n]);
        // Mask bits beyond page_start in the final byte.
        let rem = page_start % 8;
        if rem != 0 && id.len() > 2 {
            let last = id.len() - 1;
            let mask = 0xFFu8 << (8 - rem);
            id[last] &= mask;
        }
    }
    id
}

pub struct PagedCore {
    /// Live pages: id -> LIVE occupied node slots. Creations increment and
    /// deletions decrement; rewrites do not, so this is occupancy, not
    /// cumulative write traffic.
    pages: HashMap<Vec<u8>, u32>,
    pub m: Metrics,
    pub resident_levels: u32,
    pub wal_bytes: u64,
    pub occupancy_samples: Vec<u32>,
    pub packing: Packing,
    /// Elide sparse pages the way NOMT does.
    pub elision: bool,
    pub elided_writes_saved: u64,
    pub elided_read_fallbacks: u64,
    resident_pages: HashSet<Vec<u8>>,
    resident_page_bytes: u64,
    /// Bounded page-cache profile of the write-path page reads (O7): the
    /// same DRAM the log store spends on a branch cache, spent on pages.
    pub profile: Option<StackProfile>,
    /// DIAGNOSTIC (charges nothing): non-resident pages touched by a block
    /// while below the elision threshold. They are charged no read, no
    /// write and no WAL bytes, and the leaf records in them are persisted
    /// nowhere in this model, while `read_into` charges one leaf-store read
    /// to rebuild such a page on the read path.
    pub elided_touches: u64,
    /// DIAGNOSTIC (charges nothing): leaf records created, rewritten or
    /// deleted. A page slot holds the leaf's 32-byte node hash; its key and
    /// value live in a value store this model never writes.
    pub leaf_changes: u64,
    /// The value store's leaf layer, built from genesis when a cache sweep
    /// is requested. None otherwise, so ordinary runs pay nothing for it.
    pub value_leaves: Option<ValueLeaves>,
    /// One cache over merkle pages AND value-store leaf pages: what a
    /// NOMT-style store has to share a DRAM budget between.
    pub profile_vs: Option<StackProfile>,
    /// MEASURED: distinct value-store leaf pages dirtied, summed over blocks.
    pub value_leaf_page_writes: u64,
    /// MEASURED: distinct bottom branch nodes above them, summed over blocks.
    pub value_bbn_writes: u64,
    /// The untouched state around the keys this run writes, when the run
    /// replays a sliver of a much larger state. None for a self-contained
    /// workload, where every page is judged by what the run itself put in it.
    pub shape: Option<SharedShape>,
}

/// The bit path of a page's root, recovered from its id: `page_id_from`
/// stores the starting bit and the path bytes masked to exactly that length.
fn page_root_path(id: &[u8]) -> Vec<u8> {
    let bits = u16::from_le_bytes([id[0], id[1]]);
    let mut out = Vec::with_capacity(id.len());
    out.extend_from_slice(&bits.to_be_bytes());
    out.extend_from_slice(&id[2..]);
    out
}

/// A dirty page in one block: id -> (level, nodes touched, net live-slot
/// delta, min node depth).
type DirtyPage = (Vec<u8>, (u32, u32, i64, u32));

impl PagedCore {
    pub fn new(resident_levels: u32) -> Self {
        Self::with_packing(resident_levels, Packing::Positional)
    }

    pub fn with_packing(resident_levels: u32, packing: Packing) -> Self {
        PagedCore {
            pages: HashMap::new(),
            m: Metrics::default(),
            resident_levels,
            wal_bytes: 0,
            occupancy_samples: Vec::new(),
            packing,
            elision: true,
            elided_writes_saved: 0,
            elided_read_fallbacks: 0,
            resident_pages: HashSet::new(),
            resident_page_bytes: 0,
            profile: None,
            elided_touches: 0,
            leaf_changes: 0,
            value_leaves: None,
            profile_vs: None,
            value_leaf_page_writes: 0,
            value_bbn_writes: 0,
            shape: None,
        }
    }

    /// Bounded-cache curve for the page store WITH its value store: merkle
    /// pages and value-store leaf pages compete for one cache, and every
    /// bottom branch node of the value store is resident, as in Beatree.
    pub fn value_store_profile(&self) -> Option<CacheProfile> {
        let p = self.profile_vs.as_ref()?;
        let merkle = self.cache_profile()?;
        // Under a state shape the leaf layer is not modeled here: its size
        // is a property of the full state, which the report supplies.
        let (leaf_pages, bbns) = match (&self.value_leaves, &self.shape) {
            (Some(vl), _) => (vl.page_count(), vl.bbn_count()),
            (None, Some(_)) => (0, 0),
            (None, None) => return None,
        };
        let bbn_bytes = bbns * PAGE;
        let mut c = p.curve(
            PAGE + PAGE_CACHE_OVERHEAD,
            self.resident_page_bytes + bbn_bytes,
            merkle.cacheable_entries + leaf_pages,
        );
        c.uncharged_elided_touches = self.elided_touches;
        c.uncharged_leaf_changes = self.leaf_changes;
        c.value_leaf_page_writes = self.value_leaf_page_writes;
        c.value_leaf_pages = leaf_pages;
        c.value_bbn_writes = self.value_bbn_writes;
        c.value_bbns = bbns;
        Some(c)
    }

    /// Bounded page-cache curve for this store's write-path page reads.
    /// Only stored pages can be cached: an elided page has no bytes to
    /// hold, and resident pages are already in the grant.
    pub fn cache_profile(&self) -> Option<CacheProfile> {
        let p = self.profile.as_ref()?;
        let floor = if self.elision {
            PAGE_ELISION_THRESHOLD
        } else {
            1
        };
        let cacheable = self
            .pages
            .iter()
            .filter(|(id, occ)| **occ >= floor && !self.resident_pages.contains(*id))
            .count() as u64;
        let mut c = p.curve(
            PAGE + PAGE_CACHE_OVERHEAD,
            self.resident_page_bytes,
            cacheable,
        );
        c.uncharged_elided_touches = self.elided_touches;
        c.uncharged_leaf_changes = self.leaf_changes;
        Some(c)
    }

    fn is_elided(&self, id: &[u8]) -> bool {
        self.elision
            && self
                .pages
                .get(id)
                .map_or(true, |occ| *occ < PAGE_ELISION_THRESHOLD)
    }

    /// (page level, page id) for a record.
    fn page_of(
        &self,
        start_bit: u32,
        node_depth: u32,
        pos: &[u8],
        node_pos: &[u8],
    ) -> (u32, Vec<u8>) {
        match self.packing {
            Packing::Positional => {
                let level = level_of(start_bit);
                (level, page_id_from(pos, level))
            }
            Packing::Structural => {
                let level = level_of(node_depth);
                (level, page_id_from(node_pos, level))
            }
        }
    }

    /// Residency is expressed in NODE DEPTH, the same unit the append
    /// discipline uses, so both sides are granted the identical record set.
    /// Expressing it per page level made the grants incomparable: under
    /// positional packing a page level spans 6 BIT levels, which is a far
    /// smaller record set than 6 node depths.
    fn is_resident_depth(&self, node_depth: u32) -> bool {
        node_depth < self.resident_levels * PAGE_DEPTH
    }

    /// Apply the records selected by `take(node_depth)`; returns WAL bytes.
    pub fn apply(&mut self, batch: &CommitSink, take: impl Fn(u32) -> bool) -> u64 {
        // Group changed nodes by page.
        // id -> (level, nodes touched, net live-slot delta, min node depth)
        let mut dirty: HashMap<Vec<u8>, (u32, u32, i64, u32)> = HashMap::new();
        for r in &batch.records {
            if !take(r.node_depth) {
                continue;
            }
            let (level, id) = self.page_of(r.start_bit, r.node_depth, &r.pos, &r.node_pos);
            let e = dirty.entry(id).or_insert((level, 0, 0, r.node_depth));
            e.1 += 1;
            e.3 = e.3.min(r.node_depth);
            if r.op == RecordOp::Create {
                e.2 += 1; // a new slot becomes occupied
            }
            if r.kind == RecordKind::Leaf {
                self.leaf_changes += 1;
            }
            self.m.record_writes += 1;
        }
        // Deletions free a slot; the page still rewrites, and the meta map
        // records a tombstone.
        for d in &batch.deletes {
            if !take(d.node_depth) {
                continue;
            }
            let (level, id) = self.page_of(d.start_bit, d.node_depth, &d.pos, &d.node_pos);
            let e = dirty.entry(id).or_insert((level, 0, 0, d.node_depth));
            e.2 -= 1;
            e.3 = e.3.min(d.node_depth);
            if d.was_leaf {
                self.leaf_changes += 1;
            }
            self.m.record_deletes += 1;
        }

        let mut wal = 0u64;
        let mut fresh_pages = 0u64;
        let mut dirty: Vec<DirtyPage> = dirty.into_iter().collect();
        if self.profile.is_some() {
            // Stack distances depend on access order, and HashMap order is
            // not reproducible across runs.
            dirty.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }
        for (id, (level, nodes, live_delta, min_depth)) in dirty {
            let _ = level;
            // A page the full state fills is stored whether or not this run
            // has written to it. NOMT's rule, applied to the full state: a
            // page is stored when at least the threshold of leaves sit under it.
            let bg_stored = self.shape.as_ref().is_some_and(|s| {
                s.keys_under(&page_root_path(&id)) >= PAGE_ELISION_THRESHOLD as u64
            });
            let prev = self.pages.get(&id).copied();
            let known = prev.is_some() || bg_stored;
            if !known {
                fresh_pages += 1;
            }
            let occ = self.pages.entry(id.clone()).or_insert(0);
            let next = (*occ as i64 + live_delta).clamp(0, NODES_PER_PAGE as i64);
            *occ = next as u32;
            let resident = self.is_resident_depth(min_depth);
            if resident && self.resident_pages.insert(id.clone()) {
                self.resident_page_bytes += PAGE;
            }
            // Read-modify-write: a non-resident page that already exists must
            // be read before it can be rewritten (unless every slot changed).
            let stored_before = bg_stored || prev.is_some_and(|o| o >= PAGE_ELISION_THRESHOLD);
            let stored_after = bg_stored || next >= PAGE_ELISION_THRESHOLD as i64;
            let read_needed = if self.shape.is_some() {
                stored_before && stored_after && !resident && nodes < NODES_PER_PAGE
            } else {
                known && !resident && nodes < NODES_PER_PAGE && stored_after
            };
            if read_needed {
                self.m.page_reads += 1;
                self.m.read_bytes += PAGE;
            }
            let elided = self.elision && !stored_after;
            if elided && !resident {
                self.elided_touches += 1;
            }
            if !resident {
                // The same event goes to both curves: merkle pages alone,
                // and merkle pages sharing a cache with value-store leaves.
                for p in [self.profile.as_mut(), self.profile_vs.as_mut()]
                    .into_iter()
                    .flatten()
                {
                    let h = id64(&id);
                    if elided || (next == 0 && !bg_stored) {
                        p.remove(h); // nothing stored, nothing to hold
                    } else if read_needed {
                        p.access(h, PAGE);
                    } else {
                        p.place(h); // written in full: in hand
                    }
                }
            }
            if elided {
                // Not stored: the parent records it in its elided bitfield,
                // which is already part of the parent's own page write.
                self.elided_writes_saved += 1;
            } else {
                // In-place 4KB write at the page's own bucket.
                self.m.rand_page_writes += 1;
                wal += WAL_PAGE_OVERHEAD + nodes as u64 * NODE_BYTES;
            }
        }
        // Meta-map pages touched by fresh-bucket allocation and tombstones:
        // 4096 buckets per 4KB meta page, one write per touched meta page.
        if fresh_pages > 0 {
            self.m.rand_page_writes += fresh_pages.div_ceil(4096).max(1);
        }
        self.pages.retain(|_, occ| *occ > 0);
        self.apply_value_store(batch, &take);
        self.m.live_bytes = self.pages.len() as u64 * PAGE;
        self.wal_bytes += wal;
        self.m.seq_bytes_written += wal; // WAL is a sequential blob write
        wal
    }

    /// The value store's share of a commit: every changed leaf lands in a
    /// leaf page, which is read if uncached and rewritten copy-on-write.
    /// Charges nothing to `m`; it feeds the value-store curve only.
    fn apply_value_store(&mut self, batch: &CommitSink, take: &impl Fn(u32) -> bool) {
        if let Some(shape) = self.shape.clone() {
            // Leaf pages are a property of the full state: two keys share
            // one when the shape says so, and every page already exists.
            let Some(p) = self.profile_vs.as_mut() else {
                return;
            };
            let mut pages: std::collections::BTreeSet<u64> = Default::default();
            let mut bbns: HashSet<u64> = HashSet::new();
            let keys = batch
                .records
                .iter()
                .filter(|r| take(r.node_depth) && r.kind == RecordKind::Leaf)
                .map(|r| r.leaf_key.as_ref().expect("leaf has a key"))
                .chain(
                    batch
                        .deletes
                        .iter()
                        .filter(|d| take(d.node_depth) && d.was_leaf)
                        .map(|d| d.leaf_key.as_ref().expect("leaf has a key")),
                );
            for k in keys {
                pages.insert(shape.value_leaf_page(k));
                bbns.insert(shape.value_bbn(k));
            }
            for pg in &pages {
                p.access(value_leaf_id(*pg), PAGE);
            }
            self.value_leaf_page_writes += pages.len() as u64;
            self.value_bbn_writes += bbns.len() as u64;
            return;
        }
        let Some(vl) = self.value_leaves.as_mut() else {
            return;
        };
        // page id -> (fresh: created by a split in this block, dropped)
        let mut dirty: BTreeMap<u64, (bool, bool)> = BTreeMap::new();
        let mut dirty_bbns: HashSet<u64> = HashSet::new();
        for r in &batch.records {
            if !take(r.node_depth) || r.kind != RecordKind::Leaf {
                continue;
            }
            let t = vl.upsert(r.leaf_key.as_ref().expect("leaf has a key"));
            dirty.entry(t.page).or_insert((false, false));
            if let Some(n) = t.split_off {
                dirty.insert(n, (true, false));
            }
            dirty_bbns.extend(t.bbns.into_iter().flatten());
        }
        for d in &batch.deletes {
            if !take(d.node_depth) || !d.was_leaf {
                continue;
            }
            if let Some(t) = vl.remove(d.leaf_key.as_ref().expect("leaf has a key")) {
                let e = dirty.entry(t.page).or_insert((false, false));
                e.1 = t.dropped;
                dirty_bbns.extend(t.bbns.into_iter().flatten());
            }
        }
        if let Some(p) = self.profile_vs.as_mut() {
            // BTreeMap order: reproducible across runs.
            for (page, (fresh, dropped)) in &dirty {
                let h = value_leaf_id(*page);
                if *dropped {
                    p.remove(h);
                } else if *fresh {
                    p.place(h); // split off this block: in hand
                } else {
                    p.access(h, PAGE); // read-modify-write of an existing leaf
                }
            }
            self.value_leaf_page_writes +=
                dirty.values().filter(|(_, dropped)| !dropped).count() as u64;
            self.value_bbn_writes += dirty_bbns.len() as u64;
        }
    }

    /// A lookup walks pages, not nodes: one page read per 6 tree levels,
    /// skipping DRAM-resident levels. Probing may add a misprobe read.
    ///
    /// Served INDEPENDENTLY: the dedup set covers this key only, so a page
    /// the whole block shares is refetched for every key.
    pub fn read(&mut self, rp: &ReadPath, take: impl Fn(u32) -> bool) {
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        self.read_into(rp, &take, &mut seen);
    }

    /// One multiproof over a block's whole read set. A witness generator
    /// resolves every touched key against the same page store in one pass,
    /// so a page crossed by many paths -- which is what the upper trie is --
    /// is fetched ONCE for the batch. The dedup set therefore spans the
    /// batch rather than the key.
    pub fn read_batch(&mut self, rps: &[ReadPath], take: impl Fn(u32) -> bool) {
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        for rp in rps {
            self.read_into(rp, &take, &mut seen);
        }
    }

    fn read_into<F: Fn(u32) -> bool>(
        &mut self,
        rp: &ReadPath,
        take: &F,
        seen: &mut HashSet<Vec<u8>>,
    ) {
        for (i, (start_bit, pos, node_pos, _)) in rp.branches.iter().enumerate() {
            if !take(i as u32) {
                continue;
            }
            let (_level, id) = self.page_of(*start_bit, i as u32, pos, node_pos);
            if self.is_resident_depth(i as u32) {
                continue;
            }
            let elided = self.is_elided(&id);
            if !seen.insert(id) {
                continue;
            }
            // Either a page read, or a leaf-store lookup to rebuild the
            // elided subtree: one random read either way.
            if elided {
                self.elided_read_fallbacks += 1;
            }
            self.m.page_reads += 1;
            self.m.read_bytes += PAGE;
        }
        // The terminal leaf record sits in its own page, one level below the
        // deepest branch; charge it unless that page was already fetched.
        if rp.leaf.is_some() {
            let d = rp.branches.len() as u32;
            if take(d) {
                let (sb, pos, npos) = rp
                    .branches
                    .last()
                    .map(|(s, p, n, _)| (*s, p.clone(), n.clone()))
                    .unwrap_or((0, Vec::new(), Vec::new()));
                let (_level, id) = self.page_of(sb + 1, d, &pos, &npos);
                if !self.is_resident_depth(d) && seen.insert(id) {
                    self.m.page_reads += 1;
                    self.m.read_bytes += PAGE;
                }
            }
        }
    }

    pub fn sample_occupancy(&mut self) {
        self.occupancy_samples = self.pages.values().copied().collect();
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Bytes that must stay in DRAM for the modeled residency grant.
    pub fn dram_bytes(&self) -> u64 {
        self.resident_page_bytes
    }
}

pub struct PagedBackend {
    core: PagedCore,
    wal: Option<File>,
    buf: Vec<u8>,
}

impl PagedBackend {
    pub fn new(path: Option<&str>, resident_levels: u32) -> Self {
        Self::with_packing(path, resident_levels, Packing::Positional)
    }

    /// Judge pages against the full state `shape` describes rather than
    /// against this run's own records. Positional packing only: a page's
    /// root is then a bit path, which is what a shape is asked about.
    pub fn with_shape(mut self, shape: SharedShape) -> Self {
        assert!(
            self.core.packing == Packing::Positional,
            "a state shape needs positional packing"
        );
        self.core.shape = Some(shape);
        self
    }

    pub fn with_packing(path: Option<&str>, resident_levels: u32, packing: Packing) -> Self {
        let wal = path.map(|p| {
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(p)
                .expect("wal file")
        });
        PagedBackend {
            core: PagedCore::with_packing(resident_levels, packing),
            wal,
            buf: Vec::new(),
        }
    }
}

impl Backend for PagedBackend {
    fn name(&self) -> String {
        let p = match self.core.packing {
            Packing::Positional => "pos",
            Packing::Structural => "struct",
        };
        format!("paged-{p}(res={})", self.core.resident_levels)
    }

    fn commit_block(&mut self, batch: &CommitSink) {
        let t0 = Instant::now();
        let wal = self.core.apply(batch, |_| true);
        if let Some(f) = &mut self.wal {
            // bitbox truncates and rewrites a single-commit WAL blob, fsync.
            self.buf.clear();
            self.buf.resize(wal as usize, 0xCD);
            f.write_all(&self.buf).unwrap();
            f.sync_data().unwrap();
        }
        // WAL fsync + manifest fsync + ht fsync.
        self.core.m.fsyncs += 3;
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

    fn serve_state_read(&mut self, key: &[u8], _present: bool) {
        self.core.m.reads_served += 1;
        // The value store finds a key, or that it is absent, in the leaf
        // page where it would sit: one page either way (interior BBNs are
        // resident). Like the value store's write share, this feeds the
        // value-store curve only.
        let Some(shape) = self.core.shape.clone() else {
            return;
        };
        if let Some(p) = self.core.profile_vs.as_mut() {
            p.access_tagged(value_leaf_id(shape.value_leaf_page(key)), PAGE);
        }
    }

    fn serve_value_read(&mut self, _rp: &ReadPath) {
        self.core.m.reads_served += 1;
        // NOMT serves values from beatree, not from merkle pages. A B-tree
        // over N records with ~250-way 4KB nodes reaches a leaf in
        // ceil(log_250 N) hops; interior levels are assumed cached, so the
        // charge is the leaf fetch plus one interior miss.
        let n = self.core.page_count().max(1) as f64;
        let hops = (n.ln() / 250f64.ln()).ceil().max(1.0) as u64;
        let charged = hops.min(2);
        self.core.m.page_reads += charged;
        self.core.m.read_bytes += charged * PAGE;
    }

    fn metrics(&self) -> Metrics {
        let mut m = self.core.m.clone();
        m.disk_bytes = self.core.page_count() as u64 * PAGE;
        m.dram_bytes = self.core.dram_bytes();
        m
    }

    fn finalize(&mut self) {
        self.core.sample_occupancy();
        if let Some(f) = &mut self.wal {
            f.sync_all().unwrap();
        }
    }

    fn prepare_cache_sweep(&mut self) {
        if self.core.shape.is_none() {
            self.core.value_leaves = Some(ValueLeaves::new());
        }
    }

    fn enable_cache_profile(&mut self) {
        self.core.profile = Some(StackProfile::new());
        if self.core.value_leaves.is_some() || self.core.shape.is_some() {
            self.core.profile_vs = Some(StackProfile::new());
        }
    }

    fn begin_measurement(&mut self) {
        for p in [self.core.profile.as_mut(), self.core.profile_vs.as_mut()]
            .into_iter()
            .flatten()
        {
            p.restart_counts();
        }
        self.core.elided_touches = 0;
        self.core.leaf_changes = 0;
        self.core.value_leaf_page_writes = 0;
        self.core.value_bbn_writes = 0;
    }

    fn value_store_profile(&self) -> Option<CacheProfile> {
        self.core.value_store_profile()
    }

    fn cache_profile(&self) -> Option<CacheProfile> {
        self.core.cache_profile()
    }

    fn extra_report(&self) -> String {
        let n = self.core.occupancy_samples.len().max(1) as f64;
        let sum: u32 = self.core.occupancy_samples.iter().sum();
        let mean = sum as f64 / n;
        let mut hist = [0u64; 6]; // <10%, <25%, <50%, <75%, <100%, ==100%
        for &o in &self.core.occupancy_samples {
            let pct = o as f64 / NODES_PER_PAGE as f64;
            let b = if pct >= 1.0 {
                5
            } else if pct >= 0.75 {
                4
            } else if pct >= 0.5 {
                3
            } else if pct >= 0.25 {
                2
            } else if pct >= 0.10 {
                1
            } else {
                0
            };
            hist[b] += 1;
        }
        format!(
            "- {}: {} stored pages, mean occupancy {:.1}/{} slots ({:.1}%), WAL {} MB\n  - elision: {} page writes avoided, {} read fallbacks\n  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: {:?}\n",
            self.name(),
            self.core.page_count(),
            mean,
            NODES_PER_PAGE,
            100.0 * mean / NODES_PER_PAGE as f64,
            self.core.wal_bytes >> 20,
            self.core.elided_writes_saved,
            self.core.elided_read_fallbacks,
            hist
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(i: u32) -> Vec<u8> {
        i.to_be_bytes().to_vec()
    }

    /// A leaf page splits only when it overflows, the two halves keep every
    /// key, and a key always lands in the page whose range covers it.
    #[test]
    fn value_leaves_split_at_capacity_and_keep_keys_in_range() {
        let mut vl = ValueLeaves::new();
        let mut splits = 0;
        for i in 0..VALUE_LEAF_ENTRIES as u32 {
            assert!(vl.upsert(&key(i * 2)).split_off.is_none());
        }
        assert_eq!(vl.page_count(), 1);
        if vl.upsert(&key(1)).split_off.is_some() {
            splits += 1;
        }
        assert_eq!(splits, 1);
        assert_eq!(vl.page_count(), 2);
        let total: usize = vl.pages.values().map(|(_, k)| k.len()).sum();
        assert_eq!(total, VALUE_LEAF_ENTRIES + 1);
        // Overwriting an existing key changes nothing structurally and
        // reports the page that holds it.
        let low = vl.upsert(&key(0));
        let high = vl.upsert(&key((VALUE_LEAF_ENTRIES as u32 - 1) * 2));
        assert!(low.split_off.is_none() && high.split_off.is_none());
        assert_ne!(low.page, high.page);
        assert_eq!(
            vl.pages.values().map(|(_, k)| k.len()).sum::<usize>(),
            total
        );
    }

    /// Bottom branch nodes split when they overflow and always account for
    /// every leaf page exactly once.
    #[test]
    fn value_bbns_split_at_capacity_and_cover_every_page() {
        let mut vl = ValueLeaves::new();
        // Enough ascending keys for well over one full node of leaf pages.
        let n = (VALUE_LEAF_ENTRIES * VALUE_BBN_MAX * 3) as u32;
        for i in 0..n {
            vl.upsert(&key(i));
        }
        assert!(vl.bbn_count() > 1);
        let covered: usize = vl.bbns.values().map(|(_, c)| *c).sum();
        assert_eq!(covered as u64, vl.page_count());
        assert!(vl.bbns.values().all(|(_, c)| *c <= VALUE_BBN_MAX && *c > 0));
        // A key reports the node whose range covers its page.
        let t = vl.upsert(&key(n / 2));
        let first = floor_key(&vl.pages, &key(n / 2));
        assert_eq!(t.bbns[0], Some(vl.bbn_id(&first)));
    }

    /// Removing every key of a page drops the page; the catch-all first
    /// page is never dropped; removing an absent key reports nothing.
    #[test]
    fn value_leaves_drop_emptied_pages() {
        let mut vl = ValueLeaves::new();
        for i in 0..200u32 {
            vl.upsert(&key(i));
        }
        let before = vl.page_count();
        assert!(before > 1);
        assert!(vl.remove(&key(9999)).is_none());
        let mut dropped = 0;
        for i in 0..200u32 {
            if vl.remove(&key(i)).expect("present").dropped {
                dropped += 1;
            }
        }
        assert_eq!(vl.page_count(), 1);
        assert_eq!(dropped as u64, before - 1);
        assert_eq!(vl.bbns.values().map(|(_, c)| *c).sum::<usize>(), 1);
    }

    /// Bit depths 0..n so both page levels 0 and 1 are exercised.
    fn path(tail: u8, depth: u32) -> ReadPath {
        ReadPath {
            branches: (0..depth)
                .map(|d| {
                    let mut pos = vec![0u8, d as u8];
                    pos.extend_from_slice(&[0xF0, tail]);
                    (d, pos.clone(), pos, 100u32)
                })
                .collect(),
            leaf: Some((vec![tail].into_boxed_slice(), 100)),
        }
    }

    /// The upper trie is crossed by every path in a block. Per-key service
    /// refetches its pages once per key; one multiproof fetches them once.
    #[test]
    fn batched_reads_charge_a_shared_page_once() {
        let a = path(1, 10);
        let b = path(2, 10);

        let mut per_key = PagedCore::with_packing(0, Packing::Positional);
        per_key.read(&a, |_| true);
        per_key.read(&b, |_| true);

        let mut batched = PagedCore::with_packing(0, Packing::Positional);
        batched.read_batch(&[a, b], |_| true);

        assert!(batched.m.page_reads < per_key.m.page_reads);
        assert_eq!(batched.m.read_bytes, batched.m.page_reads * PAGE);
    }

    /// A batch of one is exactly the per-key charge: batching must not change
    /// the single-lookup baseline.
    #[test]
    fn batch_of_one_matches_per_key() {
        let mut per_key = PagedCore::with_packing(0, Packing::Positional);
        per_key.read(&path(7, 14), |_| true);
        let mut batched = PagedCore::with_packing(0, Packing::Positional);
        batched.read_batch(&[path(7, 14)], |_| true);
        assert_eq!(batched.m.page_reads, per_key.m.page_reads);
        assert_eq!(batched.m.read_bytes, per_key.m.read_bytes);
    }
}
