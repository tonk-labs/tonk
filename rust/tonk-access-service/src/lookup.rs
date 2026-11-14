//! Lookup of customers by email address.
//!
//! An address names the customers registered under it through a `did:web`
//! DID under this service's host:
//!
//! ```text
//! did:web:tonk.network:customer:example.com:jsmith
//! ```
//!
//! The split follows [`did:mailto`][spec] — domain first, then the local
//! part percent-encoded — but under our own `did:web` name, so it
//! resolves through ordinary `did:web` resolution to this service:
//!
//! ```text
//! GET https://tonk.network/customer/example.com/jsmith/did.json
//! ```
//!
//! [spec]: https://github.com/storacha/specs/blob/main/did-mailto.md
//!
//! The address is encoded rather than hashed. Hashing it would keep the
//! plaintext out of the URL, but the input space is enumerable, so the
//! obscurity is thin, and it would force a stored hash column that D1
//! cannot backfill in SQL and where a missed row is silently unfindable.
//! The cost of encoding is that the address is plaintext wherever one of
//! these DIDs is written down — a delegation, a log line, a browser
//! history entry.
//!
//! One address holds one customer: `customer_email` is unique.

use serde_json::Value;

use crate::email::normalize_email;
use crate::service::customer_document;
use crate::store::{Store, StoreError};
use tonk_account::customer::CustomerStatus;

/// The path segment the customer namespace lives under, in both the DID
/// (`did:web:{host}:customer:...`) and the URL it resolves to
/// (`/customer/...`).
pub const CUSTOMER_SEGMENT: &str = "customer";

/// Characters that pass through a `did:mailto` local part unencoded.
/// Everything else becomes `%XX`. From the spec's `idchar` rule.
fn is_idchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

/// Percent-encode a local part per the `did:mailto` `idchar` rule.
pub fn encode_local(local: &str) -> String {
    let mut encoded = String::with_capacity(local.len());
    for byte in local.bytes() {
        if is_idchar(byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The `did:web` DID naming the customers registered under `address`,
/// under this service's `host`.
///
/// The address is normalized first, so two spellings of one address
/// produce one DID.
pub fn customer_did(host: &str, address: &str) -> Option<String> {
    let address = normalize_email(address);
    let (local, domain) = split_address(&address)?;
    Some(format!(
        "did:web:{}:{CUSTOMER_SEGMENT}:{domain}:{}",
        encode_host(host),
        encode_local(local)
    ))
}

/// A host as a single `did:web` segment.
///
/// `did:web` separates path segments with `:`, so a host carrying a port
/// has to percent-encode it — `localhost%3A8090`, per the method spec.
/// Left raw, `did:web:localhost:8090:customer:...` reads `8090` as the
/// first path segment, and a resolver fetches `https://localhost/8090/...`
/// instead of the port it was meant to reach.
pub(crate) fn encode_host(host: &str) -> String {
    host.replace(':', "%3A")
}

/// Split a normalized address into its local part and domain at the last
/// `@`, which is the separator RFC 5322 designates. `None` when either
/// side is empty or there is no `@` at all.
fn split_address(address: &str) -> Option<(&str, &str)> {
    let (local, domain) = address.rsplit_once('@')?;
    (!local.is_empty() && !domain.is_empty()).then_some((local, domain))
}

/// Rebuild an address from the `{domain}` and `{local}` path segments of
/// a `did:web` URL.
///
/// A standard `did:web` resolver copies each DID segment into the URL
/// as it stands, so the local part's own encoding reaches us intact:
/// `tag%2Balice` arrives as `tag%2Balice`, and the router hands it over
/// undecoded. Each segment is therefore percent-decoded here. Callers that
/// spell the address raw -- `tag+alice` -- still resolve, since a raw `+`
/// is not an escape and decodes to itself.
///
/// Decoding is strict `%XX` only. Form decoding would turn a `+` into a
/// space and the lookup would miss.
///
/// `None` when a segment is empty, holds a malformed escape, decodes to
/// invalid UTF-8, or holds an `@` or `/` once decoded: an `@` would make
/// the split ambiguous, and a `/` cannot appear in a segment the router
/// matched.
///
/// The result is normalized, so a DID spelled with different casing than
/// the stored row still resolves to it.
pub fn address_from_segments(domain: &str, local: &str) -> Option<String> {
    let domain = decode_segment(domain)?;
    let local = decode_segment(local)?;
    Some(normalize_email(&format!("{local}@{domain}")))
}

/// Percent-decode one path segment, refusing anything that could not have
/// come from a single address component.
fn decode_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let high = (hex[0] as char).to_digit(16)?;
            let low = (hex[1] as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.is_empty() && !decoded.contains(['@', '/'])).then_some(decoded)
}

/// The HTTP status a customer's registration state answers with.
///
/// `Registered` answers `202` rather than `200` because the address is
/// claimed but not confirmed: the DID is real and worth returning, but a
/// caller about to act on it should know the confirmation has not
/// happened. `Suspended` answers `410` because the resource existed and
/// is not currently available, which is what a suspension is, and unlike
/// `403` it does not read as a permission failure on the caller's part.
pub fn status_of(status: CustomerStatus) -> u16 {
    match status {
        CustomerStatus::Active => 200,
        CustomerStatus::Registered => 202,
        CustomerStatus::Suspended => 410,
    }
}

/// A resolved lookup: the document to answer with, and its status code.
#[derive(Debug, Clone)]
pub struct Found {
    /// The DID document naming the customer on the address.
    pub document: Value,
    /// The status to answer with, fixed by the customer's state.
    pub status: u16,
}

/// The answer for a customer row, whichever store it was read from:
/// the document built for `did` at `origin`, with the status its state
/// fixes.
pub fn found_of(did: &str, customer: &crate::store::Customer, origin: &str) -> Found {
    let suspended = customer.status == CustomerStatus::Suspended;
    Found {
        document: customer_document(did, customer, suspended, origin),
        status: status_of(customer.status),
    }
}

/// Resolve `address` against `store`, building the document for `did`.
///
/// `Ok(None)` when no customer holds the address, which the caller
/// answers as `404`: an address nobody registered and an address that
/// does not exist are the same fact.
pub async fn resolve<S: Store>(
    store: &S,
    did: &str,
    address: &str,
    origin: &str,
) -> Result<Option<Found>, StoreError> {
    let Some(customer) = store.customer_by_email(address).await? else {
        return Ok(None);
    };
    Ok(Some(found_of(did, &customer, origin)))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_encodes_an_address_as_a_did() {
        assert_eq!(
            customer_did("tonk.network", "jsmith@example.com").unwrap(),
            "did:web:tonk.network:customer:example.com:jsmith"
        );
    }

    #[dialog_common::test]
    fn it_percent_encodes_a_local_part() {
        assert_eq!(
            customer_did("tonk.network", "tag+alice@web.mail").unwrap(),
            "did:web:tonk.network:customer:web.mail:tag%2Balice"
        );
    }

    #[dialog_common::test]
    fn it_normalizes_before_encoding() {
        assert_eq!(
            customer_did("tonk.network", "  JSmith@Example.COM ").unwrap(),
            customer_did("tonk.network", "jsmith@example.com").unwrap()
        );
    }

    #[dialog_common::test]
    fn it_refuses_an_address_without_a_local_part_or_domain() {
        assert!(customer_did("tonk.network", "jsmith").is_none());
        assert!(customer_did("tonk.network", "@example.com").is_none());
        assert!(customer_did("tonk.network", "jsmith@").is_none());
    }

    #[dialog_common::test]
    fn it_splits_at_the_last_at_sign() {
        // RFC 5322 quotes a local part containing `@`; the domain never
        // holds one, so the last is the separator.
        assert_eq!(
            split_address("a@b@example.com").unwrap(),
            ("a@b", "example.com")
        );
    }

    #[dialog_common::test]
    fn it_rebuilds_an_address_from_encoded_segments() {
        // What a standard `did:web` resolver sends for
        // `did:web:host:customer:web.mail:tag%2Balice`.
        assert_eq!(
            address_from_segments("web.mail", "tag%2Balice").unwrap(),
            "tag+alice@web.mail"
        );
    }

    #[dialog_common::test]
    fn it_rebuilds_an_address_from_raw_segments() {
        // A raw `+` is not an escape, and is never read as a space.
        assert_eq!(
            address_from_segments("web.mail", "tag+alice").unwrap(),
            "tag+alice@web.mail"
        );
    }

    #[dialog_common::test]
    fn it_normalizes_after_decoding() {
        assert_eq!(
            address_from_segments("web.mail", "TAG%2BAlice").unwrap(),
            "tag+alice@web.mail"
        );
        // Lowercase hex decodes the same.
        assert_eq!(
            address_from_segments("web.mail", "tag%2balice").unwrap(),
            "tag+alice@web.mail"
        );
    }

    #[dialog_common::test]
    fn it_decodes_a_non_ascii_local_part() {
        assert_eq!(
            address_from_segments("example.com", "jos%C3%A9").unwrap(),
            "jos\u{e9}@example.com"
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_separator_once_decoded() {
        assert!(address_from_segments("example.com", "a%40b").is_none());
        assert!(address_from_segments("example%40com", "jsmith").is_none());
        assert!(address_from_segments("example.com", "a%2Fb").is_none());
    }

    #[dialog_common::test]
    fn it_refuses_a_malformed_escape() {
        assert!(address_from_segments("example.com", "a%").is_none());
        assert!(address_from_segments("example.com", "a%4").is_none());
        assert!(address_from_segments("example.com", "a%zzb").is_none());
    }

    #[dialog_common::test]
    fn it_refuses_a_segment_that_decodes_to_invalid_utf8() {
        assert!(address_from_segments("example.com", "a%FFb").is_none());
        // A truncated two-byte sequence.
        assert!(address_from_segments("example.com", "jos%C3").is_none());
    }

    #[dialog_common::test]
    fn it_refuses_a_segment_that_decodes_to_nothing() {
        // Not reachable through the router, but nothing here relies on it.
        assert!(address_from_segments("example.com", "").is_none());
    }

    #[dialog_common::test]
    fn it_round_trips_an_address_through_a_did() {
        for address in [
            "jsmith@example.com",
            "tag+alice@web.mail",
            "a.b-c_d@sub.example.org",
            // A literal `%`, which encodes to `%25` and decodes once back
            // to itself.
            "a%b@example.com",
            // Byte-wise encoding over UTF-8, so a non-ASCII local part
            // survives as its encoded bytes.
            "josé@example.com",
            "!#$&'*=?^`{|}~@example.com",
        ] {
            let did = customer_did("tonk.network", address).unwrap();
            // A resolver copies the encoded segment into the URL as is.
            let encoded = did.rsplit(':').next().unwrap();
            let domain = did.split(':').nth(4).unwrap();
            assert_eq!(
                address_from_segments(domain, encoded).unwrap(),
                address,
                "round trip failed for {address}"
            );
        }
    }

    #[dialog_common::test]
    fn it_normalizes_rebuilt_segments() {
        assert_eq!(
            address_from_segments("Example.COM", "JSmith").unwrap(),
            "jsmith@example.com"
        );
    }

    /// A `/` in a local part encodes to `%2F`, so the DID keeps one
    /// segment for the local part. Leaving it raw would split the path
    /// and route the request somewhere else entirely. Like a quoted `@`,
    /// such an address encodes but does not resolve.
    #[dialog_common::test]
    fn it_encodes_a_separator_that_would_break_the_path() {
        let did = customer_did("tonk.network", "a/b@example.com").unwrap();
        assert_eq!(did, "did:web:tonk.network:customer:example.com:a%2Fb");
        assert!(
            !did.trim_start_matches("did:web:tonk.network:")
                .contains('/'),
            "no raw separator survives into the DID"
        );
    }

    /// A literal `%` encodes to `%25`, so one decoding pass returns the
    /// `%` itself rather than consuming the two characters after it.
    #[dialog_common::test]
    fn it_encodes_a_literal_percent() {
        assert_eq!(
            customer_did("tonk.network", "a%b@example.com").unwrap(),
            "did:web:tonk.network:customer:example.com:a%25b"
        );
    }

    /// An address whose local part holds a quoted `@` encodes, but does
    /// not resolve: the rebuilt segments refuse an `@` because it would
    /// make the split ambiguous. Encoding and resolution disagree here,
    /// and the disagreement is deliberate -- such an address is
    /// vanishingly rare, and refusing it is better than resolving it to
    /// the wrong customer.
    #[dialog_common::test]
    fn it_does_not_resolve_an_at_sign_inside_a_local_part() {
        assert!(customer_did("tonk.network", "a@b@example.com").is_some());
        assert!(address_from_segments("example.com", "a@b").is_none());
    }

    #[dialog_common::test]
    fn it_refuses_empty_or_malformed_segments() {
        assert!(address_from_segments("", "jsmith").is_none());
        assert!(address_from_segments("example.com", "").is_none());
        assert!(address_from_segments("exa@mple.com", "jsmith").is_none());
    }

    #[dialog_common::test]
    fn it_maps_each_registration_state_to_its_status() {
        assert_eq!(status_of(CustomerStatus::Active), 200);
        assert_eq!(status_of(CustomerStatus::Registered), 202);
        assert_eq!(status_of(CustomerStatus::Suspended), 410);
    }
}
