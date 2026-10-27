//! Replay of mainnet state diffs on a tree whose untouched state is modeled.
//!
//! The diffs name the keys a range of blocks writes, a sliver of the whole
//! state. Inserted alone they would make a tree far shallower than
//! mainnet's, and every path-length-dependent cost would come out low. The
//! full state cannot be loaded, so the tree gets its depth another way:
//! beside every written path, wherever the full state has keys on the
//! other side of a branch, one STAND-IN leaf is inserted. It makes the
//! branch exist, it is never written again, and the subtree it stands for
//! is never looked into. Where a written path ENDS beside untouched state,
//! that neighbour gets two leaves when it is a subtree in the full state,
//! so that creating or deleting the written key moves a branch, as it
//! would there, and not a leaf. The reference tree is used unchanged; its
//! root is a genuine PBT root over the written keys plus the stand-ins.
//!
//! Where the full state has keys is not known key by key. It is known in
//! aggregate, and EIP-8297 places keys by hash, so within a region the
//! untouched keys are spread uniformly:
//!
//! - account zone: `key_hash(address)` over all accounts;
//! - storage zone, first 256 bits: `key_hash(address)` over all contracts
//!   that have overflow storage;
//! - storage zone, next 256 bits: `key_hash(address || tree_index)` over
//!   that one contract's storage groups.
//!
//! Each region's count is split between the two children of every prefix by
//! a coin-flip that depends only on the prefix, so every path through a
//! prefix agrees on what lies beside it.
//!
//! ASSUMPTIONS, all reported: a contract's untouched slots sit as many to a
//! stem as the slots the sample writes to it do; a contract without a
//! fetched slot count has as many untouched stems as written ones; an
//! untouched contract holds the mean number of slots; a written stem and a
//! written account's header hold nothing but what the sample writes; code
//! and delegations are not replayed.

use crate::bench::Workload;
use crate::gen::Write;
use pbt_ref::bits::bit;
use pbt_ref::hash::{h, key_hash, Digest, ZERO_VALUE};
use pbt_ref::keys::*;
use pbt_store::mrc::id64;
use pbt_store::shape::StateShape;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::sync::Arc;

/// Size of the full state the replay stands in for.
#[derive(Clone, Copy)]
pub struct Scale {
    pub accounts: u64,
    pub storage_slots: u64,
    /// Contracts with at least one slot in the storage zone. ASSUMED: the
    /// state-size source counts slots, not the contracts that hold them.
    pub storage_contracts: u64,
    pub code_bytes: u64,
    /// Slots an untouched contract holds. None: the mean. Real sizes are
    /// heavy-tailed, so most contracts hold far fewer; a small value here
    /// is the other extreme.
    pub neighbor_slots: Option<u64>,
}

impl Scale {
    pub fn code_chunks(&self) -> u64 {
        self.code_bytes / 31
    }
    /// Leaves of the full tree: two header leaves per account (basic data
    /// and code hash), one per storage slot, one per code chunk.
    pub fn leaves(&self) -> u64 {
        self.accounts * 2 + self.storage_slots + self.code_chunks()
    }
    pub fn neighbor_slots(&self) -> u64 {
        self.neighbor_slots
            .unwrap_or(self.storage_slots / self.storage_contracts.max(1))
    }
    /// Value-store leaf pages of the full state at typical fill.
    pub fn value_leaf_pages(&self) -> u64 {
        ((self.accounts * 2 + self.code_chunks()) as f64 / VALUE_LEAF_FILL_SHORT
            + self.storage_slots as f64 / VALUE_LEAF_FILL_LONG) as u64
    }
}

const REGION_ACCOUNTS: u8 = b'A';
const REGION_CONTRACTS: u8 = b'S';
const REGION_GROUPS: u8 = b'G';

/// Deterministic seed for one prefix of one region.
fn seed(tag: u8, region: &[u8], path: &[u8], nbits: usize) -> Digest {
    let full = nbits / 8;
    let mut buf = Vec::with_capacity(4 + region.len() + full + 1);
    buf.push(tag);
    buf.extend_from_slice(region);
    buf.extend_from_slice(&(nbits as u16).to_be_bytes());
    buf.extend_from_slice(&path[..full]);
    if nbits % 8 != 0 {
        buf.push(path[full] & (0xFFu8 << (8 - nbits % 8)));
    }
    h(&buf)
}

/// Split `c` untouched keys under a prefix between its 0-child and its
/// 1-child. Binomial(c, 1/2): exact for small counts, normal for large.
fn split(s: &Digest, c: u64) -> (u64, u64) {
    if c == 0 {
        return (0, 0);
    }
    if c <= 256 {
        let ones: u64 = (0..c as usize)
            .map(|i| ((s[i / 8] >> (i % 8)) & 1) as u64)
            .sum();
        return (c - ones, ones);
    }
    let unit = |b: &[u8]| {
        ((u64::from_le_bytes(b.try_into().unwrap()) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    let (u1, u2) = (unit(&s[0..8]), unit(&s[8..16]));
    let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    let left = (c as f64 / 2.0 + z * (c as f64).sqrt() / 2.0)
        .round()
        .clamp(0.0, c as f64) as u64;
    (left, c - left)
}

// The splits define, implicitly and exactly, one particular set of `n`
// untouched keys per region: a count that reaches 1 under some prefix is one
// key, whose remaining bits are a fixed pseudo-random tail of that prefix.
// Everything below asks questions of that one set, so the answers agree:
// how many of its keys lie under a prefix, and which of its keys to insert
// as the stand-in for a subtree. A stand-in is therefore always a MEMBER of
// the set, never an arbitrary key in the right subtree. The difference
// matters: an arbitrary key runs alongside other written paths further
// than any real neighbour does and gives them branches they do not have.

/// The one untouched key under the first `nbits` of `path`, when the count
/// there is 1.
fn lone_key(tag: u8, region: &[u8], path: &[u8], nbits: usize) -> [u8; 32] {
    let mut k = h(&[&seed(tag, region, path, nbits)[..], b"lone"].concat());
    let full = nbits / 8;
    k[..full].copy_from_slice(&path[..full]);
    if nbits % 8 != 0 {
        let mask = 0xFFu8 << (8 - nbits % 8);
        k[full] = (path[full] & mask) | (k[full] & !mask);
    }
    k
}

/// Untouched keys under the first `nbits` of `path` in a region of `n`.
fn count_under(tag: u8, region: &[u8], n: u64, path: &[u8], nbits: usize) -> u64 {
    let mut c = n;
    for i in 0..nbits {
        match c {
            0 => return 0,
            1 => {
                let lone = lone_key(tag, region, path, i);
                return (i..nbits).all(|j| bit(&lone, j) == bit(path, j)) as u64;
            }
            _ => {
                let (c0, c1) = split(&seed(tag, region, path, i), c);
                c = if bit(path, i) == 0 { c0 } else { c1 };
            }
        }
    }
    c
}

/// `count_under` for every prefix length `0..=nbits` of `path`, in one
/// walk: `out[i]` is the count under the first `i` bits.
fn count_along(tag: u8, region: &[u8], n: u64, path: &[u8], nbits: usize) -> Vec<u64> {
    let mut out = Vec::with_capacity(nbits + 1);
    let mut c = n;
    for i in 0..nbits {
        out.push(c);
        match c {
            0 => {
                out.resize(nbits + 1, 0);
                return out;
            }
            1 => {
                let lone = lone_key(tag, region, path, i);
                let mut ok = true;
                for j in i..nbits {
                    ok = ok && bit(&lone, j) == bit(path, j);
                    out.push(ok as u64);
                }
                return out;
            }
            _ => {
                let (c0, c1) = split(&seed(tag, region, path, i), c);
                c = if bit(path, i) == 0 { c0 } else { c1 };
            }
        }
    }
    out.push(c);
    out
}

/// The leftmost untouched key under the first `nbits` of `prefix`, given
/// that `c >= 1` of them are there.
fn leftmost(
    tag: u8,
    region: &[u8],
    mut prefix: [u8; 32],
    mut nbits: usize,
    mut c: u64,
) -> [u8; 32] {
    while c > 1 && nbits < 256 {
        let (c0, c1) = split(&seed(tag, region, &prefix, nbits), c);
        let mask = 0x80u8 >> (nbits % 8);
        if c0 > 0 {
            prefix[nbits / 8] &= !mask;
            c = c0;
        } else {
            prefix[nbits / 8] |= mask;
            c = c1;
        }
        nbits += 1;
    }
    lone_key(tag, region, &prefix, nbits)
}

/// With `leftmost`, the second key that gives the subtree of `c >= 2`
/// untouched keys under a prefix its top branch: the first key on the far
/// side of the first level at which those keys part ways.
fn fork_mate(
    tag: u8,
    region: &[u8],
    mut prefix: [u8; 32],
    mut nbits: usize,
    c: u64,
) -> Option<[u8; 32]> {
    while c >= 2 && nbits < 256 {
        let (c0, c1) = split(&seed(tag, region, &prefix, nbits), c);
        let mask = 0x80u8 >> (nbits % 8);
        if c0 > 0 && c1 > 0 {
            prefix[nbits / 8] |= mask;
            return Some(leftmost(tag, region, prefix, nbits + 1, c1));
        }
        if c0 == 0 {
            prefix[nbits / 8] |= mask;
        } else {
            prefix[nbits / 8] &= !mask;
        }
        nbits += 1;
    }
    None
}

/// What to insert beside a 256-bit path so that it has every branch it has
/// in the full tree and no others.
struct Beside {
    /// For every level at which untouched keys sit on the other side, the
    /// leftmost of them. Where the path runs out of untouched company with
    /// two or more keys on the other side, those keys are a subtree, and a
    /// second key of it follows so that it is a branch here too.
    keys: Vec<[u8; 32]>,
    /// The one untouched key the path ends beside, when it ends beside one
    /// key. Whether that key is a leaf or a subtree of its own in the full
    /// state depends on the region, so the caller decides how to insert it.
    lone: Option<[u8; 32]>,
}

fn siblings(tag: u8, region: &[u8], n: u64, path: &[u8; 32]) -> Beside {
    let mut keys = Vec::new();
    let mut c = n;
    for i in 0..256 {
        match c {
            0 => break,
            1 => {
                // One untouched key shares this prefix: it leaves the path
                // somewhere below, and is its own stand-in.
                let lone = lone_key(tag, region, path, i);
                return Beside {
                    keys,
                    lone: (lone != *path).then_some(lone),
                };
            }
            _ => {}
        }
        let (c0, c1) = split(&seed(tag, region, path, i), c);
        let b = bit(path, i);
        let (on, off) = if b == 0 { (c0, c1) } else { (c1, c0) };
        if off > 0 {
            let mut other = *path;
            other[i / 8] ^= 0x80 >> (i % 8);
            keys.push(leftmost(tag, region, other, i + 1, off));
            if on == 0 {
                keys.extend(fork_mate(tag, region, other, i + 1, off));
            }
        }
        c = on;
    }
    Beside { keys, lone: None }
}

/// Identity of everything under the first `nbits` of `data`, tagged.
fn prefix_id(tag: &[u8], data: &[u8], nbits: usize) -> u64 {
    let nbits = nbits.min(data.len() * 8);
    let full = nbits / 8;
    let mut buf = Vec::with_capacity(tag.len() + 2 + full + 1);
    buf.extend_from_slice(tag);
    buf.extend_from_slice(&(nbits as u16).to_be_bytes());
    buf.extend_from_slice(&data[..full]);
    if nbits % 8 != 0 {
        buf.push(data[full] & (0xFFu8 << (8 - nbits % 8)));
    }
    id64(&buf)
}

fn log2_round(x: f64) -> usize {
    if x <= 1.0 {
        0
    } else {
        x.log2().round() as usize
    }
}

/// Entries a value-store leaf page holds at typical fill, about 0.7 of its
/// capacity: 62 entries of a 34-byte key and a 32-byte value fit a page, 41
/// of a 66-byte storage key. And leaf pages under one bottom branch node.
pub const VALUE_LEAF_FILL_SHORT: f64 = 43.0;
pub const VALUE_LEAF_FILL_LONG: f64 = 29.0;
pub const VALUE_BBN_FILL: f64 = 357.0;

/// One written contract's storage as the replay models it.
struct Contract {
    /// Stems of the contract the sample never writes.
    untouched_stems: u64,
    /// Slots per stem, from the slots and stems the sample writes.
    slots_per_stem: f64,
    /// Every storage key the sample writes to it, as group hash and
    /// sub-index, sorted.
    written: Vec<[u8; 33]>,
}

impl Contract {
    fn leaves(&self) -> u64 {
        (self.untouched_stems as f64 * self.slots_per_stem) as u64 + self.written.len() as u64
    }
    /// Written keys under the first `nbits` of `tail` (group and sub-index).
    fn written_under(&self, tail: &[u8], nbits: usize) -> u64 {
        let nbits = nbits.min(264);
        let (mut lo, mut hi) = ([0u8; 33], [0xFFu8; 33]);
        let full = nbits / 8;
        lo[..full].copy_from_slice(&tail[..full]);
        hi[..full].copy_from_slice(&tail[..full]);
        if nbits % 8 != 0 {
            let mask = 0xFFu8 << (8 - nbits % 8);
            lo[full] = tail[full] & mask;
            hi[full] = (tail[full] & mask) | !mask;
        }
        (self.written.partition_point(|k| k <= &hi) - self.written.partition_point(|k| k < &lo))
            as u64
    }
}

pub struct MainnetShape {
    scale: Scale,
    /// key_hash(address) -> model, for every contract the replay writes.
    contracts: HashMap<[u8; 32], Contract>,
    /// The same contracts sorted by hash, with running totals of leaves.
    sorted: Vec<[u8; 32]>,
    running: Vec<u64>,
}

impl MainnetShape {
    fn new(scale: Scale, contracts: HashMap<[u8; 32], Contract>) -> Self {
        let mut sorted: Vec<[u8; 32]> = contracts.keys().copied().collect();
        sorted.sort_unstable();
        let mut running = Vec::with_capacity(sorted.len() + 1);
        running.push(0u64);
        for k in &sorted {
            running.push(running.last().unwrap() + contracts[k].leaves());
        }
        MainnetShape {
            scale,
            contracts,
            sorted,
            running,
        }
    }

    fn contract(&self, contract_hash: &[u8]) -> Option<&Contract> {
        let k: [u8; 32] = contract_hash.try_into().expect("32-byte hash");
        self.contracts.get(&k)
    }

    fn slots_of(&self, contract_hash: &[u8]) -> u64 {
        self.contract(contract_hash).map_or(0, Contract::leaves)
    }

    fn untouched_stems(&self, contract_hash: &[u8]) -> u64 {
        self.contract(contract_hash)
            .map_or(0, |c| c.untouched_stems)
    }

    /// Leaves of one zone, as `keys_under` counts them at the zone byte.
    fn zone_total(&self, zone: u8) -> u64 {
        match zone {
            ACCOUNT_ZONE => self.scale.accounts * 2,
            CODE_ZONE => self.scale.code_chunks(),
            STORAGE_ZONE => {
                self.scale.storage_contracts * self.scale.neighbor_slots()
                    + self.running.last().copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Slots of written contracts whose hash starts with `nbits` of `path`.
    fn written_slots_under(&self, path: &[u8], nbits: usize) -> u64 {
        let nbits = nbits.min(256);
        let (mut lo, mut hi) = ([0u8; 32], [0xFFu8; 32]);
        let full = nbits / 8;
        lo[..full].copy_from_slice(&path[..full]);
        hi[..full].copy_from_slice(&path[..full]);
        if nbits % 8 != 0 {
            let mask = 0xFFu8 << (8 - nbits % 8);
            lo[full] = path[full] & mask;
            hi[full] = (path[full] & mask) | !mask;
        }
        let a = self.sorted.partition_point(|k| k < &lo);
        let b = self.sorted.partition_point(|k| k <= &hi);
        self.running[b] - self.running[a]
    }
}

impl StateShape for MainnetShape {
    fn keys_under(&self, path: &[u8]) -> u64 {
        let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
        let body = &path[2..];
        if nbits < 8 {
            return self.scale.leaves();
        }
        let rest = nbits - 8;
        match body[0] {
            ACCOUNT_ZONE => {
                let stems = count_under(
                    REGION_ACCOUNTS,
                    &[],
                    self.scale.accounts,
                    &body[1..],
                    rest.min(256),
                );
                if rest > 256 {
                    stems.min(1) // inside one stem: only the run's own leaves
                } else {
                    stems * 2
                }
            }
            STORAGE_ZONE if rest <= 256 => {
                let others = count_under(
                    REGION_CONTRACTS,
                    &[],
                    self.scale.storage_contracts,
                    &body[1..],
                    rest,
                );
                others * self.scale.neighbor_slots() + self.written_slots_under(&body[1..], rest)
            }
            STORAGE_ZONE => {
                let Some(c) = self.contract(&body[1..33]) else {
                    return 0;
                };
                let g = rest - 256;
                let stems = count_under(
                    REGION_GROUPS,
                    &body[1..33],
                    c.untouched_stems,
                    &body[33..],
                    g.min(256),
                );
                let untouched = if g > 256 {
                    stems.min(1)
                } else {
                    (stems as f64 * c.slots_per_stem) as u64
                };
                untouched + c.written_under(&body[33..], g)
            }
            CODE_ZONE => self.scale.code_chunks() >> rest.min(63),
            _ => 0,
        }
    }

    /// `keys_under` at every prefix length, one walk per region instead of
    /// one per prefix. Agrees with `keys_under` from the zone byte down;
    /// above it, the zones whose byte starts with the prefix are summed,
    /// which puts the zones in key order (`keys_under` answers the whole
    /// state there).
    fn keys_along(&self, path: &[u8]) -> Vec<u64> {
        let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
        let body = &path[2..];
        let mut out = Vec::with_capacity(nbits + 1);
        let zone = body.first().copied().unwrap_or(0);
        for i in 0..=nbits.min(8) {
            let mask = if i == 0 { 0 } else { 0xFFu8 << (8 - i) };
            out.push(
                [ACCOUNT_ZONE, CODE_ZONE, STORAGE_ZONE]
                    .iter()
                    .filter(|z| **z & mask == zone & mask)
                    .map(|z| self.zone_total(*z))
                    .sum(),
            );
        }
        if nbits <= 8 {
            return out;
        }
        let rest = nbits - 8;
        match body[0] {
            ACCOUNT_ZONE => {
                let stems = count_along(
                    REGION_ACCOUNTS,
                    &[],
                    self.scale.accounts,
                    &body[1..],
                    rest.min(256),
                );
                for r in 1..=rest {
                    out.push(if r > 256 {
                        stems[256].min(1)
                    } else {
                        stems[r] * 2
                    });
                }
            }
            STORAGE_ZONE => {
                let top = rest.min(256);
                let others = count_along(
                    REGION_CONTRACTS,
                    &[],
                    self.scale.storage_contracts,
                    &body[1..],
                    top,
                );
                for r in 1..=top {
                    out.push(
                        others[r] * self.scale.neighbor_slots()
                            + self.written_slots_under(&body[1..], r),
                    );
                }
                if rest > 256 {
                    match self.contract(&body[1..33]) {
                        None => out.resize(nbits + 1, 0),
                        Some(c) => {
                            let stems = count_along(
                                REGION_GROUPS,
                                &body[1..33],
                                c.untouched_stems,
                                &body[33..],
                                (rest - 256).min(256),
                            );
                            for g in 1..=(rest - 256) {
                                let untouched = if g > 256 {
                                    stems[256].min(1)
                                } else {
                                    (stems[g] as f64 * c.slots_per_stem) as u64
                                };
                                out.push(untouched + c.written_under(&body[33..], g));
                            }
                        }
                    }
                }
            }
            CODE_ZONE => {
                for r in 1..=rest {
                    out.push(self.scale.code_chunks() >> r.min(63));
                }
            }
            _ => out.resize(nbits + 1, 0),
        }
        out
    }

    fn value_leaf_page(&self, key: &[u8]) -> u64 {
        match key[0] {
            ACCOUNT_ZONE => {
                let bits = log2_round(self.scale.accounts as f64 * 2.0 / VALUE_LEAF_FILL_SHORT);
                prefix_id(b"vl-a", &key[1..33], bits)
            }
            STORAGE_ZONE if key.len() == STORAGE_KEY_LENGTH => {
                let n = self.slots_of(&key[1..33]) as f64;
                if n >= 1.5 * VALUE_LEAF_FILL_LONG {
                    let mut tag = b"vl-g".to_vec();
                    tag.extend_from_slice(&key[1..33]);
                    prefix_id(&tag, &key[33..65], log2_round(n / VALUE_LEAF_FILL_LONG))
                } else {
                    // A small contract's slots are adjacent and fit one page.
                    prefix_id(b"vl-c", &key[1..33], 256)
                }
            }
            _ => prefix_id(b"vl-x", key, key.len() * 8),
        }
    }

    fn value_bbn(&self, key: &[u8]) -> u64 {
        match key[0] {
            ACCOUNT_ZONE => {
                let bits = log2_round(
                    self.scale.accounts as f64 * 2.0 / VALUE_LEAF_FILL_SHORT / VALUE_BBN_FILL,
                );
                prefix_id(b"bbn-a", &key[1..33], bits)
            }
            STORAGE_ZONE if key.len() == STORAGE_KEY_LENGTH => {
                let n = self.slots_of(&key[1..33]) as f64;
                if n >= 1.5 * VALUE_LEAF_FILL_LONG * VALUE_BBN_FILL {
                    let mut tag = b"bbn-g".to_vec();
                    tag.extend_from_slice(&key[1..33]);
                    prefix_id(
                        &tag,
                        &key[33..65],
                        log2_round(n / VALUE_LEAF_FILL_LONG / VALUE_BBN_FILL),
                    )
                } else {
                    let bits = log2_round(
                        self.scale.storage_slots as f64 / VALUE_LEAF_FILL_LONG / VALUE_BBN_FILL,
                    );
                    prefix_id(b"bbn-c", &key[1..33], bits)
                }
            }
            _ => prefix_id(b"bbn-x", key, 16),
        }
    }
}

/// What the replay was built from, for the report.
pub struct ReplayFacts {
    pub first_block: u64,
    pub blocks: usize,
    pub account_changes: u64,
    pub slot_changes: u64,
    pub genesis_keys: u64,
    pub stand_ins_accounts: u64,
    pub stand_ins_contracts: u64,
    pub stand_ins_groups: u64,
    /// Contracts with a write in the storage zone, and how many of them
    /// have a fetched slot count.
    pub contracts: usize,
    pub contracts_with_count: usize,
    /// Share of storage-zone changes that fall in contracts with a fetched count.
    pub slot_changes_with_count: f64,
    /// Distinct storage-zone slots the sample writes, and the stems they sit in.
    pub written_slots: u64,
    pub written_stems: u64,
    pub account_creations: u64,
    /// Execution reads of keys the block does not change: existing, absent.
    pub reads_present: u64,
    pub reads_absent: u64,
    /// The replay covers one key-space subtree in `2^sample_bits`.
    pub sample_bits: u32,
    pub sample_part: u8,
}

/// One execution read: a key the block reads but does not change, and
/// whether it exists.
pub type StateRead = (Vec<u8>, bool);

/// One change of the diffs file, as the tree sees it.
struct Change {
    kind: u8,
    key: Vec<u8>,
    /// For an account: its code-hash leaf, created and deleted with it.
    code_key: Option<Vec<u8>>,
}

enum Item {
    Block(u64),
    Change(Change),
    Read(StateRead),
}

/// The diffs file, line by line, with the first `skip` blocks left out.
/// With `sample_bits` > 0 only keys whose address hash starts with that
/// many bits equal to `sample_part` are kept: one subtree in `2^sample_bits` of each zone,
/// with an account and its storage always kept together.
struct DiffReader {
    lines: std::io::Lines<BufReader<std::fs::File>>,
    skip: usize,
    started: bool,
    with_reads: bool,
    sample_bits: u32,
    /// Which subtree: the address hash's top `sample_bits` bits.
    sample_part: u8,
}

impl DiffReader {
    fn open(path: &str, skip: usize, with_reads: bool, sample: (u32, u8)) -> Self {
        let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
        DiffReader {
            lines: BufReader::new(f).lines(),
            skip,
            started: false,
            with_reads,
            sample_bits: sample.0,
            sample_part: sample.1,
        }
    }

    fn kept(&self, key: &[u8]) -> bool {
        self.sample_bits == 0 || key[1] >> (8 - self.sample_bits) == self.sample_part
    }
}

impl Iterator for DiffReader {
    type Item = Item;
    fn next(&mut self) -> Option<Item> {
        loop {
            let line = self.lines.next()?.expect("read diffs");
            let mut it = line.split(' ');
            let item = match it.next() {
                Some("B") => {
                    if self.skip > 0 {
                        self.skip -= 1;
                        continue;
                    }
                    self.started = true;
                    Item::Block(it.next().unwrap().parse().expect("block"))
                }
                _ if !self.started => continue,
                Some("A") => {
                    let kind = it.next().unwrap().as_bytes()[0];
                    let a = address32(it.next().unwrap());
                    Item::Change(Change {
                        kind,
                        key: get_tree_key_for_basic_data(&a),
                        code_key: Some(get_tree_key_for_code_hash(&a)),
                    })
                }
                Some("S") => {
                    let kind = it.next().unwrap().as_bytes()[0];
                    let a = address32(it.next().unwrap());
                    let slot: StorageSlot =
                        unhex(it.next().unwrap()).try_into().expect("32-byte slot");
                    Item::Change(Change {
                        kind,
                        key: get_tree_key_for_storage_slot(&a, &slot),
                        code_key: None,
                    })
                }
                Some(tag @ ("R" | "Z")) if self.with_reads => {
                    let zone = it.next().unwrap();
                    let a = address32(it.next().unwrap());
                    let key = if zone == "A" {
                        get_tree_key_for_basic_data(&a)
                    } else {
                        let slot: StorageSlot =
                            unhex(it.next().unwrap()).try_into().expect("32-byte slot");
                        get_tree_key_for_storage_slot(&a, &slot)
                    };
                    Item::Read((key, tag == "R"))
                }
                _ => continue,
            };
            match &item {
                Item::Change(c) if !self.kept(&c.key) => continue,
                Item::Read((k, _)) if !self.kept(k) => continue,
                _ => {}
            }
            return Some(item);
        }
    }
}

/// The replay reads its blocks from the file as it goes, so memory does
/// not grow with the number of blocks.
pub struct Replay {
    desc: String,
    genesis: Vec<Vec<Write>>,
    reader: DiffReader,
    /// The next block's number, read ahead from its `B` line.
    next: Option<u64>,
    reads: Vec<StateRead>,
}

impl Replay {
    fn next_block(&mut self) -> Vec<Write> {
        self.reads.clear();
        let Some(number) = self.next.take() else {
            return Vec::new();
        };
        let mut w = Vec::new();
        for item in self.reader.by_ref() {
            match item {
                Item::Block(n) => {
                    self.next = Some(n);
                    break;
                }
                Item::Read(r) => self.reads.push(r),
                Item::Change(c) => match c.kind {
                    b'd' => {
                        if let Some(ck) = c.code_key {
                            w.push((ck, ZERO_VALUE));
                        }
                        w.push((c.key, ZERO_VALUE));
                    }
                    kind => {
                        if kind == b'c' {
                            if let Some(ck) = c.code_key {
                                let v = value(&ck, number);
                                w.push((ck, v));
                            }
                        }
                        let v = value(&c.key, number);
                        w.push((c.key, v));
                    }
                },
            }
        }
        w
    }
}

impl Workload for Replay {
    fn describe(&self) -> String {
        self.desc.clone()
    }
    fn genesis(&mut self) -> Vec<Vec<Write>> {
        std::mem::take(&mut self.genesis)
    }
    fn block(&mut self) -> Vec<Write> {
        self.next_block()
    }
    fn block_reads(&mut self) -> Vec<StateRead> {
        std::mem::take(&mut self.reads)
    }
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn address32(hex40: &str) -> Address32 {
    let a: [u8; 20] = unhex(hex40).try_into().expect("20-byte address");
    address20_to_address32(&a)
}

/// A nonzero value that differs for every (key, salt).
fn value(key: &[u8], salt: u64) -> Digest {
    let mut v = h(&[key, &salt.to_le_bytes()].concat());
    v[31] |= 1;
    v
}

fn read_csv(path: &str) -> Vec<Vec<String>> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    BufReader::new(f)
        .lines()
        .map(|l| l.expect("read"))
        .filter(|l| !l.starts_with('#') && !l.starts_with("address"))
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect()
}

pub struct Loaded {
    pub replay: Replay,
    pub shape: Arc<MainnetShape>,
    pub facts: ReplayFacts,
}

/// Build the replay: parse the diffs, size each written contract, place
/// the stand-ins, and split the starting state into commit-sized batches.
/// The first `skip` blocks of the file are left out, as if it began later.
/// With `with_reads`, the file's execution reads (R and Z lines) are kept
/// for the blocks they belong to; they do not enter the tree.
pub fn load(
    diffs: &str,
    slot_counts_csv: &str,
    scale: Scale,
    genesis_batch: usize,
    skip: usize,
    with_reads: bool,
    sample_bits: u32,
    sample_part: u8,
) -> Loaded {
    assert!(sample_bits <= 4, "sample at most 1/16 of the key space");
    assert!(
        (sample_part as u32) < (1 << sample_bits),
        "sample part out of range"
    );
    let fetched: HashMap<[u8; 32], u64> = read_csv(slot_counts_csv)
        .into_iter()
        .map(|r| {
            (
                key_hash(&address32(&r[0])),
                r[1].parse::<u64>().expect("active_slots"),
            )
        })
        .filter(|(_, n)| *n > 0)
        .collect();

    // Starting state: every key the sample first meets as a rewrite or a
    // deletion existed before it. A header slot implies its account.
    // One pass over the file; the blocks themselves are read again, one at
    // a time, while the replay runs.
    #[derive(Default)]
    struct Seen {
        changes: u64,
        written: Vec<[u8; 33]>,
        stems: HashSet<[u8; 32]>,
        /// Stems holding a key that existed before the sample.
        old_stems: HashSet<[u8; 32]>,
    }
    let mut first_seen: HashSet<u64> = HashSet::new();
    let mut genesis_keys: Vec<Vec<u8>> = Vec::new();
    let mut created_accounts: HashSet<[u8; 32]> = HashSet::new(); // by header hash
    let mut header_hashes: HashSet<[u8; 32]> = HashSet::new();
    let mut account_hashes: HashSet<[u8; 32]> = HashSet::new();
    let mut seen: HashMap<[u8; 32], Seen> = HashMap::new();
    let (mut reads_present, mut reads_absent) = (0u64, 0u64);
    let (mut account_changes, mut slot_changes, mut account_creations) = (0u64, 0u64, 0u64);
    let (mut first_block, mut n_blocks) = (None, 0usize);
    for item in DiffReader::open(diffs, skip, with_reads, (sample_bits, sample_part)) {
        let c = match item {
            Item::Block(n) => {
                first_block.get_or_insert(n);
                n_blocks += 1;
                continue;
            }
            Item::Read((_, present)) => {
                if present {
                    reads_present += 1;
                } else {
                    reads_absent += 1;
                }
                continue;
            }
            Item::Change(c) => c,
        };
        if c.code_key.is_some() {
            account_changes += 1;
            account_creations += (c.kind == b'c') as u64;
        } else {
            slot_changes += 1;
        }
        let stem: [u8; 32] = c.key[1..33].try_into().unwrap();
        let fresh = first_seen.insert(id64(&c.key));
        if c.key.len() == ACCOUNT_KEY_LENGTH {
            account_hashes.insert(stem);
            if c.code_key.is_none() {
                header_hashes.insert(stem); // a header storage slot
            }
        } else {
            let group: [u8; 32] = c.key[33..65].try_into().unwrap();
            let e = seen.entry(stem).or_default();
            e.changes += 1;
            e.stems.insert(group);
            if fresh {
                e.written.push(c.key[33..66].try_into().unwrap());
                if c.kind != b'c' {
                    e.old_stems.insert(group);
                }
            }
        }
        if fresh {
            if c.kind == b'c' {
                if c.code_key.is_some() {
                    created_accounts.insert(stem);
                }
            } else {
                if let Some(ck) = &c.code_key {
                    genesis_keys.push(ck.clone());
                    first_seen.insert(id64(ck));
                }
                genesis_keys.push(c.key);
            }
        }
    }
    let first_block = first_block.unwrap_or(0);
    // Accounts known only through a header slot: their header leaves exist.
    for stem in &header_hashes {
        if created_accounts.contains(stem) {
            continue;
        }
        for sub in [BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY] {
            let k = get_tree_key(ACCOUNT_ZONE, stem, sub);
            if first_seen.insert(id64(&k)) {
                genesis_keys.push(k);
            }
        }
    }

    // Contract sizes. The fetched figure counts slots; the tree branches on
    // stems, and the sample shows how many slots this contract puts in one.
    // What the sample writes is inserted for real, so the modeled part is
    // only the stems it never touches.
    let total_changes: u64 = seen.values().map(|e| e.changes).sum();
    let covered: u64 = seen
        .iter()
        .filter(|(k, _)| fetched.contains_key(*k))
        .map(|(_, e)| e.changes)
        .sum();
    let (written_slots, written_stems) = seen.values().fold((0u64, 0u64), |(a, b), e| {
        (a + e.written.len() as u64, b + e.stems.len() as u64)
    });
    let n_contracts = seen.len();
    let contracts_with_count = seen.keys().filter(|k| fetched.contains_key(*k)).count();
    let mut group_hashes: Vec<([u8; 32], [u8; 32])> = Vec::new();
    let contracts: HashMap<[u8; 32], Contract> = seen
        .into_iter()
        .map(|(k, mut e)| {
            let slots_per_stem = e.written.len() as f64 / e.stems.len().max(1) as f64;
            let untouched_stems = match fetched.get(&k) {
                Some(n) => ((*n as f64 / slots_per_stem).round() as u64)
                    .saturating_sub(e.old_stems.len() as u64),
                None => e.stems.len() as u64,
            };
            group_hashes.extend(e.stems.iter().map(|g| (k, *g)));
            e.written.sort_unstable();
            (
                k,
                Contract {
                    untouched_stems,
                    slots_per_stem,
                    written: e.written,
                },
            )
        })
        .collect();
    group_hashes.sort_unstable();
    let contract_hashes: Vec<[u8; 32]> = {
        let mut v: Vec<[u8; 32]> = contracts.keys().copied().collect();
        v.sort_unstable();
        v
    };
    let shape = Arc::new(MainnetShape::new(scale, contracts));

    // Stand-ins beside every written path, in each region it runs through.
    let mut placed: HashSet<u64> = HashSet::new();
    let mut stand_ins: Vec<Vec<u8>> = Vec::new();
    let mut place = |k: Vec<u8>, n: &mut u64| {
        if placed.insert(id64(&k)) {
            stand_ins.push(k);
            *n += 1;
        }
    };
    let (mut n_a, mut n_s, mut n_g) = (0u64, 0u64, 0u64);
    for stem in &account_hashes {
        let b = siblings(REGION_ACCOUNTS, &[], scale.accounts, stem);
        for f in &b.keys {
            place(get_tree_key(ACCOUNT_ZONE, f, BASIC_DATA_LEAF_KEY), &mut n_a);
        }
        // An untouched account is a stem of two header leaves: a branch.
        if let Some(f) = &b.lone {
            place(get_tree_key(ACCOUNT_ZONE, f, BASIC_DATA_LEAF_KEY), &mut n_a);
            place(get_tree_key(ACCOUNT_ZONE, f, CODE_HASH_LEAF_KEY), &mut n_a);
        }
    }
    let slot_of = |contract: &[u8; 32], second: bool| {
        let mut group = h(&[&contract[..], b"group"].concat());
        if second {
            group[0] ^= 0x80; // parts from the first at the top of the stem hash
        }
        let mut pos = contract.to_vec();
        pos.extend_from_slice(&group);
        get_tree_key(STORAGE_ZONE, &pos, 0)
    };
    for stem in &contract_hashes {
        let b = siblings(REGION_CONTRACTS, &[], scale.storage_contracts, stem);
        for f in &b.keys {
            place(slot_of(f, false), &mut n_s);
        }
        // An untouched contract is a subtree of slots unless it holds one.
        if let Some(f) = &b.lone {
            place(slot_of(f, false), &mut n_s);
            if scale.neighbor_slots() >= 2 {
                place(slot_of(f, true), &mut n_s);
            }
        }
    }
    for (stem, group) in &group_hashes {
        let b = siblings(REGION_GROUPS, stem, shape.untouched_stems(stem), group);
        // An untouched stem is modeled as one leaf.
        for f in b.keys.iter().chain(&b.lone) {
            let mut pos = stem.to_vec();
            pos.extend_from_slice(f);
            place(get_tree_key(STORAGE_ZONE, &pos, 0), &mut n_g);
        }
    }
    // One key so the code zone, which is not replayed, exists at all.
    stand_ins.push(get_tree_key(CODE_ZONE, &h(b"code zone stand-in"), 0));

    let n_genesis = genesis_keys.len() as u64;
    let mut all: Vec<Write> = stand_ins
        .into_iter()
        .chain(genesis_keys)
        .map(|k| {
            let v = value(&k, 0);
            (k, v)
        })
        .collect();
    // Hash order: batches then spread over the tree instead of each
    // rewriting one corner of it.
    all.sort_unstable_by_key(|(k, _)| id64(k));
    let genesis: Vec<Vec<Write>> = all
        .chunks(genesis_batch.max(1))
        .map(<[Write]>::to_vec)
        .collect();

    let facts = ReplayFacts {
        first_block,
        blocks: n_blocks,
        account_changes,
        slot_changes,
        genesis_keys: n_genesis,
        stand_ins_accounts: n_a,
        stand_ins_contracts: n_s,
        stand_ins_groups: n_g,
        contracts: n_contracts,
        contracts_with_count,
        slot_changes_with_count: if total_changes > 0 {
            covered as f64 / total_changes as f64
        } else {
            0.0
        },
        written_slots,
        written_stems,
        account_creations,
        reads_present,
        reads_absent,
        sample_bits,
        sample_part,
    };
    let desc = format!(
        "mainnet blocks {} to {} replayed from state diffs (Xatu, CC BY 4.0); untouched state modeled as {} accounts, {} storage slots in {} contracts",
        first_block,
        first_block + n_blocks as u64 - 1,
        scale.accounts,
        scale.storage_slots,
        scale.storage_contracts
    );
    let mut reader = DiffReader::open(diffs, skip, with_reads, (sample_bits, sample_part));
    let next = match reader.next() {
        Some(Item::Block(n)) => Some(n),
        None => None,
        Some(_) => unreachable!("a block line comes first"),
    };
    Loaded {
        replay: Replay {
            desc,
            genesis,
            reader,
            next,
            reads: Vec::new(),
        },
        shape,
        facts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbt_ref::tree::{CommitSink, PbtTree, RecordKind, RecordOp, RewriteStats};
    use pbt_store::shape::prefix_path;

    /// `keys_along` is `keys_under` at every prefix length, from the zone
    /// byte down, for every kind of path a replay asks about.
    #[test]
    fn keys_along_agrees_with_keys_under() {
        let scale = Scale {
            accounts: 1000,
            storage_slots: 50_000,
            storage_contracts: 100,
            code_bytes: 31 * 1000,
            neighbor_slots: None,
        };
        let written_addr = [7u8; 32];
        let written_hash = key_hash(&written_addr);
        let slot_a = get_tree_key_for_storage_slot(&written_addr, &slot_from_u64(100));
        let slot_b = get_tree_key_for_storage_slot(&written_addr, &slot_from_u64(900));
        let mut written: Vec<[u8; 33]> = [&slot_a, &slot_b]
            .iter()
            .map(|k| k[33..66].try_into().unwrap())
            .collect();
        written.sort_unstable();
        let mut contracts = HashMap::new();
        contracts.insert(
            written_hash,
            Contract {
                untouched_stems: 5,
                slots_per_stem: 3.0,
                written,
            },
        );
        let shape = MainnetShape::new(scale, contracts);
        let other_addr = [9u8; 32];
        let paths = [
            get_tree_key_for_basic_data(&written_addr),
            slot_a.clone(),
            get_tree_key_for_storage_slot(&other_addr, &slot_from_u64(1)),
            [&[CODE_ZONE][..], &[0x5Au8; 33][..]].concat(),
        ];
        for key in paths {
            let path = pbt_store::btree::key_path(&key);
            let nbits = key.len() * 8;
            let along = shape.keys_along(&path);
            assert_eq!(along.len(), nbits + 1);
            for i in 8..=nbits {
                assert_eq!(
                    along[i],
                    shape.keys_under(&prefix_path(&path, i)),
                    "zone {:#x} at {i} bits",
                    key[0]
                );
            }
        }
    }

    /// Every key of the set the splits define, by walking all of them.
    fn enumerate(tag: u8, region: &[u8], n: u64) -> Vec<[u8; 32]> {
        fn go(
            tag: u8,
            region: &[u8],
            prefix: [u8; 32],
            nbits: usize,
            c: u64,
            out: &mut Vec<[u8; 32]>,
        ) {
            match c {
                0 => {}
                1 => out.push(lone_key(tag, region, &prefix, nbits)),
                _ => {
                    let (c0, c1) = split(&seed(tag, region, &prefix, nbits), c);
                    let mask = 0x80u8 >> (nbits % 8);
                    let (mut l, mut r) = (prefix, prefix);
                    l[nbits / 8] &= !mask;
                    r[nbits / 8] |= mask;
                    go(tag, region, l, nbits + 1, c0, out);
                    go(tag, region, r, nbits + 1, c1, out);
                }
            }
        }
        let mut out = Vec::new();
        go(tag, region, [0u8; 32], 0, n, &mut out);
        out
    }

    /// Keys of a sorted set under the first `nbits` of `prefix`.
    fn range(sorted: &[[u8; 32]], prefix: &[u8; 32], nbits: usize) -> std::ops::Range<usize> {
        let (mut lo, mut hi) = ([0u8; 32], [0xFFu8; 32]);
        let full = nbits / 8;
        lo[..full].copy_from_slice(&prefix[..full]);
        hi[..full].copy_from_slice(&prefix[..full]);
        if nbits % 8 != 0 {
            let mask = 0xFFu8 << (8 - nbits % 8);
            lo[full] = prefix[full] & mask;
            hi[full] = (prefix[full] & mask) | !mask;
        }
        sorted.partition_point(|k| k < &lo)..sorted.partition_point(|k| k <= &hi)
    }

    /// Stand-ins placed from EXACT knowledge of an untouched key set, by
    /// the same rule `siblings` applies to the modeled one.
    fn siblings_exact(untouched: &[[u8; 32]], path: &[u8; 32]) -> Beside {
        let mut keys = Vec::new();
        for i in 0..256 {
            let here = range(untouched, path, i);
            match here.len() {
                0 => break,
                1 => {
                    return Beside {
                        keys,
                        lone: Some(untouched[here.start]),
                    }
                }
                _ => {}
            }
            let mut other = *path;
            other[i / 8] ^= 0x80 >> (i % 8);
            let off = range(untouched, &other, i + 1);
            if !off.is_empty() {
                keys.push(untouched[off.start]);
                if off.len() == here.len() {
                    // Nothing untouched goes on with the path: the other
                    // side is a subtree, and its top branch needs the first
                    // key beyond its first fork as well.
                    let (first, last) = (&untouched[off.start], &untouched[off.end - 1]);
                    let fork = (0..256)
                        .find(|&b| bit(first, b) != bit(last, b))
                        .expect("distinct keys");
                    let mate =
                        off.start + untouched[off.clone()].partition_point(|k| bit(k, fork) == 0);
                    keys.push(untouched[mate]);
                }
            }
        }
        Beside { keys, lone: None }
    }

    /// An account's leaves: basic data, and its code hash when accounts are
    /// modeled with both header leaves, as every real one has.
    fn account(hash: &[u8; 32], two: bool) -> Vec<Vec<u8>> {
        let mut v = vec![get_tree_key(ACCOUNT_ZONE, hash, BASIC_DATA_LEAF_KEY)];
        if two {
            v.push(get_tree_key(ACCOUNT_ZONE, hash, CODE_HASH_LEAF_KEY));
        }
        v
    }

    type Rec = (u8, u8, u32, u32, u32, Vec<u8>, Option<Vec<u8>>);

    /// Everything a commit reports, in a comparable order: records written
    /// and records deleted.
    fn emitted(sink: &CommitSink) -> (Vec<Rec>, Vec<Rec>) {
        let mut w: Vec<Rec> = sink
            .records
            .iter()
            .map(|r| {
                (
                    (r.kind == RecordKind::Leaf) as u8,
                    (r.op == RecordOp::Create) as u8,
                    r.node_depth,
                    r.start_bit,
                    r.bytes,
                    r.pos.clone(),
                    r.leaf_key.as_ref().map(|k| k.to_vec()),
                )
            })
            .collect();
        let mut d: Vec<Rec> = sink
            .deletes
            .iter()
            .map(|d| {
                (
                    d.was_leaf as u8,
                    0,
                    d.node_depth,
                    d.start_bit,
                    d.bytes,
                    d.pos.clone(),
                    d.leaf_key.as_ref().map(|k| k.to_vec()),
                )
            })
            .collect();
        w.sort();
        d.sort();
        (w, d)
    }

    /// Run the same writes through a full tree and a lean one (written keys
    /// plus stand-ins) and require that every commit reports the same
    /// records, written and deleted, in both. The blocks rewrite, create,
    /// delete and re-create. `two`: accounts hold both header leaves.
    fn assert_same_commits(
        background: &[[u8; 32]],
        existing: &[[u8; 32]],
        fresh: &[[u8; 32]],
        two: bool,
        stand_ins: impl Fn(&[u8; 32]) -> Beside,
    ) {
        let v0 = |k: &Vec<u8>| value(k, 0);
        let mut full = PbtTree::new();
        for k in background
            .iter()
            .chain(existing)
            .flat_map(|k| account(k, two))
        {
            full.write(&k, &v0(&k));
        }
        let mut lean = PbtTree::new();
        for k in existing.iter().flat_map(|k| account(k, two)) {
            lean.write(&k, &v0(&k));
        }
        let mut placed: HashSet<Vec<u8>> = HashSet::new();
        for k in existing.iter().chain(fresh) {
            let b = stand_ins(k);
            let keys = b
                .keys
                .iter()
                .flat_map(|f| account(f, false))
                .chain(b.lone.iter().flat_map(|f| account(f, two)));
            for f in keys {
                lean.write(&f, &v0(&f));
                placed.insert(f);
            }
        }
        // The lean tree is a fraction of the full one.
        let full_leaves = background.len() * if two { 2 } else { 1 };
        assert!(
            placed.len() < full_leaves / 2,
            "{} stand-ins for {} leaves",
            placed.len(),
            full_leaves
        );
        let mut st = RewriteStats::default();
        full.commit(&mut st, None);
        lean.commit(&mut st, None);

        let (mut compared, mut deleted) = (0, 0);
        for block in 1..=9u64 {
            let mut writes: Vec<(Vec<u8>, Digest)> = Vec::new();
            for (i, k) in existing.iter().enumerate() {
                let keys = account(k, two);
                if i % 5 == 0 && block >= 4 {
                    // Deleted in block 4, and every other one back in block 8.
                    if block == 4 {
                        writes.extend(keys.into_iter().map(|k| (k, ZERO_VALUE)));
                    } else if block == 8 && i % 10 == 0 {
                        writes.extend(keys.into_iter().map(|k| {
                            let v = value(&k, block);
                            (k, v)
                        }));
                    }
                } else if i as u64 % 6 == block % 6 || i % 2 == 0 {
                    writes.push((keys[0].clone(), value(&keys[0], block)));
                }
            }
            for (i, k) in fresh.iter().enumerate() {
                let keys = account(k, two);
                let born = (i as u64 % 3) + 1;
                if i % 4 == 0 && block >= 6 {
                    // Deleted in block 6, and every other one back in block 9.
                    if block == 6 {
                        writes.extend(keys.into_iter().map(|k| (k, ZERO_VALUE)));
                    } else if block == 9 && i % 8 == 0 {
                        writes.extend(keys.into_iter().map(|k| {
                            let v = value(&k, block);
                            (k, v)
                        }));
                    }
                } else if block == born {
                    writes.extend(keys.into_iter().map(|k| {
                        let v = value(&k, block);
                        (k, v)
                    }));
                } else if block > born {
                    writes.push((keys[0].clone(), value(&keys[0], block)));
                }
            }
            let (mut a, mut b) = (CommitSink::default(), CommitSink::default());
            for (k, v) in &writes {
                full.write(k, v);
                lean.write(k, v);
            }
            full.commit(&mut st, Some(&mut a));
            lean.commit(&mut st, Some(&mut b));
            let ((wa, da), (wb, db)) = (emitted(&a), emitted(&b));
            assert!(!wa.is_empty());
            assert_eq!(wa.len(), wb.len(), "block {block}: record counts differ");
            assert_eq!(wa, wb, "block {block}: records differ");
            assert_eq!(da, db, "block {block}: deletions differ");
            compared += wa.len();
            deleted += da.len();
        }
        assert!(compared > 10_000, "only {compared} records compared");
        assert!(deleted > 100, "only {deleted} deletions compared");
    }

    fn hashes(tag: &[u8], n: u32) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| h(&[tag, &i.to_le_bytes()].concat()))
            .collect()
    }

    /// THE claim the replay rests on, with the untouched set known exactly:
    /// a tree holding only the written keys plus the stand-ins reports, in
    /// every commit, exactly the records the full tree reports. Same
    /// branches, positions, depths, sizes and kinds, on the written paths
    /// and beside them, through rewrites, creations and deletions.
    #[test]
    fn stand_in_tree_commits_what_the_full_tree_commits() {
        let all = hashes(b"bg", 30_000);
        let existing: Vec<[u8; 32]> = all.iter().step_by(100).copied().collect();
        let fresh = hashes(b"new", 300);
        let mut untouched: Vec<[u8; 32]> = all
            .iter()
            .filter(|k| !existing.contains(k))
            .copied()
            .collect();
        untouched.sort_unstable();
        for two in [false, true] {
            assert_same_commits(&untouched, &existing, &fresh, two, |k| {
                siblings_exact(&untouched, k)
            });
        }
    }

    /// The same claim for the code the replay actually runs: the untouched
    /// set is the one the splits define, enumerated here so a full tree
    /// can be built from it, and the stand-ins come from `siblings`.
    #[test]
    fn modeled_stand_ins_commit_what_the_modeled_full_tree_commits() {
        let n = 20_000u64;
        let mut background = enumerate(REGION_ACCOUNTS, &[], n);
        assert_eq!(background.len() as u64, n);
        background.sort_unstable();
        background.dedup();
        assert_eq!(
            background.len() as u64,
            n,
            "the modeled set has duplicate keys"
        );
        // Written keys are outside the modeled set, as on mainnet: the
        // model covers the state the replay does not touch.
        let (existing, fresh) = (hashes(b"old", 250), hashes(b"new", 250));
        for two in [false, true] {
            assert_same_commits(&background, &existing, &fresh, two, |k| {
                siblings(REGION_ACCOUNTS, &[], n, k)
            });
        }
    }

    /// Counting under a prefix agrees with the enumerated set, at every
    /// depth, including below the point where one key is left.
    #[test]
    fn counts_agree_with_the_enumerated_set() {
        let n = 5_000u64;
        let mut set = enumerate(REGION_GROUPS, b"some contract", n);
        set.sort_unstable();
        for k in 0..300u32 {
            // Half the probes follow a real member, so deep prefixes are hit.
            let path = if k % 2 == 0 {
                set[(k as usize * 7) % set.len()]
            } else {
                h(&k.to_be_bytes())
            };
            for nbits in [0usize, 1, 5, 9, 12, 13, 14, 20, 40, 256] {
                let want = range(&set, &path, nbits).len() as u64;
                assert_eq!(
                    count_under(REGION_GROUPS, b"some contract", n, &path, nbits),
                    want,
                    "probe {k} at {nbits} bits"
                );
            }
        }
    }

    /// The split of a prefix is a function of the prefix alone, conserves
    /// the count, and is unbiased over many prefixes.
    #[test]
    fn split_is_deterministic_conserving_and_fair() {
        let mut left_total = 0u64;
        for i in 0..2000u32 {
            let s = h(&i.to_le_bytes());
            for c in [0u64, 1, 2, 17, 256, 257, 10_000, 1_000_000_000] {
                let (a, b) = split(&s, c);
                assert_eq!(a + b, c);
                assert_eq!(split(&s, c), (a, b));
            }
            left_total += split(&s, 1000).0;
        }
        let mean = left_total as f64 / 2000.0;
        assert!((mean - 500.0).abs() < 2.0, "mean left share {mean}");
    }

    /// Stand-ins leave the path at successive levels, about log2(n) of
    /// them; only the pair that makes the last neighbour a branch shares one.
    #[test]
    fn stand_ins_leave_the_path_at_successive_levels() {
        let (mut pairs, mut lones) = (0, 0);
        for p in 0..200u32 {
            let path = h(&p.to_le_bytes());
            let b = siblings(REGION_ACCOUNTS, &[], 1_000_000, &path);
            let all: Vec<[u8; 32]> = b.keys.iter().chain(&b.lone).copied().collect();
            assert!((17..=28).contains(&all.len()), "{} stand-ins", all.len());
            let levels: Vec<usize> = all
                .iter()
                .map(|f| {
                    (0..256)
                        .find(|&i| bit(f, i) != bit(&path, i))
                        .expect("differs")
                })
                .collect();
            let n = levels.len();
            assert!(
                levels[..n - 1].windows(2).all(|w| w[0] < w[1]),
                "levels repeat or come out of order: {levels:?}"
            );
            assert!(levels[n - 2] <= levels[n - 1]);
            if levels[n - 2] == levels[n - 1] {
                assert!(
                    b.lone.is_none(),
                    "a path ends beside a lone key or beside a subtree, not both"
                );
                pairs += 1;
            }
            lones += b.lone.is_some() as u32;
        }
        // A path ends beside a subtree about 28% of the time (1 - 1/(2 ln 2)).
        assert_eq!(pairs + lones, 200);
        assert!(
            (35..=80).contains(&pairs),
            "{pairs} of 200 paths end beside a subtree"
        );
    }
}
