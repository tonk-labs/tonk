//! Canonical path-segment encoding for scoped language-server authority.

/// Encode one repository, profile, or branch identity as a single canonical
/// URI/HTTP path segment.
///
/// RFC 3986 unreserved ASCII bytes remain readable. Every other UTF-8 byte is
/// written as an uppercase percent triplet, so legal identities such as
/// `did:key:zAlice` and `feat/artifact` cannot change route structure.
/// Authority boundaries validate legality by decoding the result; empty,
/// control-bearing, and complete dot-segment values intentionally do not
/// round-trip.
pub fn encode_lsp_scope_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

/// Decode one canonical language-server authority segment.
///
/// Non-canonical aliases are refused: percent hex must be uppercase, an
/// unreserved byte must not be escaped, reserved bytes must not appear raw,
/// decoded text must be non-empty UTF-8 without control characters, and the
/// complete dot segments normalized by URL parsers are not legal identities.
pub fn decode_lsp_scope_segment(segment: &str) -> Option<String> {
    if segment.is_empty() {
        return None;
    }

    let input = segment.as_bytes();
    let mut decoded = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        let byte = input[index];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            decoded.push(byte);
            index += 1;
            continue;
        }
        if byte != b'%' || index + 2 >= input.len() {
            return None;
        }
        decoded.push((uppercase_hex(input[index + 1])? << 4) | uppercase_hex(input[index + 2])?);
        index += 3;
    }

    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.is_empty()
        || matches!(decoded.as_str(), "." | "..")
        || decoded.chars().any(char::is_control)
        || encode_lsp_scope_segment(&decoded) != segment
    {
        return None;
    }
    Some(decoded)
}

fn uppercase_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_lsp_scope_segment, encode_lsp_scope_segment};

    #[test]
    fn slash_branch_and_repository_identity_have_one_reversible_spelling() {
        for (identity, encoded) in [
            ("did:key:zAlice", "did%3Akey%3AzAlice"),
            ("feat/artifact", "feat%2Fartifact"),
            ("Jack Douglas", "Jack%20Douglas"),
            ("café", "caf%C3%A9"),
        ] {
            assert_eq!(encode_lsp_scope_segment(identity), encoded);
            assert_eq!(decode_lsp_scope_segment(encoded).as_deref(), Some(identity));
        }
    }

    #[test]
    fn aliases_and_route_structure_are_rejected() {
        for invalid in [
            "",
            "feat/artifact",
            "feat%2fartifact",
            "feat%2F%61rtifact",
            "did:key:zAlice",
            "%",
            "%GG",
            "%00",
            ".",
            "..",
        ] {
            assert_eq!(decode_lsp_scope_segment(invalid), None, "accepts {invalid}");
        }
    }
}
