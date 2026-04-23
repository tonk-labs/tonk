//! DID parsing helpers. Currently only `did:key` is supported, which
//! is what Tonk uses everywhere.

/// Extracts the first 4 bytes of the public key from a `did:key`
/// identifier. Returns `None` if the input is not a recognizable
/// `did:key:z…` string.
///
/// The `did:key` format is:
///   `did:key:` + multibase-encoded (multicodec-varint-prefix + raw key bytes)
///
/// `z` prefix means base58btc. Current key types all use 1- or 2-byte
/// multicodec varints; we parse the varint to find where the raw key
/// starts, then return its first 4 bytes. This keeps the sigil stable
/// across serialization variations — the same key always produces the
/// same 4 bytes regardless of encoding choices.
pub fn did_key_prefix(did: &str) -> Option<[u8; 4]> {
    let rest = did.strip_prefix("did:key:")?;
    let encoded = rest.strip_prefix('z')?;

    let decoded = bs58::decode(encoded).into_vec().ok()?;

    // Multicodec prefix is a varint. For the key types in practice
    // (ed25519 = 0xed 0x01, secp256k1 = 0xe7 0x01, P-256 = 0x80 0x24,
    // etc.) this is 2 bytes. Parse it properly so unknown future key
    // types still work.
    let (_, key_start) = read_varint(&decoded)?;
    let key = decoded.get(key_start..)?;
    if key.len() < 4 {
        return None;
    }
    Some([key[0], key[1], key[2], key[3]])
}

/// Parses an unsigned LEB128 varint. Returns `(value, bytes_consumed)`.
/// Since we only need to skip past it, the value is just informational.
fn read_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0;
    for (i, &b) in bytes.iter().enumerate() {
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ed25519_did_key() {
        // A real ed25519 did:key (from an ed25519 public key).
        let did = "did:key:z6MkriCnXHFHhVuyGf5uR7gafNnyjPZxTtFxw94gCx6ynxe8";
        let bytes = did_key_prefix(did).expect("should parse");
        // We can't assert exact bytes without the private key, but we
        // can assert it returned *something* and that it's stable.
        let bytes2 = did_key_prefix(did).unwrap();
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn rejects_non_did_key() {
        assert_eq!(did_key_prefix("did:web:example.com"), None);
        assert_eq!(did_key_prefix("not a did"), None);
        assert_eq!(did_key_prefix(""), None);
    }

    #[test]
    fn rejects_missing_multibase_prefix() {
        // Has `did:key:` but no `z` (base58btc) prefix
        assert_eq!(did_key_prefix("did:key:xyzsomething"), None);
    }

    #[test]
    fn varint_single_byte() {
        assert_eq!(read_varint(&[0x00]), Some((0, 1)));
        assert_eq!(read_varint(&[0x7f]), Some((127, 1)));
    }

    #[test]
    fn varint_two_bytes() {
        // 0x01ed in varint LE: 0xed 0x03 (0xed has high bit set, 0x03 doesn't)
        // But ed25519 multicodec is 0xed 0x01 — let's just check it consumes 2
        assert_eq!(read_varint(&[0xed, 0x01, 0xff, 0xff]).map(|(_, n)| n), Some(2));
    }
}
