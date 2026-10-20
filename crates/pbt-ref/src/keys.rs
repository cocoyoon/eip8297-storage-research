//! EIP-8297 tree embedding: zones, key derivation, header packing, and
//! code chunking. Constants and functions transcribed from the spec.

use crate::hash::{key_hash, Digest};

pub const BASIC_DATA_LEAF_KEY: u8 = 0;
pub const CODE_HASH_LEAF_KEY: u8 = 1;
pub const DELEGATION_LEAF_KEY: u8 = 2;
pub const HEADER_STORAGE_OFFSET: u8 = 64;
pub const HEADER_STORAGE_SLOTS: u8 = 64;
pub const STEM_SUBTREE_WIDTH: u32 = 256;

pub const ACCOUNT_ZONE: u8 = 0x00;
pub const CODE_ZONE: u8 = 0x01;
pub const STORAGE_ZONE: u8 = 0xFF;

pub const ACCOUNT_KEY_LENGTH: usize = 34;
pub const CODE_KEY_LENGTH: usize = 34;
pub const STORAGE_KEY_LENGTH: usize = 66;

pub const MAX_KEY_LENGTH: usize = 8192;

// Required invariant (spec, "Tree embedding").
const _: () =
    assert!(HEADER_STORAGE_OFFSET as u32 + HEADER_STORAGE_SLOTS as u32 <= STEM_SUBTREE_WIDTH);

pub type Address32 = [u8; 32];

/// 256-bit storage slot number, big-endian.
pub type StorageSlot = [u8; 32];

pub fn address20_to_address32(address: &[u8; 20]) -> Address32 {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(address);
    out
}

pub fn slot_from_u64(n: u64) -> StorageSlot {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&n.to_be_bytes());
    out
}

/// get_tree_key(zone, tree_position, sub_index) = zone || tree_position || sub_index
pub fn get_tree_key(zone: u8, tree_position: &[u8], sub_index: u8) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + tree_position.len());
    key.push(zone);
    key.extend_from_slice(tree_position);
    key.push(sub_index);
    key
}

/// Account header stem: keyed by the address alone.
pub fn get_tree_key_for_header(address: &Address32, sub_index: u8) -> Vec<u8> {
    let key = get_tree_key(ACCOUNT_ZONE, &key_hash(address), sub_index);
    assert_eq!(key.len(), ACCOUNT_KEY_LENGTH);
    key
}

pub fn get_tree_key_for_basic_data(address: &Address32) -> Vec<u8> {
    get_tree_key_for_header(address, BASIC_DATA_LEAF_KEY)
}

pub fn get_tree_key_for_code_hash(address: &Address32) -> Vec<u8> {
    get_tree_key_for_header(address, CODE_HASH_LEAF_KEY)
}

pub fn get_tree_key_for_delegation(address: &Address32) -> Vec<u8> {
    get_tree_key_for_header(address, DELEGATION_LEAF_KEY)
}

/// Code chunk key: content-addressed by code_hash, grouped by tree_index.
pub fn get_tree_key_for_code_chunk(code_hash: &Digest, chunk_id: u64) -> Vec<u8> {
    let tree_index = chunk_id / STEM_SUBTREE_WIDTH as u64;
    let sub_index = (chunk_id % STEM_SUBTREE_WIDTH as u64) as u8;
    let mut inp = Vec::with_capacity(64);
    inp.extend_from_slice(code_hash);
    inp.extend_from_slice(&u256_be(tree_index));
    let key = get_tree_key(CODE_ZONE, &key_hash(&inp), sub_index);
    assert_eq!(key.len(), CODE_KEY_LENGTH);
    key
}

fn u256_be(n: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&n.to_be_bytes());
    out
}

/// storage_tree_position(address, tree_index) = key_hash(address) || key_hash(address || tree_index)
/// tree_index passed as its 32-byte big-endian encoding (slot >> 8).
pub fn storage_tree_position(address: &Address32, tree_index_be: &[u8; 32]) -> Vec<u8> {
    let prefix = key_hash(address);
    let mut inp = Vec::with_capacity(64);
    inp.extend_from_slice(address);
    inp.extend_from_slice(tree_index_be);
    let suffix = key_hash(&inp);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&prefix);
    out.extend_from_slice(&suffix);
    out
}

/// True iff the slot lives in the account header (slot < HEADER_STORAGE_SLOTS).
pub fn slot_in_header(slot: &StorageSlot) -> bool {
    slot[..31].iter().all(|&b| b == 0) && slot[31] < HEADER_STORAGE_SLOTS
}

pub fn get_tree_key_for_storage_slot(address: &Address32, slot: &StorageSlot) -> Vec<u8> {
    if slot_in_header(slot) {
        return get_tree_key_for_header(address, HEADER_STORAGE_OFFSET + slot[31]);
    }
    // tree_index = slot // 256  (shift right one byte); sub_index = slot % 256.
    let mut tree_index_be = [0u8; 32];
    tree_index_be[1..].copy_from_slice(&slot[..31]);
    let sub_index = slot[31];
    let key = get_tree_key(
        STORAGE_ZONE,
        &storage_tree_position(address, &tree_index_be),
        sub_index,
    );
    assert_eq!(key.len(), STORAGE_KEY_LENGTH);
    key
}

/// BASIC_DATA value: version(1B@0) || reserved(3B) || code_size(4B BE@4)
/// || nonce(8B BE@8) || balance(16B BE@16). Version is always written as
/// zero ("Setting any header field also sets version to zero").
pub fn pack_basic_data(code_size: u32, nonce: u64, balance: u128) -> Digest {
    let mut out = [0u8; 32];
    out[4..8].copy_from_slice(&code_size.to_be_bytes());
    out[8..16].copy_from_slice(&nonce.to_be_bytes());
    out[16..32].copy_from_slice(&balance.to_be_bytes());
    out
}

/// Delegation leaf value: 0xef0100 || target(20B) || 0x00 * 9.
pub fn make_delegation_value(target: &[u8; 20]) -> Digest {
    let mut out = [0u8; 32];
    out[0] = 0xef;
    out[1] = 0x01;
    out[2] = 0x00;
    out[3..23].copy_from_slice(target);
    out
}

pub const PUSH_OFFSET: u8 = 95;
pub const PUSH1: u8 = PUSH_OFFSET + 1;
pub const PUSH32: u8 = PUSH_OFFSET + 32;

/// chunkify_code: 31-byte chunks with a leading PUSHDATA-count byte
/// (spec, "Code"). Direct transcription.
pub fn chunkify_code(code: &[u8]) -> Vec<Digest> {
    let mut code = code.to_vec();
    if code.len() % 31 != 0 {
        let pad = 31 - (code.len() % 31);
        code.extend(std::iter::repeat(0u8).take(pad));
    }
    let mut bytes_to_exec_data = vec![0usize; code.len() + 32];
    let mut pos = 0usize;
    while pos < code.len() {
        let pushdata_bytes = if (PUSH1..=PUSH32).contains(&code[pos]) {
            (code[pos] - PUSH_OFFSET) as usize
        } else {
            0
        };
        pos += 1;
        for x in 0..pushdata_bytes {
            bytes_to_exec_data[pos + x] = pushdata_bytes - x;
        }
        pos += pushdata_bytes;
    }
    (0..code.len())
        .step_by(31)
        .map(|pos| {
            let mut chunk = [0u8; 32];
            chunk[0] = bytes_to_exec_data[pos].min(31) as u8;
            chunk[1..].copy_from_slice(&code[pos..pos + 31]);
            chunk
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::key_hash;

    const A20: [u8; 20] = [0xAA; 20];

    fn a32() -> Address32 {
        address20_to_address32(&A20)
    }

    #[test]
    fn address_padding() {
        let a = a32();
        assert_eq!(&a[..12], &[0u8; 12]);
        assert_eq!(&a[12..], &A20);
    }

    /// Spec test vector: BASIC_DATA key = 0x00 || H(A) || 0x00, 34 bytes.
    #[test]
    fn vector_basic_data_key() {
        let a = a32();
        let key = get_tree_key_for_basic_data(&a);
        assert_eq!(key.len(), 34);
        assert_eq!(key[0], 0x00);
        assert_eq!(&key[1..33], &key_hash(&a));
        assert_eq!(key[33], 0x00);
    }

    /// Spec test vector: delegation key = 0x00 || H(A) || 0x02; value layout.
    #[test]
    fn vector_delegation() {
        let a = a32();
        let key = get_tree_key_for_delegation(&a);
        assert_eq!(key.len(), 34);
        assert_eq!(key[33], 0x02);
        let t20 = [0xBBu8; 20];
        let v = make_delegation_value(&t20);
        assert_eq!(&v[..3], &[0xef, 0x01, 0x00]);
        assert_eq!(&v[3..23], &t20);
        assert_eq!(&v[23..], &[0u8; 9]);
    }

    /// Spec test vector: slot 5 -> header sub-index 69 (0x45).
    #[test]
    fn vector_storage_slot_5() {
        let a = a32();
        let key = get_tree_key_for_storage_slot(&a, &slot_from_u64(5));
        assert_eq!(key.len(), 34);
        assert_eq!(key[0], 0x00);
        assert_eq!(&key[1..33], &key_hash(&a));
        assert_eq!(key[33], 0x45);
    }

    /// Spec test vector: slot 1000 -> 0xFF || H(A) || H(A||3) || 0xE8, 66 bytes.
    #[test]
    fn vector_storage_slot_1000() {
        let a = a32();
        let key = get_tree_key_for_storage_slot(&a, &slot_from_u64(1000));
        assert_eq!(key.len(), 66);
        assert_eq!(key[0], 0xFF);
        assert_eq!(&key[1..33], &key_hash(&a));
        let mut inp = Vec::new();
        inp.extend_from_slice(&a);
        let mut ti = [0u8; 32];
        ti[31] = 3;
        inp.extend_from_slice(&ti);
        assert_eq!(&key[33..65], &key_hash(&inp));
        assert_eq!(key[65], 0xE8);
    }

    /// Spec test vectors: code chunk 5 -> H(C||0)||0x05; chunk 300 -> H(C||1)||0x2C.
    #[test]
    fn vector_code_chunks() {
        let c: Digest = [0xCC; 32];
        let k5 = get_tree_key_for_code_chunk(&c, 5);
        assert_eq!(k5.len(), 34);
        assert_eq!(k5[0], 0x01);
        let mut inp = Vec::new();
        inp.extend_from_slice(&c);
        inp.extend_from_slice(&[0u8; 32]);
        assert_eq!(&k5[1..33], &key_hash(&inp));
        assert_eq!(k5[33], 0x05);

        let k300 = get_tree_key_for_code_chunk(&c, 300);
        let mut inp = Vec::new();
        inp.extend_from_slice(&c);
        let mut ti = [0u8; 32];
        ti[31] = 1;
        inp.extend_from_slice(&ti);
        assert_eq!(&k300[1..33], &key_hash(&inp));
        assert_eq!(k300[33], 0x2C);
    }

    /// Spec example: a chunk beginning with the last 2 bytes of a PUSH4's
    /// data records 2 leading PUSHDATA bytes.
    #[test]
    fn chunkify_pushdata_straddle() {
        // 28 ADD opcodes, then PUSH4 at position 28; its 4 data bytes sit at
        // positions 29,30 (chunk 0) and 31,32 (chunk 1).
        let mut code = vec![0x01u8; 28];
        code.push(0x63); // PUSH4
        code.extend_from_slice(&[99, 98, 97, 96]);
        code.push(PUSH1);
        code.push(128);
        code.push(0x52); // MSTORE
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0][0], 0); // chunk 0 starts on an opcode
        assert_eq!(chunks[1][0], 2); // chunk 1 starts inside PUSHDATA, 2 bytes remain
        assert_eq!(chunks[1][1], 97);
        assert_eq!(chunks[1][2], 96);
        assert_eq!(chunks[1][3], PUSH1);
    }

    /// A 31-byte all-zero chunk with no PUSHDATA carry encodes to 32 zero bytes.
    #[test]
    fn chunkify_zero_chunk() {
        let code = vec![0u8; 62]; // two chunks of STOPs
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], [0u8; 32]);
        assert_eq!(chunks[1], [0u8; 32]);
    }

    /// Zero bytes that continue PUSHDATA from an earlier chunk do NOT
    /// qualify as a zero chunk: byte 0 records the continuation.
    #[test]
    fn chunkify_pushdata_zeros_not_zero_chunk() {
        // PUSH32 at position 29: data at 30 (chunk 0) and 31..61 (chunk 1) -- 31 bytes.
        let mut code = vec![0x01u8; 29];
        code.push(PUSH32);
        code.extend_from_slice(&[0u8; 32]); // all-zero pushdata
        let chunks = chunkify_code(&code);
        assert_eq!(chunks[1][0], 31); // 31 leading pushdata bytes (capped)
        assert_ne!(chunks[1], [0u8; 32]);
    }

    #[test]
    fn basic_data_layout() {
        let v = pack_basic_data(0x0102_0304, 0x1122_3344_5566_7788, 42);
        assert_eq!(v[0], 0); // version
        assert_eq!(&v[1..4], &[0, 0, 0]); // reserved
        assert_eq!(&v[4..8], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(&v[8..16], &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        assert_eq!(u128::from_be_bytes(v[16..32].try_into().unwrap()), 42);
    }
}
