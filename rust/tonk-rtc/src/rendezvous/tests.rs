use super::*;

/// Where the certificate is served from, for a browser that would
/// rather hash 340 bytes than carry an elliptic-curve library.
const ASSET: &str = "assets/rendezvous.der";

/// The served certificate is the derived one.
///
/// The browser takes SHA-256 of this file to get the fingerprint, which
/// is one WebCrypto call and no crypto library. That only works while
/// the file *is* what the phrase derives, so this is what keeps it
/// honest. Run with `TONK_REWRITE_RENDEZVOUS=1` to regenerate after
/// changing the phrase.
#[test]
fn the_served_certificate_is_the_derived_one() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ASSET);
    let derived = certificate_der(RENDEZVOUS).unwrap();

    if std::env::var_os("TONK_REWRITE_RENDEZVOUS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &derived).unwrap();
    }

    let served = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("could not read {ASSET}: {error}. Regenerate with TONK_REWRITE_RENDEZVOUS=1")
    });

    assert_eq!(
        served, derived,
        "{ASSET} is not what {RENDEZVOUS} derives, so the browser would hash the wrong \
         certificate and every dial would fail the DTLS check. Regenerate with \
         TONK_REWRITE_RENDEZVOUS=1"
    );
}

/// The property everything rests on: same phrase, same bytes, every
/// time and on every target.
#[test]
fn a_phrase_yields_one_certificate() {
    assert_eq!(certificate_der(RENDEZVOUS), certificate_der(RENDEZVOUS));
    assert_eq!(fingerprint(RENDEZVOUS), fingerprint(RENDEZVOUS));
}

/// And a different phrase yields a different one, or "versioned" would
/// mean nothing.
#[test]
fn a_different_phrase_yields_a_different_certificate() {
    assert_ne!(
        fingerprint("tonk/rtc/rendezvous/v1").unwrap(),
        fingerprint("tonk/rtc/rendezvous/v2").unwrap()
    );
}

/// Why this module exists rather than deriving a key and letting rcgen
/// sign it. If this ever fails, rcgen's default signer became
/// deterministic and `Deterministic` could be dropped.
#[test]
fn a_certificate_is_not_reproducible_with_a_random_nonce() {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let params = || {
        let mut params = rcgen::CertificateParams::new(vec![SUBJECT.to_owned()]).unwrap();
        params.serial_number = Some(rcgen::SerialNumber::from(SERIAL));
        params
    };

    assert_ne!(
        params().self_signed(&key).unwrap().der(),
        params().self_signed(&key).unwrap().der(),
        "rcgen signs deterministically now; the RFC 6979 signer here is redundant"
    );
}

#[test]
fn the_fingerprint_is_the_shape_an_sdp_line_takes() {
    let fingerprint = fingerprint(RENDEZVOUS).unwrap();
    let (algorithm, value) = fingerprint.split_once(' ').expect("algorithm and value");
    assert_eq!(algorithm, "sha-256");
    assert_eq!(value.split(':').count(), 32, "sha-256 is 32 octets");
}

/// A derived port has to be dialable and out of the registered range,
/// or deriving it would trade one problem for a worse one.
#[test]
fn the_derived_port_is_in_the_dynamic_range() {
    for phrase in ["tonk/rtc/rendezvous/v1", "tonk/rtc/rendezvous/v2", ""] {
        let port = port(phrase);
        assert!((49152..=65535).contains(&port), "{phrase} gave {port}");
    }
    assert_eq!(port(RENDEZVOUS), port(RENDEZVOUS));
}

/// The certificate's own validity, read back from what was built.
/// rcgen's defaults are 1975 and 4096; this pins that nothing moved
/// them, because an expiring rendezvous would fail everywhere at once.
#[test]
fn the_certificate_outlives_the_question_of_expiry() {
    let der = certificate_der(RENDEZVOUS).unwrap();
    // 4096 as a GeneralizedTime appears literally in the DER.
    let text = der.windows(4).any(|window| window == b"4096");
    assert!(text, "expected a not_after in 4096");
}

/// The property this module rests on, and the one most likely to be
/// taken away by a dependency change rather than by a code change.
///
/// Ed25519 is deterministic by *specification* — RFC 8032 derives the
/// nonce from the key and the message, so every conforming
/// implementation agrees. ECDSA is not: the standard wants a random
/// nonce, and repeating one leaks the key. P-256 is deterministic here
/// only because RustCrypto's `ecdsa` chooses RFC 6979 as its default,
/// which is a crate policy and not a guarantee anyone else honours —
/// `ring`, which rcgen reaches for by default, does not, as the test
/// above shows.
///
/// So the certificate is reproducible because of a choice made in a
/// dependency. That is worth an assertion of its own: swap the
/// implementation and dials would fail the DTLS check with nothing to
/// read, and nothing else here would notice.
///
/// P-256 rather than Ed25519 is not a preference — it is what browser
/// DTLS stacks accept, and this certificate exists to be checked by a
/// browser. Ed25519 having the stronger guarantee is of no use if the
/// far end refuses the certificate.
#[test]
fn p256_signs_deterministically() {
    use p256::ecdsa::{SigningKey, signature::Signer};

    let key = SigningKey::from_bytes(&sha2::Sha256::digest(b"probe")).unwrap();
    let once: p256::ecdsa::Signature = key.sign(b"message");
    let twice: p256::ecdsa::Signature = key.sign(b"message");

    assert_eq!(
        once, twice,
        "p256 no longer signs with RFC 6979, so the rendezvous certificate is no longer \
         reproducible and every dial will fail the DTLS check"
    );
}

/// Both ends derive the same peer name, which is what makes a route
/// match at all — and a different phrase names a different peer.
#[test]
fn the_transport_tag_is_a_function_of_the_phrase_and_the_side() {
    assert_eq!(
        transport_tag(RENDEZVOUS, Side::Listener),
        transport_tag(RENDEZVOUS, Side::Listener)
    );
    assert_eq!(transport_tag(RENDEZVOUS, Side::Listener).len(), 16);

    // The two sides must differ, or a listener would attach its dialer
    // under its own address and a reply would alias the request.
    assert_ne!(
        transport_tag(RENDEZVOUS, Side::Listener),
        transport_tag(RENDEZVOUS, Side::Dialer)
    );
    assert_ne!(
        transport_tag(RENDEZVOUS, Side::Listener),
        transport_tag("tonk/rtc/rendezvous/v2", Side::Listener)
    );
}
