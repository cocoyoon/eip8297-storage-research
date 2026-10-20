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
}

pub type SharedShape = Arc<dyn StateShape>;
