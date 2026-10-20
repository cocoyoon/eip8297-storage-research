//! EIP-8297 Partitioned Binary Tree — reference layer.
//!
//! Direct transcription of the EIP-8297 specification (Draft, 2026-06-11):
//! key derivation, code chunking, node merkelization, and two tree
//! implementations — a spec-faithful rebuild oracle (`oracle`) and an
//! incremental tree with hash caching (`tree`) used by the Phase-1
//! rewrite-frequency characterization.
//!
//! NOTE: the spec's hash function is not final; like the EIP reference
//! implementation this crate uses BLAKE3 throughout (`hash::h`).

pub mod bits;
pub mod hash;
pub mod keys;
pub mod oracle;
pub mod tree;
