// No wasm-side consumer yet; writer / model wire `encode_ulid` in
// once they land.
#![allow(dead_code)]

//! Tiny ULID generator.
//!
//! ULIDs are 128-bit identifiers — 48 bits of millisecond timestamp
//! followed by 80 bits of randomness — encoded as 26 Crockford
//! base32 characters. Two ULIDs minted on different devices in the
//! same millisecond essentially never collide; ULIDs minted in
//! ascending wall-clock order sort lexicographically.
//!
//! Used as the stable identity for every workspace / column / tile
//! entity created from this element: the `this:` URI is
//! `id:<ulid>`, so subsequent edits target the same entity instead
//! of spawning a fresh one each time a field changes.

/// Crockford base32 alphabet — `I`, `L`, `O`, `U` excluded for
/// visual unambiguity.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode a 48-bit timestamp (ms since epoch) + 80 bits of
/// randomness as a 26-character Crockford-base32 ULID string. The
/// timestamp occupies the leading 10 characters so two ULIDs minted
/// in ascending time order sort lexicographically.
pub fn encode_ulid(timestamp_ms: u64, random: [u8; 10]) -> String {
    let mut bytes = [0u8; 16];
    // Take the low 48 bits of the timestamp, big-endian.
    bytes[..6].copy_from_slice(&timestamp_ms.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&random);
    let value = u128::from_be_bytes(bytes);

    let mut out = [0u8; 26];
    // First char encodes the top 3 bits of the 128-bit value (with
    // an implicit 2-bit zero padding on top to make a clean 130-bit
    // = 26 * 5 bit string). Its value is therefore in 0..=7.
    out[0] = ALPHABET[(value >> 125) as usize];
    // Remaining 25 chars take 5 bits each, MSB first.
    for i in 1..26 {
        let shift = 125 - i * 5;
        out[i] = ALPHABET[((value >> shift) & 0x1F) as usize];
    }
    String::from_utf8(out.to_vec()).expect("ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_produces_a_twenty_six_character_string() {
        let ulid = encode_ulid(0, [0; 10]);
        assert_eq!(ulid.len(), 26);
    }

    #[dialog_common::test]
    fn it_uses_only_crockford_base32_characters() {
        let ulid = encode_ulid(0x017A_5BD3_FE00, [0xAB; 10]);
        for c in ulid.bytes() {
            assert!(
                ALPHABET.contains(&c),
                "char {:?} not in Crockford alphabet",
                c as char,
            );
        }
    }

    #[dialog_common::test]
    fn it_encodes_all_zeros_as_all_zero_chars() {
        // Sanity check the bit layout — zero input means every
        // 5-bit chunk is zero, so every char is `0`.
        assert_eq!(encode_ulid(0, [0; 10]), "0".repeat(26));
    }

    #[dialog_common::test]
    fn it_sorts_lexicographically_by_timestamp() {
        // Two ULIDs with identical random bytes but increasing
        // timestamps must sort the same way as their timestamps.
        let a = encode_ulid(1_000_000_000_000, [0x42; 10]);
        let b = encode_ulid(1_000_000_000_001, [0x42; 10]);
        let c = encode_ulid(2_000_000_000_000, [0x42; 10]);
        assert!(a < b, "{a} not < {b}");
        assert!(b < c, "{b} not < {c}");
    }

    #[dialog_common::test]
    fn it_differs_when_random_differs_at_the_same_timestamp() {
        // Same timestamp, different randomness → different ULID.
        let ts = 1_700_000_000_000;
        let a = encode_ulid(ts, [0; 10]);
        let b = encode_ulid(ts, [0xFF; 10]);
        assert_ne!(a, b);
        // The timestamp prefix (first 10 chars) is identical.
        assert_eq!(&a[..10], &b[..10]);
    }
}
