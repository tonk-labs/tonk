//! The per-space fake origin a sealed guest believes it lives at.
//!
//! A space rendered in a sealed portal iframe is given its own synthetic
//! origin — `https://{label}.tonk.spot/` — so that navigation inside the
//! guest resolves like an ordinary web page: in-space routes are plain
//! absolute paths (`/`, `/activity`, `/activity/{id}`) under that origin,
//! and any href that escapes the origin is, by definition, external. The
//! guest sets a `<base>` to this origin and lets the browser do all URL
//! resolution; classification then reduces to an origin comparison.
//!
//! It is an ILLUSION. The document is really served from the host origin
//! (`staging.tonk.spot`) at `/space/{did}/...`; the host translates a
//! guest-world path back to the real route at the bridge. Only the guest's
//! internal coordinate system is the per-space origin.
//!
//! The `{label}` is a DNS-safe, case-insensitive encoding of the space's
//! `did:key` identifier, following the IPFS CIDv1-subdomain precedent
//! (gateways moved from `/ipfs/{cid}` paths to `{cid}.ipfs.dweb.link`
//! subdomains for the same reason — content gets its own origin). The
//! raw did suffix is base58btc (`z6Mk…`), which is case-sensitive and so
//! unusable as a DNS label; we re-encode as base32-lower (RFC4648, no
//! pad) which round-trips through case-insensitive DNS.

use multibase::Base;

/// The host suffix every space origin lives under. Purely internal (never
/// resolved by real DNS), so the literal value only has to be stable and
/// distinct from the real host origin.
const SPACE_ORIGIN_SUFFIX: &str = "tonk.spot";

/// The synthetic origin a guest rendering `space` (a `did:key` string)
/// believes it lives at, WITH a trailing slash so it is a directory base
/// (`https://{label}.tonk.spot/`). Relative in-space hrefs resolve under
/// it; the browser's own URL resolution does the rest.
///
/// Returns `None` for anything that is not a `did:key` space (e.g. the
/// profile/Hub, whose links are genuinely top-level and want the real
/// origin).
pub fn space_origin_for(space: &str) -> Option<String> {
    let label = encode_label(space)?;
    Some(format!("https://{label}.{SPACE_ORIGIN_SUFFIX}/"))
}

/// Encode a `did:key:…` string as a DNS-safe, case-insensitive label:
/// the identifier's KEY BYTES re-encoded from base58btc (`z…`, case
/// sensitive) to multibase base32-lower (`b…`, DNS-safe). Encoding the
/// bytes — not the base58 string — keeps the label under the 63-char DNS
/// limit (32 key bytes → ~52 base32 chars, vs ~77 for the string).
///
/// `None` when `space` is not a `did:key` or its identifier is not valid
/// multibase.
pub fn encode_label(space: &str) -> Option<String> {
    let mb = space.strip_prefix("did:key:")?; // e.g. `z6Mk…` (base58btc multibase)
    let (_base, bytes) = multibase::decode(mb).ok()?;
    // `multibase::encode` prefixes the base code (`b` for base32-lower), which
    // is itself a lowercase letter, so the whole label stays DNS-legal.
    Some(multibase::encode(Base::Base32Lower, bytes))
}

/// Reverse [`encode_label`]: a subdomain label back to the full
/// `did:key:…` string. The label is multibase base32-lower of the key
/// bytes; re-encode those to base58btc (the did:key canonical multibase)
/// and re-attach the `did:key:` prefix. `None` when the label is not valid
/// multibase.
pub fn decode_label(label: &str) -> Option<String> {
    let (_base, bytes) = multibase::decode(label).ok()?;
    let mb = multibase::encode(Base::Base58Btc, bytes);
    Some(format!("did:key:{mb}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_round_trips() {
        let did = "did:key:z6Mki8Mf2Trp2qmXqNoSihfVi9sEg8Z4aSCSnyUfadj4jB1E";
        let label = encode_label(did).expect("did encodes");
        // DNS-legal single label: lowercase alphanumerics only, under 63 chars.
        assert!(
            label.len() < 63,
            "label too long for a DNS label: {}",
            label.len()
        );
        assert!(label.starts_with('b'), "multibase base32-lower prefix");
        assert!(
            label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert_eq!(decode_label(&label).as_deref(), Some(did));
    }

    #[test]
    fn origin_has_trailing_slash_directory_base() {
        let origin = space_origin_for("did:key:z6MkTest").expect("origin");
        assert!(origin.starts_with("https://"));
        assert!(origin.ends_with(".tonk.spot/"));
    }

    #[test]
    fn non_did_space_has_no_origin() {
        assert_eq!(space_origin_for("profile"), None);
        assert_eq!(encode_label("not-a-did"), None);
    }
}
