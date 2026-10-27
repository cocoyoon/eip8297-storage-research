//! What the untouched part of a large state looks like.
//!
//! A replay of real traffic only ever inserts the keys that traffic writes,
//! a sliver of the whole state. The tree gets its true depth from stand-in
//! leaves placed beside every written path, but a page store also has to
//! know things about state it never sees: whether a merkle page holds
//! enough leaves to be stored at all, and which value-store page a key
//! shares with neighbours that are not in the run. A `StateShape` answers
//! those questions from the size and layout of the full state.

use std::sync::Arc;

pub trait StateShape: Send + Sync {
    /// Leaves under `path` in the full state, the run's own keys aside.
    /// `path` is a bit path in `encode_bit_prefix` form: a 2-byte big-endian
    /// bit count, then the bits packed MSB first.
    fn keys_under(&self, path: &[u8]) -> u64;
    /// Stable identity of the value-store leaf page that holds `key`.
    fn value_leaf_page(&self, key: &[u8]) -> u64;
    /// Stable identity of the bottom branch node above that leaf page.
    fn value_bbn(&self, key: &[u8]) -> u64;
    /// `keys_under` of every prefix of `path` at once: `out[i]` is the
    /// count under its first `i` bits, for `i` in `0..=nbits`. The default
    /// asks `keys_under` once per prefix; a shape that walks a path can
    /// answer in one pass.
    fn keys_along(&self, path: &[u8]) -> Vec<u64> {
        let nbits = u16::from_be_bytes([path[0], path[1]]) as usize;
        (0..=nbits)
            .map(|i| self.keys_under(&prefix_path(path, i)))
            .collect()
    }
}

/// The first `nbits` bits of a path, as a path.
pub fn prefix_path(path: &[u8], nbits: usize) -> Vec<u8> {
    let body = &path[2..];
    let full = nbits / 8;
    let mut out = Vec::with_capacity(3 + full);
    out.extend_from_slice(&(nbits as u16).to_be_bytes());
    out.extend_from_slice(&body[..full]);
    if nbits % 8 != 0 {
        out.push(body[full] & (0xFFu8 << (8 - nbits % 8)));
    }
    out
}

pub type SharedShape = Arc<dyn StateShape>;
