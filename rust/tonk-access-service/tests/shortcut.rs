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
    let shortcut = Shortcut::new(b"/join?access=abc&remote=https%3A%2F%2Ftonk.network").unwrap();
    assert_eq!(
        shortcut.target,
        "/join?access=abc&remote=https%3A%2F%2Ftonk.network"
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
        let target = "/join?access=abc123&remote=https%3A%2F%2Ftonk.network%2Fucan%2F";
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

        // Public campaign tags added to the short URL survive expansion;
        // capability and space-attribution overrides do not.
        let response = client
            .get(format!(
                "{}/@/{hash}?tonk_source=reddit&tonk_channel=outreach&access=EVIL&tonk_space=EVIL",
                env.access_service_url
            ))
            .send()
            .await?;
        assert_eq!(response.status(), 301);
        let location = response
            .headers()
            .get("Location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let redirected = url::Url::parse(&format!("https://tonk.invalid{location}"))?;
        let query_value = |name: &str| {
            redirected
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        assert_eq!(query_value("access").as_deref(), Some("abc123"));
        assert_eq!(query_value("tonk_source").as_deref(), Some("reddit"));
        assert_eq!(query_value("tonk_channel").as_deref(), Some("outreach"));
        assert_eq!(query_value("tonk_space"), None);

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

    /// The shortcut routes are preflighted too, and their preflight
    /// carries the same cache lifetime as the UCAN one.
    #[dialog_common::test]
    async fn it_caches_the_shortcut_preflight(env: AccessServiceAddress) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let hash = Shortcut::new(b"/join?access=abc").unwrap().hash_str();

        for path in ["/@", &format!("/@/{hash}")] {
            let response = client
                .request(
                    reqwest::Method::OPTIONS,
                    format!("{}{path}", env.access_service_url),
                )
                .send()
                .await?;
            assert_eq!(response.status(), 204, "{path}");
            assert_eq!(
                response
                    .headers()
                    .get("Access-Control-Max-Age")
                    .and_then(|value| value.to_str().ok()),
                Some("86400"),
                "{path} preflight must carry a cache lifetime",
            );
        }

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
        assert_eq!(
            response
                .headers()
                .get("Content-Type")
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8"),
        );
        assert_eq!(
            response
                .headers()
                .get("Cache-Control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store"),
        );
        let body = response.text().await?;
        assert!(
            body.contains("This share link is no longer available"),
            "unexpected expired-link page: {body}",
        );
        assert!(
            body.contains("expire after 7 days"),
            "the page must explain the default lifetime: {body}",
        );
        assert!(
            body.contains("Ask the person who shared it with you to create a new one"),
            "the page must give the recipient a recovery action: {body}",
        );

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

    /// A real invite URL survives the shorten → redirect → reassemble
    /// round trip: following the short link's 301 the way a browser does
    /// reproduces the original long URL exactly, secret fragment included.
    ///
    /// This is the seam the mint handler depends on, driven through the
    /// same [`ShortcutRequest`] glue it uses. The two halves it guards:
    ///
    /// - The seed (`#…`) is never PUT — it must not reach the server, and
    ///   the service must not echo it back in `Location`. It survives only
    ///   because the browser re-attaches the *short* link's fragment to
    ///   the redirect target (RFC 7231 fragment inheritance).
    /// - The `Location` is relative, so it resolves against the origin the
    ///   user is on, not the service's.
    ///
    /// [`ShortcutRequest`]: tonk_invite::shortcut::ShortcutRequest
    #[dialog_common::test]
    async fn it_round_trips_an_invite_url(env: AccessServiceAddress) -> anyhow::Result<()> {
        use tonk_invite::shortcut::ShortcutRequest;

        let origin = env.access_service_url.trim_end_matches('/');
        let seed = "BsBZG8W93RD527tyLRA142rcewpggxBsRSUe5eEDEmGh";
        let long = format!(
            "{origin}/join\
             ?access=2DoZVyzdw3q5w9WQ1MJgH4zwGuhKdryoLUgVLYhkbA5N\
             &remote=http%3A%2F%2Flocalhost%3A8080%2Fucan%2F\
             #{seed}"
        );

        let request = ShortcutRequest::new(&long)?;

        // The secret never goes on the wire: only the path + query is PUT.
        assert!(
            !request.target.contains(seed),
            "the seed must not be sent to the service, got {:?}",
            request.target,
        );

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let response = client
            .put(request.endpoint.as_str())
            .body(request.target.clone())
            .send()
            .await?;
        assert_eq!(response.status(), 200);
        let hash = response.text().await?;

        // The short link keeps the fragment; the stored part does not.
        let short = request.short_url(&hash)?;
        assert!(
            short.starts_with(&format!("{origin}/@/{hash}")),
            "short URL should be the shortcut endpoint, got {short}",
        );
        assert!(
            short.ends_with(&format!("#{seed}")),
            "short URL must carry the seed in its fragment, got {short}",
        );
        assert!(
            short.len() < long.len(),
            "the short URL should be shorter ({} vs {})",
            short.len(),
            long.len(),
        );

        // Follow the redirect the way a browser does, and reassemble.
        let response = client.get(format!("{origin}/@/{hash}")).send().await?;
        assert_eq!(response.status(), 301);
        let location = response
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            location.starts_with('/'),
            "Location must be relative so it resolves against the user's \
             origin, got {location}",
        );
        assert!(
            !location.contains('#'),
            "the service must not echo a fragment; the browser inherits the \
             short link's own. Got {location}",
        );

        // Fragment inheritance: the browser carries the short link's `#seed`
        // onto the relative target. That reconstructs the original URL.
        let rebuilt = format!("{origin}{location}#{seed}");
        assert_eq!(
            rebuilt, long,
            "following the short link must reproduce the original invite URL",
        );

        Ok(())
    }
}
