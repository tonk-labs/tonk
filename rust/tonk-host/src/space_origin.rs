//! The per-space fake origin a sealed guest believes it lives at.
//!
//! A space rendered in a sealed portal iframe is given its own synthetic
//! origin — `https://{label}.tonk.network/` — so that navigation inside the
//! guest resolves like an ordinary web page: in-space routes are plain
//! absolute paths (`/`, `/activity`, `/activity/{id}`) under that origin,
//! and any href that escapes the origin is, by definition, external. The
//! guest sets a `<base>` to this origin and lets the browser do all URL
//! resolution; classification then reduces to an origin comparison.
//!
//! It is an ILLUSION. The document is really served from the host origin
//! (`staging.tonk.xyz`) at `/space/{did}/...`; the host translates a
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
const SPACE_ORIGIN_SUFFIX: &str = "tonk.network";

/// The synthetic origin a guest rendering `space` (a `did:key` string)
/// believes it lives at, WITH a trailing slash so it is a directory base
/// (`https://{label}.tonk.network/`). Relative in-space hrefs resolve under
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

/// The longest a single DNS label may be.
const MAX_LABEL_LENGTH: usize = 63;

/// The hostname a site with `label` renders at under `pattern`, a hostname
/// with `*` where the label goes: `*.tonk.spot`, or `*-pr33.tonk.spot` for a
/// deployment that puts a suffix after it.
///
/// A suffix shares the label's 63 characters, so a label too long to fit
/// beside it is cut short. Nothing reads a space back out of its hostname
/// (its worker is told which space it holds), so the label only has to tell
/// spaces apart, and each character it keeps carries five bits of the key:
/// fifty still leave 250.
pub fn site_hostname(pattern: &str, label: &str) -> String {
    let (before, after) = pattern.split_once('*').unwrap_or(("", pattern));
    let suffix = after.split('.').next().unwrap_or("");
    let room = MAX_LABEL_LENGTH.saturating_sub(before.len() + suffix.len());
    let kept: String = label.chars().take(room).collect();
    format!("{before}{kept}{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DID: &str = "did:key:z6Mki8Mf2Trp2qmXqNoSihfVi9sEg8Z4aSCSnyUfadj4jB1E";

    fn first_label(hostname: &str) -> &str {
        hostname.split('.').next().unwrap()
    }

    #[test]
    fn it_puts_a_label_under_a_plain_host() {
        let label = encode_label(DID).unwrap();
        assert_eq!(
            site_hostname("*.tonk.spot", &label),
            format!("{label}.tonk.spot")
        );
        assert_eq!(
            site_hostname("*.localhost:8080", "profile"),
            "profile.localhost:8080"
        );
    }

    #[test]
    fn it_keeps_a_whole_label_beside_a_suffix_that_fits() {
        let label = encode_label(DID).unwrap();
        // 56 characters and `-pr9999` make 63: the last that fits whole.
        let hostname = site_hostname("*-pr9999.tonk.spot", &label);
        assert_eq!(hostname, format!("{label}-pr9999.tonk.spot"));
        assert_eq!(first_label(&hostname).len(), 63);
        assert_eq!(
            site_hostname("*-pr33.tonk.spot", "profile"),
            "profile-pr33.tonk.spot"
        );
    }

    #[test]
    fn it_cuts_a_label_short_to_fit_beside_a_long_suffix() {
        let label = encode_label(DID).unwrap();
        let hostname = site_hostname("*-pr12345.tonk.spot", &label);
        assert_eq!(first_label(&hostname).len(), 63);
        assert!(hostname.ends_with("-pr12345.tonk.spot"));
        assert!(label.starts_with(first_label(&hostname).trim_end_matches("-pr12345")));
        // Two spaces still land on different origins.
        let other =
            encode_label("did:key:z6MkkAKBuUTy2r88au4Ehu6uUwdRRpDYnKd1euvreZi3YG7M").unwrap();
        assert_ne!(hostname, site_hostname("*-pr12345.tonk.spot", &other));
    }

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
        assert!(origin.ends_with(".tonk.network/"));
    }

    #[test]
    fn non_did_space_has_no_origin() {
        assert_eq!(space_origin_for("profile"), None);
        assert_eq!(encode_label("not-a-did"), None);
    }
}
