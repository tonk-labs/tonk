use super::*;

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
