//! Bit-level helpers shared by the oracle and the incremental tree.

/// i-th bit of a byte string, MSB-first (spec `_bytes_to_bits` ordering).
#[inline]
pub fn bit(data: &[u8], i: usize) -> u8 {
    (data[i / 8] >> (7 - (i % 8))) & 1
}

/// Number of addressable bits in a key.
#[inline]
pub fn bit_len(data: &[u8]) -> usize {
    data.len() * 8
}

/// encode_bit_prefix: two-byte big-endian bit count, then the bits packed
/// MSB-first and zero-padded to a byte boundary (spec, "Node merkelization").
pub fn encode_bit_prefix(prefix: &[u8]) -> Vec<u8> {
    assert!(
        prefix.len() < (1 << 16),
        "prefix exceeds encodable bit count"
    );
    let mut packed = vec![0u8; (prefix.len() + 7) / 8];
    for (i, &b) in prefix.iter().enumerate() {
        packed[i / 8] |= b << (7 - (i % 8));
    }
    let mut out = Vec::with_capacity(2 + packed.len());
    out.extend_from_slice(&(prefix.len() as u16).to_be_bytes());
    out.extend_from_slice(&packed);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_empty() {
        assert_eq!(encode_bit_prefix(&[]), vec![0x00, 0x00]);
    }

    #[test]
    fn encode_single_one() {
        assert_eq!(encode_bit_prefix(&[1]), vec![0x00, 0x01, 0x80]);
    }

    #[test]
    fn encode_six_zeros() {
        assert_eq!(
            encode_bit_prefix(&[0, 0, 0, 0, 0, 0]),
            vec![0x00, 0x06, 0x00]
        );
    }

    #[test]
    fn encode_nine_alternating() {
        // 1,0,1,0,1,0,1,0,1 -> 0xAA 0x80
        assert_eq!(
            encode_bit_prefix(&[1, 0, 1, 0, 1, 0, 1, 0, 1]),
            vec![0x00, 0x09, 0xAA, 0x80]
        );
    }

    #[test]
    fn bit_msb_first() {
        let d = [0b1010_0000u8];
        assert_eq!(bit(&d, 0), 1);
        assert_eq!(bit(&d, 1), 0);
        assert_eq!(bit(&d, 2), 1);
        assert_eq!(bit(&d, 3), 0);
    }
}
