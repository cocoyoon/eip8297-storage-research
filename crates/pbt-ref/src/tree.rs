//! Incremental PBT: insert/delete maintaining the canonical form, cached
//! node hashes, and an instrumented root recomputation that reports which
//! node records changed, bucketed by (zone, node depth). This is the
//! Phase-1 characterization instrument.
//!
//! Key structural facts exploited here (from the spec):
//! - A LeafNode commits its complete key, so a leaf's hash changes only
//!   when its value changes -- promotion and demotion never dirty it.
//! - A BranchNode's prefix is exactly the shared bit run, so splits and
//!   merges are bounded to one level and always dirty the branch involved.

use crate::bits::{bit, bit_len, encode_bit_prefix};
use crate::hash::{h, Digest, EMPTY_ROOT, ZERO_VALUE};
use crate::keys::MAX_KEY_LENGTH;
use std::collections::BTreeMap;

pub const LEAF_TAG: u8 = 0x00;
pub const BRANCH_TAG: u8 = 0x01;

pub enum Node {
    Leaf {
        key: Box<[u8]>,
        value: Digest,
        cache: Option<Digest>,
        committed: Option<Digest>,
    },
    Branch {
        prefix: Vec<u8>,
        left: Box<Node>,
        right: Box<Node>,
        cache: Option<Digest>,
        committed: Option<Digest>,
    },
}

/// Zone attribution of a node record. A node is attributed to a zone as
/// soon as the bits fixed on its path PLUS its own prefix determine the
/// full first key byte; the handful of nodes above that are `Top`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum ZoneLabel {
    Top,
    Account,
    Code,
    Storage,
    Other(u8),
}

impl ZoneLabel {
    fn from_byte(b: u8) -> ZoneLabel {
        match b {
            0x00 => ZoneLabel::Account,
            0x01 => ZoneLabel::Code,
            0xFF => ZoneLabel::Storage,
            other => ZoneLabel::Other(other),
        }
    }
    pub fn name(&self) -> String {
        match self {
            ZoneLabel::Top => "top".into(),
            ZoneLabel::Account => "account".into(),
            ZoneLabel::Code => "code".into(),
            ZoneLabel::Storage => "storage".into(),
            ZoneLabel::Other(b) => format!("zone_{b:02x}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Leaf,
    Branch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordOp {
    Create,
    Rewrite,
}

/// One physical record whose committed bytes changed in this block: the
/// unit handed to a storage backend.
pub struct CommitRecord {
    pub kind: RecordKind,
    pub op: RecordOp,
    pub zone: ZoneLabel,
    pub node_depth: u32,
    /// Bits consumed to reach this record (its stable tree position).
    pub start_bit: u32,
    /// Position key: encode_bit_prefix of the path bits at entry.
    pub pos: Vec<u8>,
    /// Node-tree position: one child-selector bit per ancestor branch,
    /// packed. Unlike `pos` this ignores compressed prefix bits, so it
    /// counts NODE RECORDS rather than bit levels.
    pub node_pos: Vec<u8>,
    /// For leaves, the complete key.
    pub leaf_key: Option<Box<[u8]>>,
    /// Serialized record size: leaf = 1+keylen+32; branch = 3+ceil(p/8)+64.
    pub bytes: u32,
}

/// Everything a physical backend needs to apply one block.
#[derive(Default)]
pub struct CommitSink {
    pub records: Vec<CommitRecord>,
    pub deletes: Vec<DeleteEvent>,
}

/// A state lookup resolved against the tree: the records a backend must
/// fetch to serve it.
pub struct ReadPath {
    /// (start_bit, pos, node_pos, record_bytes) of every branch on the path.
    pub branches: Vec<(u32, Vec<u8>, Vec<u8>, u32)>,
    /// The terminal leaf, if the key is present.
    pub leaf: Option<(Box<[u8]>, u32)>,
}

#[derive(Default, Clone, Copy)]
pub struct Cell {
    /// Brand-new records (never committed before).
    pub branch_creates: u64,
    pub leaf_creates: u64,
    /// Records whose committed bytes actually changed (the §3 variable).
    pub branch_rewrites: u64,
    pub leaf_rewrites: u64,
    /// Records removed (leaf deletions; branches eliminated by merges).
    pub branch_deletes: u64,
    pub leaf_deletes: u64,
}

/// Rewrites bucketed by (zone, node_depth). node_depth counts node records
/// from the root (root = 0): the number of records an update path touches.
#[derive(Default)]
pub struct RewriteStats {
    pub cells: BTreeMap<(ZoneLabel, u32), Cell>,
}

impl RewriteStats {
    fn count_branch(&mut self, label: ZoneLabel, depth: u32, fresh: bool) {
        let c = self.cells.entry((label, depth)).or_default();
        if fresh {
            c.branch_creates += 1;
        } else {
            c.branch_rewrites += 1;
        }
    }
    fn count_leaf(&mut self, label: ZoneLabel, depth: u32, fresh: bool) {
        let c = self.cells.entry((label, depth)).or_default();
        if fresh {
            c.leaf_creates += 1;
        } else {
            c.leaf_rewrites += 1;
        }
    }
    pub fn absorb(&mut self, other: &RewriteStats) {
        for (k, c) in &other.cells {
            let e = self.cells.entry(*k).or_default();
            e.branch_creates += c.branch_creates;
            e.leaf_creates += c.leaf_creates;
            e.branch_rewrites += c.branch_rewrites;
            e.leaf_rewrites += c.leaf_rewrites;
            e.branch_deletes += c.branch_deletes;
            e.leaf_deletes += c.leaf_deletes;
        }
    }
    pub fn total_branch_writes(&self) -> u64 {
        self.cells
            .values()
            .map(|c| c.branch_creates + c.branch_rewrites)
            .sum()
    }
    pub fn total_leaf_writes(&self) -> u64 {
        self.cells
            .values()
            .map(|c| c.leaf_creates + c.leaf_rewrites)
            .sum()
    }
    pub fn total_leaf_rewrites(&self) -> u64 {
        self.cells.values().map(|c| c.leaf_rewrites).sum()
    }
}

#[derive(Default, Clone, Copy)]
pub struct CensusCell {
    pub branches: u64,
    pub leaves: u64,
    /// Sum of branch prefix bit lengths (for mean prefix length).
    pub prefix_bits: u64,
}

/// Static structure census: population per (zone, node_depth), plus branch
/// prefix-length distribution per zone (informs RQ5 page occupancy).
#[derive(Default)]
pub struct Census {
    pub cells: BTreeMap<(ZoneLabel, u32), CensusCell>,
    /// prefix bit-length histogram per zone label.
    pub prefix_hist: BTreeMap<(ZoneLabel, u32), u64>,
    /// leaf bit-depth (bits consumed to reach the leaf's position).
    pub leaf_bit_depth_sum: BTreeMap<ZoneLabel, (u64, u64)>, // (sum, count)
    pub max_node_depth: u32,
}

/// Path context for zone labeling: the first up-to-8 bits fixed so far.
#[derive(Clone, Copy)]
struct PathBits {
    acc: u8,
    n: u8, // number of fixed bits, saturating at 8
}

impl PathBits {
    fn root() -> Self {
        PathBits { acc: 0, n: 0 }
    }
    fn push_bit(mut self, b: u8) -> Self {
        if self.n < 8 {
            self.acc |= b << (7 - self.n);
            self.n += 1;
        }
        self
    }
    fn push_prefix(mut self, prefix: &[u8]) -> Self {
        for &b in prefix {
            if self.n >= 8 {
                break;
            }
            self = self.push_bit(b);
        }
        self
    }
    fn label(self) -> ZoneLabel {
        if self.n >= 8 {
            ZoneLabel::from_byte(self.acc)
        } else {
            ZoneLabel::Top
        }
    }
}

/// A deletion event recorded at mutation time (the record no longer exists
/// at commit, so its identity must be captured here). Zone attribution uses
/// the deleted key's zone byte, which can mislabel a Top-branch merge; with
/// only three populated zones that requires emptying a whole zone, so the
/// bias is negligible.
#[derive(Clone)]
pub struct DeleteEvent {
    pub zone_byte: u8,
    pub node_depth: u32,
    pub was_leaf: bool,
    /// Bits consumed to reach the record (its stable position in the tree).
    pub start_bit: u32,
    /// Position key: encode_bit_prefix of the path bits (leaf: full key bits
    /// are recoverable from `leaf_key`).
    pub pos: Vec<u8>,
    /// Node-tree position (child-selector bits only).
    pub node_pos: Vec<u8>,
    /// For leaves, the complete key (the leaf store's natural key).
    pub leaf_key: Option<Box<[u8]>>,
    /// Serialized record size that is being freed.
    pub bytes: u32,
}

pub struct PbtTree {
    root: Option<Box<Node>>,
    pending_deletes: Vec<DeleteEvent>,
}

impl Default for PbtTree {
    fn default() -> Self {
        Self::new()
    }
}

impl PbtTree {
    pub fn new() -> Self {
        PbtTree {
            root: None,
            pending_deletes: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// state_write: a write of 32 zero bytes is a deletion (spec, "Zero
    /// values and deletion"). Returns true if the entry set changed.
    pub fn write(&mut self, key: &[u8], value: &Digest) -> bool {
        if *value == ZERO_VALUE {
            self.delete(key)
        } else {
            self.insert(key, value)
        }
    }

    /// Insert or update. Returns true if the entry set changed.
    pub fn insert(&mut self, key: &[u8], value: &Digest) -> bool {
        assert!(
            (1..=MAX_KEY_LENGTH).contains(&key.len()),
            "key length out of range"
        );
        match self.root.take() {
            None => {
                self.root = Some(Box::new(Node::Leaf {
                    key: key.into(),
                    value: *value,
                    cache: None,
                    committed: None,
                }));
                true
            }
            Some(node) => {
                let (node, changed) = insert_node(node, key, value, 0);
                self.root = Some(node);
                changed
            }
        }
    }

    /// Delete. Absent key is a no-op. Returns true if the entry set changed.
    pub fn delete(&mut self, key: &[u8]) -> bool {
        assert!(
            (1..=MAX_KEY_LENGTH).contains(&key.len()),
            "key length out of range"
        );
        match self.root.take() {
            None => false,
            Some(node) => {
                let mut npath: Vec<u8> = Vec::new();
                let (node, changed) =
                    delete_node(node, key, 0, 0, &mut npath, &mut self.pending_deletes);
                self.root = node;
                changed
            }
        }
    }

    /// Commit: recompute the root and classify every touched node record
    /// against its state at the previous commit -- created (never committed),
    /// rewritten (committed bytes changed), or unchanged (recomputed but
    /// byte-identical, e.g. a within-block set-then-restore: not counted).
    /// Deletion events recorded since the last commit are drained into
    /// `stats` (and into `sink` when provided, along with every created or
    /// rewritten record, forming the batch a physical backend applies).
    pub fn root_with_stats(&mut self, stats: &mut RewriteStats) -> Digest {
        self.commit(stats, None)
    }

    pub fn commit(
        &mut self,
        stats: &mut RewriteStats,
        mut sink: Option<&mut CommitSink>,
    ) -> Digest {
        for ev in self.pending_deletes.drain(..) {
            let label = ZoneLabel::from_byte(ev.zone_byte);
            let c = stats.cells.entry((label, ev.node_depth)).or_default();
            if ev.was_leaf {
                c.leaf_deletes += 1;
            } else {
                c.branch_deletes += 1;
            }
            if let Some(s) = sink.as_deref_mut() {
                s.deletes.push(ev);
            }
        }
        match &mut self.root {
            None => EMPTY_ROOT,
            Some(node) => {
                let mut path: Vec<u8> = Vec::with_capacity(600);
                let mut npath: Vec<u8> = Vec::with_capacity(64);
                walk_hash(node, 0, &mut path, &mut npath, stats, &mut sink)
            }
        }
    }

    /// Resolve a state lookup into the records a backend must fetch.
    /// Returns None only for an empty tree.
    pub fn read_path(&self, key: &[u8]) -> Option<ReadPath> {
        let mut node = self.root.as_deref()?;
        let mut depth = 0usize;
        let mut npath: Vec<u8> = Vec::new();
        let mut out = ReadPath {
            branches: Vec::new(),
            leaf: None,
        };
        loop {
            match node {
                Node::Leaf { key: lk, .. } => {
                    if &**lk == key {
                        out.leaf = Some((lk.clone(), leaf_record_bytes(lk.len())));
                    }
                    return Some(out);
                }
                Node::Branch {
                    prefix,
                    left,
                    right,
                    ..
                } => {
                    out.branches.push((
                        depth as u32,
                        pos_key_of(key, depth as u32),
                        pos_key(&npath),
                        branch_record_bytes(prefix.len()),
                    ));
                    for (j, &pb) in prefix.iter().enumerate() {
                        if depth + j >= bit_len(key) || bit(key, depth + j) != pb {
                            return Some(out); // absent: proof of exclusion ends here
                        }
                    }
                    let d2 = depth + prefix.len();
                    if d2 >= bit_len(key) {
                        return Some(out);
                    }
                    let sel = bit(key, d2);
                    npath.push(sel);
                    node = if sel == 0 { left } else { right };
                    depth = d2 + 1;
                }
            }
        }
    }

    /// Root without instrumentation.
    pub fn root(&mut self) -> Digest {
        let mut sink = RewriteStats::default();
        self.root_with_stats(&mut sink)
    }

    /// Full-structure census. Requires hashes to be clean or not; walks
    /// everything regardless of caches.
    pub fn census(&self) -> Census {
        let mut c = Census::default();
        if let Some(node) = &self.root {
            walk_census(node, 0, 0, PathBits::root(), &mut c);
        }
        c
    }

    /// The nodes above every zone-pure subtree: (node_depth, prefix_bits,
    /// child_zone_split) for each Top-labeled branch. Resolves Open Item O1.
    pub fn top_structure(&self) -> Vec<TopNode> {
        let mut out = Vec::new();
        if let Some(node) = &self.root {
            collect_top(node, 0, PathBits::root(), &mut out);
        }
        out
    }
}

pub struct TopNode {
    pub node_depth: u32,
    pub prefix_bits: usize,
    pub left_label: ZoneLabel,
    pub right_label: ZoneLabel,
}

fn collect_top(node: &Node, depth: u32, path: PathBits, out: &mut Vec<TopNode>) {
    if let Node::Branch {
        prefix,
        left,
        right,
        ..
    } = node
    {
        let here = path.push_prefix(prefix);
        if here.label() != ZoneLabel::Top {
            return;
        }
        let lpath = here.push_bit(0);
        let rpath = here.push_bit(1);
        let llabel = node_label(left, lpath);
        let rlabel = node_label(right, rpath);
        out.push(TopNode {
            node_depth: depth,
            prefix_bits: prefix.len(),
            left_label: llabel,
            right_label: rlabel,
        });
        collect_top(left, depth + 1, lpath, out);
        collect_top(right, depth + 1, rpath, out);
    }
}

fn node_label(node: &Node, path: PathBits) -> ZoneLabel {
    match node {
        Node::Leaf { key, .. } => ZoneLabel::from_byte(key[0]),
        Node::Branch { prefix, .. } => path.push_prefix(prefix).label(),
    }
}

fn insert_node(node: Box<Node>, key: &[u8], value: &Digest, depth: usize) -> (Box<Node>, bool) {
    match *node {
        Node::Leaf {
            key: lk,
            value: lv,
            cache,
            committed,
        } => {
            if &*lk == key {
                if lv == *value {
                    // Same value: entry set unchanged, cache stays valid.
                    return (
                        Box::new(Node::Leaf {
                            key: lk,
                            value: lv,
                            cache,
                            committed,
                        }),
                        false,
                    );
                }
                return (
                    Box::new(Node::Leaf {
                        key: lk,
                        value: *value,
                        cache: None,
                        committed,
                    }),
                    true,
                );
            }
            // Split: find the first bit where the keys diverge.
            let mut s = depth;
            loop {
                assert!(
                    s < bit_len(&lk) && s < bit_len(key),
                    "keys are not prefix-free"
                );
                if bit(&lk, s) != bit(key, s) {
                    break;
                }
                s += 1;
            }
            let prefix: Vec<u8> = (depth..s).map(|i| bit(key, i)).collect();
            // The existing leaf commits its complete key: demotion does not
            // change its hash, so its cache is preserved.
            let old_leaf = Box::new(Node::Leaf {
                key: lk,
                value: lv,
                cache,
                committed,
            });
            let new_leaf = Box::new(Node::Leaf {
                key: key.into(),
                value: *value,
                cache: None,
                committed: None,
            });
            let (left, right) = if bit(key, s) == 0 {
                (new_leaf, old_leaf)
            } else {
                (old_leaf, new_leaf)
            };
            // The split-created branch is a brand-new record.
            (
                Box::new(Node::Branch {
                    prefix,
                    left,
                    right,
                    cache: None,
                    committed: None,
                }),
                true,
            )
        }
        Node::Branch {
            prefix,
            left,
            right,
            cache,
            committed,
        } => {
            // Compare the key against the branch prefix.
            let plen = prefix.len();
            let mut j = 0;
            while j < plen {
                assert!(depth + j < bit_len(key), "keys are not prefix-free");
                if bit(key, depth + j) != prefix[j] {
                    break;
                }
                j += 1;
            }
            if j < plen {
                // Diverges inside the prefix: split this branch.
                let s = depth + j;
                let head = prefix[..j].to_vec();
                let tail = prefix[j + 1..].to_vec();
                // The demoted branch's prefix changed: same record, new bytes.
                let demoted = Box::new(Node::Branch {
                    prefix: tail,
                    left,
                    right,
                    cache: None,
                    committed,
                });
                let new_leaf = Box::new(Node::Leaf {
                    key: key.into(),
                    value: *value,
                    cache: None,
                    committed: None,
                });
                let (l, r) = if bit(key, s) == 0 {
                    (new_leaf, demoted)
                } else {
                    (demoted, new_leaf)
                };
                // The new head branch is a brand-new record.
                return (
                    Box::new(Node::Branch {
                        prefix: head,
                        left: l,
                        right: r,
                        cache: None,
                        committed: None,
                    }),
                    true,
                );
            }
            assert!(depth + plen < bit_len(key), "keys are not prefix-free");
            let sel = bit(key, depth + plen);
            let child_depth = depth + plen + 1;
            if sel == 0 {
                let (nl, changed) = insert_node(left, key, value, child_depth);
                let cache = if changed { None } else { cache };
                (
                    Box::new(Node::Branch {
                        prefix,
                        left: nl,
                        right,
                        cache,
                        committed,
                    }),
                    changed,
                )
            } else {
                let (nr, changed) = insert_node(right, key, value, child_depth);
                let cache = if changed { None } else { cache };
                (
                    Box::new(Node::Branch {
                        prefix,
                        left,
                        right: nr,
                        cache,
                        committed,
                    }),
                    changed,
                )
            }
        }
    }
}

fn delete_node(
    node: Box<Node>,
    key: &[u8],
    depth: usize,
    node_depth: u32,
    npath: &mut Vec<u8>,
    events: &mut Vec<DeleteEvent>,
) -> (Option<Box<Node>>, bool) {
    match *node {
        Node::Leaf {
            key: lk,
            value,
            cache,
            committed,
        } => {
            if &*lk == key {
                events.push(DeleteEvent {
                    zone_byte: key[0],
                    node_depth,
                    was_leaf: true,
                    start_bit: depth as u32,
                    pos: pos_key_of(key, depth as u32),
                    node_pos: pos_key(npath),
                    leaf_key: Some(lk),
                    bytes: leaf_record_bytes(key.len()),
                });
                (None, true)
            } else {
                (
                    Some(Box::new(Node::Leaf {
                        key: lk,
                        value,
                        cache,
                        committed,
                    })),
                    false,
                )
            }
        }
        Node::Branch {
            prefix,
            left,
            right,
            cache,
            committed,
        } => {
            // If the key is inconsistent with the prefix, it is absent.
            for (j, &pb) in prefix.iter().enumerate() {
                if depth + j >= bit_len(key) || bit(key, depth + j) != pb {
                    return (
                        Some(Box::new(Node::Branch {
                            prefix,
                            left,
                            right,
                            cache,
                            committed,
                        })),
                        false,
                    );
                }
            }
            let d2 = depth + prefix.len();
            if d2 >= bit_len(key) {
                return (
                    Some(Box::new(Node::Branch {
                        prefix,
                        left,
                        right,
                        cache,
                        committed,
                    })),
                    false,
                );
            }
            let sel = bit(key, d2);
            if sel == 0 {
                npath.push(0);
                let (nl, changed) = delete_node(left, key, d2 + 1, node_depth + 1, npath, events);
                npath.pop();
                match nl {
                    Some(l) => {
                        let cache = if changed { None } else { cache };
                        (
                            Some(Box::new(Node::Branch {
                                prefix,
                                left: l,
                                right,
                                cache,
                                committed,
                            })),
                            changed,
                        )
                    }
                    None => {
                        // This branch record is eliminated by the merge.
                        events.push(DeleteEvent {
                            zone_byte: key[0],
                            node_depth,
                            was_leaf: false,
                            start_bit: depth as u32,
                            pos: pos_key_of(key, depth as u32),
                            node_pos: pos_key(npath),
                            leaf_key: None,
                            bytes: branch_record_bytes(prefix.len()),
                        });
                        (Some(merge_survivor(prefix, 1, right)), true)
                    }
                }
            } else {
                npath.push(1);
                let (nr, changed) = delete_node(right, key, d2 + 1, node_depth + 1, npath, events);
                npath.pop();
                match nr {
                    Some(r) => {
                        let cache = if changed { None } else { cache };
                        (
                            Some(Box::new(Node::Branch {
                                prefix,
                                left,
                                right: r,
                                cache,
                                committed,
                            })),
                            changed,
                        )
                    }
                    None => {
                        events.push(DeleteEvent {
                            zone_byte: key[0],
                            node_depth,
                            was_leaf: false,
                            start_bit: depth as u32,
                            pos: pos_key_of(key, depth as u32),
                            node_pos: pos_key(npath),
                            leaf_key: None,
                            bytes: branch_record_bytes(prefix.len()),
                        });
                        (Some(merge_survivor(prefix, 0, left)), true)
                    }
                }
            }
        }
    }
}

/// Deletion left the parent with a single child: restore canonical form.
/// A surviving LeafNode is promoted unchanged (its hash is intact); a
/// surviving BranchNode takes parent's prefix + selecting bit + its own.
fn merge_survivor(mut parent_prefix: Vec<u8>, sel_bit: u8, survivor: Box<Node>) -> Box<Node> {
    match *survivor {
        Node::Leaf { .. } => survivor,
        Node::Branch {
            prefix,
            left,
            right,
            committed,
            ..
        } => {
            parent_prefix.push(sel_bit);
            parent_prefix.extend_from_slice(&prefix);
            // Same surviving record with a longer prefix: rewrite, not create.
            Box::new(Node::Branch {
                prefix: parent_prefix,
                left,
                right,
                cache: None,
                committed,
            })
        }
    }
}

pub fn leaf_record_bytes(key_len: usize) -> u32 {
    (1 + key_len + 32) as u32
}

pub fn branch_record_bytes(prefix_len: usize) -> u32 {
    (3 + (prefix_len + 7) / 8 + 64) as u32
}

/// Position key from a bit path (same packing as encode_bit_prefix).
fn pos_key(bits: &[u8]) -> Vec<u8> {
    encode_bit_prefix(bits)
}

/// Position key from a byte key's first `nbits` bits.
fn pos_key_of(key: &[u8], nbits: u32) -> Vec<u8> {
    let bits: Vec<u8> = (0..nbits as usize).map(|i| bit(key, i)).collect();
    encode_bit_prefix(&bits)
}

fn leaf_hash(key: &[u8], value: &Digest) -> Digest {
    let mut buf = Vec::with_capacity(1 + key.len() + 32);
    buf.push(LEAF_TAG);
    buf.extend_from_slice(key);
    buf.extend_from_slice(value);
    h(&buf)
}

fn branch_hash(prefix: &[u8], left: &Digest, right: &Digest) -> Digest {
    let mut buf = Vec::new();
    buf.push(BRANCH_TAG);
    buf.extend_from_slice(&encode_bit_prefix(prefix));
    buf.extend_from_slice(left);
    buf.extend_from_slice(right);
    h(&buf)
}

/// Zone label from an explicit bit path plus the node's own prefix.
fn label_from_path(path: &[u8], prefix: &[u8]) -> ZoneLabel {
    if path.len() >= 8 {
        return ZoneLabel::from_byte(byte_from_bits(&path[..8]));
    }
    let need = 8 - path.len();
    if prefix.len() >= need {
        let mut bits = [0u8; 8];
        bits[..path.len()].copy_from_slice(path);
        bits[path.len()..].copy_from_slice(&prefix[..need]);
        return ZoneLabel::from_byte(byte_from_bits(&bits));
    }
    ZoneLabel::Top
}

fn byte_from_bits(bits: &[u8]) -> u8 {
    bits.iter().fold(0u8, |acc, &b| (acc << 1) | b)
}

/// Recompute hashes, skipping clean subtrees; classify every recomputed
/// record vs its previous commit; optionally emit records into a sink.
/// `path` holds the bits consumed to reach the node (its position).
fn walk_hash(
    node: &mut Node,
    depth: u32,
    path: &mut Vec<u8>,
    npath: &mut Vec<u8>,
    stats: &mut RewriteStats,
    sink: &mut Option<&mut CommitSink>,
) -> Digest {
    match node {
        Node::Leaf {
            key,
            value,
            cache,
            committed,
        } => {
            if let Some(hv) = cache {
                return *hv;
            }
            let hv = leaf_hash(key, value);
            let op = match *committed {
                Some(prev) if prev == hv => None, // net-zero within the block
                Some(_) => Some(RecordOp::Rewrite),
                None => Some(RecordOp::Create),
            };
            if let Some(op) = op {
                let zone = ZoneLabel::from_byte(key[0]);
                stats.count_leaf(zone, depth, op == RecordOp::Create);
                if let Some(s) = sink.as_deref_mut() {
                    s.records.push(CommitRecord {
                        kind: RecordKind::Leaf,
                        op,
                        zone,
                        node_depth: depth,
                        start_bit: path.len() as u32,
                        pos: pos_key(path),
                        node_pos: pos_key(npath),
                        leaf_key: Some(key.clone()),
                        bytes: leaf_record_bytes(key.len()),
                    });
                }
            }
            *cache = Some(hv);
            *committed = Some(hv);
            hv
        }
        Node::Branch {
            prefix,
            left,
            right,
            cache,
            committed,
        } => {
            if let Some(hv) = cache {
                return *hv;
            }
            let start_bit = path.len() as u32;
            let entry_pos_needed = sink.is_some();
            let entry_pos = if entry_pos_needed {
                pos_key(path)
            } else {
                Vec::new()
            };
            let entry_npos = if entry_pos_needed {
                pos_key(npath)
            } else {
                Vec::new()
            };
            let label = label_from_path(path, prefix);
            path.extend_from_slice(prefix);
            path.push(0);
            npath.push(0);
            let lh = walk_hash(left, depth + 1, path, npath, stats, sink);
            *path.last_mut().unwrap() = 1;
            *npath.last_mut().unwrap() = 1;
            let rh = walk_hash(right, depth + 1, path, npath, stats, sink);
            npath.pop();
            path.truncate(path.len() - 1 - prefix.len());
            let hv = branch_hash(prefix, &lh, &rh);
            let op = match *committed {
                Some(prev) if prev == hv => None, // net-zero within the block
                Some(_) => Some(RecordOp::Rewrite),
                None => Some(RecordOp::Create),
            };
            if let Some(op) = op {
                stats.count_branch(label, depth, op == RecordOp::Create);
                if let Some(s) = sink.as_deref_mut() {
                    s.records.push(CommitRecord {
                        kind: RecordKind::Branch,
                        op,
                        zone: label,
                        node_depth: depth,
                        start_bit,
                        pos: entry_pos,
                        node_pos: entry_npos,
                        leaf_key: None,
                        bytes: branch_record_bytes(prefix.len()),
                    });
                }
            }
            *cache = Some(hv);
            *committed = Some(hv);
            hv
        }
    }
}

fn walk_census(node: &Node, depth: u32, bit_depth: u64, path: PathBits, census: &mut Census) {
    census.max_node_depth = census.max_node_depth.max(depth);
    match node {
        Node::Leaf { key, .. } => {
            let label = ZoneLabel::from_byte(key[0]);
            census.cells.entry((label, depth)).or_default().leaves += 1;
            let e = census.leaf_bit_depth_sum.entry(label).or_default();
            e.0 += bit_depth;
            e.1 += 1;
        }
        Node::Branch {
            prefix,
            left,
            right,
            ..
        } => {
            let here = path.push_prefix(prefix);
            let label = here.label();
            let cell = census.cells.entry((label, depth)).or_default();
            cell.branches += 1;
            cell.prefix_bits += prefix.len() as u64;
            *census
                .prefix_hist
                .entry((label, prefix.len() as u32))
                .or_default() += 1;
            let child_bit_depth = bit_depth + prefix.len() as u64 + 1;
            walk_census(left, depth + 1, child_bit_depth, here.push_bit(0), census);
            walk_census(right, depth + 1, child_bit_depth, here.push_bit(1), census);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::key_hash;
    use crate::oracle;
    use std::collections::BTreeMap;

    /// Deterministic RNG (SplitMix64) for reproducible fuzzing.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_key(rng: &mut Rng) -> Vec<u8> {
        // Realistic key shapes: account/code (34B) or storage (66B) zone.
        let zone = match rng.below(3) {
            0 => 0x00,
            1 => 0x01,
            _ => 0xFF,
        };
        let pos = key_hash(&rng.next().to_be_bytes());
        if zone == 0xFF {
            let pos2 = key_hash(&rng.next().to_be_bytes());
            let mut k = vec![zone];
            k.extend_from_slice(&pos);
            k.extend_from_slice(&pos2);
            k.push(rng.below(256) as u8);
            k
        } else {
            let mut k = vec![zone];
            k.extend_from_slice(&pos);
            k.push(rng.below(256) as u8);
            k
        }
    }

    fn random_value(rng: &mut Rng) -> Digest {
        let mut v = [0u8; 32];
        v[..8].copy_from_slice(&rng.next().to_be_bytes());
        v[31] = 1; // never the zero value
        v
    }

    #[test]
    fn empty_root_is_zero() {
        let mut t = PbtTree::new();
        assert_eq!(t.root(), EMPTY_ROOT);
        assert_eq!(oracle::state_root(&BTreeMap::new()), EMPTY_ROOT);
    }

    #[test]
    fn single_leaf_root() {
        let mut t = PbtTree::new();
        let key = vec![0x00u8; 34];
        let value = [7u8; 32];
        t.insert(&key, &value);
        let mut buf = vec![LEAF_TAG];
        buf.extend_from_slice(&key);
        buf.extend_from_slice(&value);
        assert_eq!(t.root(), h(&buf));
    }

    #[test]
    fn incremental_matches_oracle_bulk() {
        let mut rng = Rng(42);
        let mut tree = PbtTree::new();
        let mut map: BTreeMap<Vec<u8>, Digest> = BTreeMap::new();
        for _ in 0..2000 {
            let k = random_key(&mut rng);
            let v = random_value(&mut rng);
            tree.insert(&k, &v);
            map.insert(k, v);
        }
        assert_eq!(tree.root(), oracle::state_root(&map));
    }

    #[test]
    fn incremental_matches_oracle_under_mutation() {
        let mut rng = Rng(1234);
        let mut tree = PbtTree::new();
        let mut map: BTreeMap<Vec<u8>, Digest> = BTreeMap::new();
        let mut live: Vec<Vec<u8>> = Vec::new();

        for round in 0..40 {
            for _ in 0..100 {
                match rng.below(10) {
                    // 60%: insert fresh
                    0..=5 => {
                        let k = random_key(&mut rng);
                        let v = random_value(&mut rng);
                        tree.insert(&k, &v);
                        map.insert(k.clone(), v);
                        live.push(k);
                    }
                    // 20%: update existing
                    6..=7 if !live.is_empty() => {
                        let k = live[rng.below(live.len() as u64) as usize].clone();
                        let v = random_value(&mut rng);
                        tree.insert(&k, &v);
                        map.insert(k, v);
                    }
                    // 10%: delete existing
                    8 if !live.is_empty() => {
                        let i = rng.below(live.len() as u64) as usize;
                        let k = live.swap_remove(i);
                        let changed = tree.delete(&k);
                        assert!(changed);
                        map.remove(&k);
                    }
                    // 10%: delete absent (no-op) / zero-write (no-op)
                    _ => {
                        let k = random_key(&mut rng);
                        if map.contains_key(&k) {
                            continue;
                        }
                        assert!(!tree.delete(&k));
                        assert!(!tree.write(&k, &ZERO_VALUE));
                    }
                }
            }
            assert_eq!(tree.root(), oracle::state_root(&map), "round {round}");
        }

        // Drain to empty.
        for k in live {
            tree.delete(&k);
            map.remove(&k);
        }
        assert_eq!(tree.root(), EMPTY_ROOT);
        assert_eq!(oracle::state_root(&map), EMPTY_ROOT);
    }

    #[test]
    fn zero_write_deletes() {
        let mut t = PbtTree::new();
        let mut rng = Rng(7);
        let k1 = random_key(&mut rng);
        let k2 = random_key(&mut rng);
        let v = random_value(&mut rng);
        t.write(&k1, &v);
        t.write(&k2, &v);
        let root_two = t.root();
        t.write(&k2, &ZERO_VALUE); // deletion
        let mut map = BTreeMap::new();
        map.insert(k1.clone(), v);
        assert_eq!(t.root(), oracle::state_root(&map));
        assert_ne!(t.root(), root_two);
    }

    /// After one leaf-value update, the dirty set is exactly the path:
    /// path branches + 1 leaf. No overcounting from cached-but-clean nodes.
    #[test]
    fn stats_count_exact_path() {
        let mut rng = Rng(99);
        let mut tree = PbtTree::new();
        let mut keys = Vec::new();
        for _ in 0..512 {
            let k = random_key(&mut rng);
            tree.insert(&k, &random_value(&mut rng));
            keys.push(k);
        }
        tree.root(); // flush caches

        // No-op write: same value again -> zero dirty nodes.
        let k = keys[0].clone();
        // Pin a known value, flush, then re-insert the same value.
        let v = random_value(&mut rng);
        tree.insert(&k, &v);
        tree.root();
        let changed = tree.insert(&k, &v);
        assert!(!changed);
        let mut s = RewriteStats::default();
        tree.root_with_stats(&mut s);
        assert_eq!(s.total_branch_writes() + s.total_leaf_writes(), 0);

        // Real update: dirty set = path branches + exactly one leaf, and the
        // leaf is a REWRITE (it existed at the previous commit), not a create.
        let v2 = random_value(&mut rng);
        tree.insert(&k, &v2);
        let mut s = RewriteStats::default();
        tree.root_with_stats(&mut s);
        assert_eq!(s.total_leaf_rewrites(), 1);
        assert_eq!(s.total_leaf_writes(), 1);
        assert!(s.total_branch_writes() >= 1);
        // Every dirtied branch lies on one root-to-leaf path: depths distinct.
        let mut depths: Vec<u32> = s
            .cells
            .iter()
            .filter(|((_, _), c)| c.branch_creates + c.branch_rewrites > 0)
            .map(|((_, d), _)| *d)
            .collect();
        depths.sort_unstable();
        depths.dedup();
        let n = depths.len() as u64;
        assert_eq!(
            s.total_branch_writes(),
            n,
            "one branch per depth on a single path"
        );
    }

    /// Within-block net-zero patterns are not counted: create-then-delete of
    /// a fresh key and set-then-restore of an existing value leave every
    /// surviving record byte-identical to the previous commit.
    #[test]
    fn stats_ignore_net_zero_patterns() {
        let mut rng = Rng(4242);
        let mut tree = PbtTree::new();
        let mut keys = Vec::new();
        for _ in 0..512 {
            let k = random_key(&mut rng);
            tree.insert(&k, &random_value(&mut rng));
            keys.push(k);
        }
        tree.root(); // commit

        // create-then-delete of a fresh key within one block.
        let fresh = random_key(&mut rng);
        tree.insert(&fresh, &random_value(&mut rng));
        tree.delete(&fresh);
        let mut s = RewriteStats::default();
        tree.root_with_stats(&mut s);
        assert_eq!(s.total_branch_writes(), 0, "no surviving record changed");
        assert_eq!(s.total_leaf_writes(), 0);
        // The churn IS visible as deletion events (leaf + no branch, since the
        // fresh leaf's split branch was also created and destroyed in-block).
        let deletes: u64 = s
            .cells
            .values()
            .map(|c| c.leaf_deletes + c.branch_deletes)
            .sum();
        assert!(deletes >= 1);

        // set-then-restore of an existing value within one block.
        let k = keys[0].clone();
        let orig = random_value(&mut rng);
        tree.insert(&k, &orig);
        tree.root(); // commit orig
        let tmp = random_value(&mut rng);
        tree.insert(&k, &tmp);
        tree.insert(&k, &orig); // restore
        let mut s = RewriteStats::default();
        tree.root_with_stats(&mut s);
        assert_eq!(s.total_branch_writes(), 0);
        assert_eq!(s.total_leaf_writes(), 0);
    }

    /// A brand-new key counts as a leaf CREATE plus one branch CREATE (the
    /// split), with rewrites confined to the pre-existing path branches.
    #[test]
    fn stats_split_create_vs_rewrite() {
        let mut rng = Rng(777);
        let mut tree = PbtTree::new();
        for _ in 0..512 {
            tree.insert(&random_key(&mut rng), &random_value(&mut rng));
        }
        tree.root();

        let fresh = random_key(&mut rng);
        tree.insert(&fresh, &random_value(&mut rng));
        let mut s = RewriteStats::default();
        tree.root_with_stats(&mut s);
        let leaf_creates: u64 = s.cells.values().map(|c| c.leaf_creates).sum();
        let branch_creates: u64 = s.cells.values().map(|c| c.branch_creates).sum();
        assert_eq!(leaf_creates, 1);
        assert_eq!(branch_creates, 1, "exactly the split-created branch");
        assert_eq!(s.total_leaf_rewrites(), 0, "no existing leaf changed");
    }

    /// Census totals match the entry count; top structure matches §2.3's
    /// prediction: exactly 2 Top branches once all three zones are populated.
    #[test]
    fn census_and_top_structure() {
        let mut rng = Rng(5);
        let mut tree = PbtTree::new();
        let mut n = 0u64;
        for _ in 0..3000 {
            let k = random_key(&mut rng);
            if tree.insert(&k, &random_value(&mut rng)) {
                n += 1;
            }
        }
        tree.root();
        let census = tree.census();
        let leaves: u64 = census.cells.values().map(|c| c.leaves).sum();
        assert_eq!(leaves, n);
        let branches: u64 = census.cells.values().map(|c| c.branches).sum();
        assert_eq!(branches, n - 1, "binary tree with two-child branches");

        let top = tree.top_structure();
        // 0x00 vs 0x01 vs 0xFF: root splits bit 0, left child (prefix 000000)
        // splits bit 7. Exactly two Top nodes.
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].node_depth, 0);
        assert_eq!(top[0].prefix_bits, 0);
        assert_eq!(top[1].node_depth, 1);
        assert_eq!(top[1].prefix_bits, 6);
    }
}
