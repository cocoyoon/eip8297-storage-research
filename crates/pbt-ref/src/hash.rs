//! Tree hash function. Spec: "H is the tree's 32-byte hash function, the
//! same function as key_hash". Not final; reference implementation uses
//! BLAKE3, and so do we.

pub type Digest = [u8; 32];

pub const EMPTY_ROOT: Digest = [0u8; 32]; // "The hash of an empty tree is [0x00] * 32"
pub const ZERO_VALUE: Digest = [0u8; 32];

/// Keccak-256 of empty bytecode. The code-hash leaf of an account with no
/// code holds this value regardless of the tree's merkelization hash
/// (spec, "Header values" / "Backwards Compatibility").
pub const KECCAK_EMPTY: Digest = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// H(x): full 32-byte digest.
pub fn h(data: &[u8]) -> Digest {
    *blake3::hash(data).as_bytes()
}

/// key_hash(inp) = blake3(inp).digest()  (spec, "Tree embedding")
pub fn key_hash(data: &[u8]) -> Digest {
    h(data)
}
