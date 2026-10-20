//! Spec-faithful rebuild oracle: `binarize` / `merkelize` / `state_root`
//! transcribed directly from EIP-8297. Used as the correctness oracle for
//! the incremental tree; O(N log N) per call, tests only.

use crate::bits::{bit, bit_len, encode_bit_prefix};
use crate::hash::{h, Digest, EMPTY_ROOT};
use crate::keys::MAX_KEY_LENGTH;
use crate::tree::{BRANCH_TAG, LEAF_TAG};
use std::collections::BTreeMap;

enum ONode {
    Leaf {
        key: Vec<u8>,
        value: Digest,
    },
    Branch {
        prefix: Vec<u8>,
        left: Box<ONode>,
        right: Box<ONode>,
    },
}

fn binarize(entries: &[(&Vec<u8>, &Digest)], depth: usize) -> ONode {
    assert!(!entries.is_empty());
    if entries.len() == 1 {
        let (key, value) = entries[0];
        return ONode::Leaf {
            key: key.clone(),
            value: *value,
        };
    }
    let mut split = depth;
    loop {
        for (key, _) in entries {
            // A key that runs out of bits while still grouped with others
            // is a prefix of theirs.
            assert!(split < bit_len(key), "keys are not prefix-free");
        }
        let first = bit(entries[0].0, split);
        if entries.iter().any(|(k, _)| bit(k, split) != first) {
            break;
        }
        split += 1;
    }
    let left: Vec<_> = entries
        .iter()
        .copied()
        .filter(|(k, _)| bit(k, split) == 0)
        .collect();
    let right: Vec<_> = entries
        .iter()
        .copied()
        .filter(|(k, _)| bit(k, split) == 1)
        .collect();
    let prefix: Vec<u8> = (depth..split).map(|i| bit(entries[0].0, i)).collect();
    ONode::Branch {
        prefix,
        left: Box::new(binarize(&left, split + 1)),
        right: Box::new(binarize(&right, split + 1)),
    }
}

fn merkelize(node: &ONode) -> Digest {
    match node {
        ONode::Leaf { key, value } => {
            let mut buf = Vec::with_capacity(1 + key.len() + 32);
            buf.push(LEAF_TAG);
            buf.extend_from_slice(key);
            buf.extend_from_slice(value);
            h(&buf)
        }
        ONode::Branch {
            prefix,
            left,
            right,
        } => {
            let mut buf = Vec::new();
            buf.push(BRANCH_TAG);
            buf.extend_from_slice(&encode_bit_prefix(prefix));
            buf.extend_from_slice(&merkelize(left));
            buf.extend_from_slice(&merkelize(right));
            h(&buf)
        }
    }
}

/// state_root over a full entry set (spec transcription).
pub fn state_root(entries: &BTreeMap<Vec<u8>, Digest>) -> Digest {
    for (key, _value) in entries {
        assert!(
            (1..=MAX_KEY_LENGTH).contains(&key.len()),
            "key length out of range"
        );
        // values are [u8; 32] by type
    }
    if entries.is_empty() {
        return EMPTY_ROOT;
    }
    let refs: Vec<_> = entries.iter().collect();
    merkelize(&binarize(&refs, 0))
}
