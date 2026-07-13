//! Tests for the shortcut service: validation core and endpoints.
//!
//! Native-only, like the crate's other test suite: the library links
//! the Cloudflare `worker` runtime, so its wasm build only
//! instantiates inside workerd — the browser harness the web CI leg
//! runs tests in can't load it.
//!
//! The HTTP round trip runs against the local test server and is gated
//! like the other integration tests:
//!
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test shortcut
//! ```

#![cfg(not(target_arch = "wasm32"))]

use tonk_access_service::shortcut::{
    DEFAULT_TTL_DAYS, KEY_PREFIX, MAX_TARGET_SIZE, MAX_TTL_DAYS, SECONDS_PER_DAY, Shortcut,
    object_key_for, requested_ttl,
};

#[dialog_common::test]
fn it_accepts_a_rooted_path_with_query() {
    let shortcut = Shortcut::new(b"/join?access=abc&remote=https%3A%2F%2Fhub.tonk.xyz").unwrap();
    assert_eq!(
        shortcut.target,
        "/join?access=abc&remote=https%3A%2F%2Fhub.tonk.xyz"
    );
    assert!(shortcut.object_key().starts_with(KEY_PREFIX));
    assert_eq!(
        object_key_for(&shortcut.hash_str()).unwrap(),
        shortcut.object_key()
    );
}

#[dialog_common::test]
fn it_hashes_deterministically() {
    let a = Shortcut::new(b"/join?access=abc").unwrap();
    let b = Shortcut::new(b"/join?access=abc").unwrap();
    let c = Shortcut::new(b"/join?access=xyz").unwrap();
    assert_eq!(a.hash_str(), b.hash_str());
    assert_ne!(a.hash_str(), c.hash_str());
}

#[dialog_common::test]
fn it_rejects_targets_that_could_leave_the_origin() {
    for target in [
        "https://evil.example/join",
        "//evil.example/join",
        "/\\evil.example/join",
        "join?access=abc",
        "../join",
    ] {
        assert!(
            Shortcut::new(target.as_bytes()).is_err(),
            "{target:?} must be rejected"
        );
    }
}

#[dialog_common::test]
fn it_rejects_fragments_and_header_injection() {
    // A fragment in `Location` would override the inheritance the
    // seed relies on; control characters could smuggle headers.
    assert!(Shortcut::new(b"/join?access=abc#seed").is_err());
    assert!(Shortcut::new(b"/join?a=1\r\nSet-Cookie: x=1").is_err());
    assert!(Shortcut::new(&[0x2f, 0xff, 0xfe]).is_err(), "non-UTF-8");
}

#[dialog_common::test]
fn it_rejects_oversized_targets() {
    let mut target = b"/join?access=".to_vec();
    target.resize(MAX_TARGET_SIZE + 1, b'a');
    let err = Shortcut::new(&target).unwrap_err();
    assert!(err.contains("exceeding"), "{err}");
}

#[dialog_common::test]
fn it_defaults_and_caps_the_requested_ttl() {
    let default = DEFAULT_TTL_DAYS * SECONDS_PER_DAY;
    assert_eq!(requested_ttl(None).unwrap(), default);
    assert_eq!(requested_ttl(Some("")).unwrap(), default);
    assert_eq!(requested_ttl(Some("other=1")).unwrap(), default);
    assert_eq!(requested_ttl(Some("ttl=3")).unwrap(), 3 * SECONDS_PER_DAY);
    assert_eq!(
        requested_ttl(Some("ttl=999999")).unwrap(),
        MAX_TTL_DAYS * SECONDS_PER_DAY
    );
    assert!(requested_ttl(Some("ttl=soon")).is_err());
    assert!(requested_ttl(Some("ttl=-1")).is_err());
}

#[dialog_common::test]
fn it_rejects_malformed_hash_segments() {
    assert!(object_key_for("not-base58-!!!").is_err());
    assert!(
        object_key_for("3vQB7B6MdGQZcSvtzcXAyC").is_err(),
        "wrong length"
    );
    assert!(object_key_for("").is_err());
}

/// HTTP round trip against the local test server.
#[cfg(feature = "integration-tests")]
mod http {
    use super::*;
    use tonk_access_service::helpers::AccessServiceAddress;

    #[dialog_common::test]
    async fn it_stores_and_redirects(env: AccessServiceAddress) -> anyhow::Result<()> {
        let target = "/join?access=abc123&remote=https%3A%2F%2Fhub.tonk.xyz%2Fucan%2F";
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        // Store — twice, to confirm idempotency.
        let put = || async {
            client
                .put(format!("{}/@", env.access_service_url))
                .body(target)
                .send()
                .await
        };
        let first = put().await?;
        assert_eq!(first.status(), 200, "{}", first.text().await?);
        let hash = put().await?.text().await?;
        assert_eq!(hash, Shortcut::new(target.as_bytes()).unwrap().hash_str());

        // Redirect: 301 with the stored target as a *relative* Location
        // and no fragment, so the short link's own fragment inherits.
        let response = client
            .get(format!("{}/@/{hash}", env.access_service_url))
            .send()
            .await?;
        assert_eq!(response.status(), 301);
        let location = response
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert_eq!(location, target);

        // Unknown (well-formed) hash → 404; malformed → 400.
        let missing = Shortcut::new(b"/never-stored").unwrap().hash_str();
        let response = client
            .get(format!("{}/@/{missing}", env.access_service_url))
            .send()
            .await?;
        assert_eq!(response.status(), 404);
        let response = client
            .get(format!("{}/@/!!!", env.access_service_url))
            .send()
            .await?;
        assert_eq!(response.status(), 400);

        // Invalid targets and TTLs are refused.
        let response = client
            .put(format!("{}/@", env.access_service_url))
            .body("https://evil.example/join")
            .send()
            .await?;
        assert_eq!(response.status(), 400);
        let response = client
            .put(format!("{}/@?ttl=soon", env.access_service_url))
            .body("/join?access=abc")
            .send()
            .await?;
        assert_eq!(response.status(), 400);

        Ok(())
    }

    /// A `ttl=0` shortcut is stored already-expired: the logical
    /// expiry check makes the very next read a 404.
    #[dialog_common::test]
    async fn it_expires_shortcuts_after_their_ttl(env: AccessServiceAddress) -> anyhow::Result<()> {
        let target = "/join?access=expiring";
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let hash = client
            .put(format!("{}/@?ttl=0", env.access_service_url))
            .body(target)
            .send()
            .await?
            .text()
            .await?;
        let response = client
            .get(format!("{}/@/{hash}", env.access_service_url))
            .send()
            .await?;
        assert_eq!(response.status(), 404);

        // Re-publishing with a fresh TTL revives it — same key, same
        // content, refreshed expiry.
        let refreshed = client
            .put(format!("{}/@?ttl=1", env.access_service_url))
            .body(target)
            .send()
            .await?
            .text()
            .await?;
        assert_eq!(refreshed, hash);
        let response = client
            .get(format!("{}/@/{hash}", env.access_service_url))
            .send()
            .await?;
        assert_eq!(response.status(), 301);

        Ok(())
    }
}
