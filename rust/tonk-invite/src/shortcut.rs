//! Client-side glue for the shortcut service.
//!
//! The shortcut service (`tonk-access-service`) is a permissionless,
//! same-origin URL shortener: `PUT /@` stores a path + query string
//! under its blake3 hash, `GET /@/{hash}` answers with a permanent
//! redirect whose relative `Location` is the stored string. A long
//! invite URL shortens to:
//!
//! ```text
//! <origin>/@/{hash}#<base58-seed>
//! ```
//!
//! The seed stays in the fragment: browsers carry it across the
//! redirect via RFC 7231 §7.1.2 fragment inheritance, and it never
//! reaches the server. Non-browser claimers resolve the redirect by
//! hand with [`resolve_location`].
//!
//! This module derives the pieces (what to store, where to PUT, how to
//! assemble the short link); the HTTP itself stays with the callers.

use anyhow::{Context, Result};
use url::Url;

/// How long one leg of a shortcut attempt may take, in milliseconds.
///
/// The service answers a `PUT /@` in ~10ms, so this is a hang detector
/// rather than a budget: a host that stops answering (a captive portal,
/// a stalled origin, a dropped connection) must not pin a mint on a
/// convenience when the long URL is already complete.
///
/// Lives here because both mint paths — the worker's share control and
/// the CLI's `tonk invite` — shorten through this module's glue, and a
/// timeout only one of them honours is how the CLI came to be able to
/// hang where the browser could not.
pub const TIMEOUT_MS: u32 = 2_000;

/// The pieces needed to shorten a URL: what to store, where to store
/// it, and how to assemble the short link from the returned hash.
#[derive(Debug, Clone)]
pub struct ShortcutRequest {
    /// `PUT` endpoint (`{origin}/@`) on the link's own origin — the
    /// only origin that can serve the relative redirect back.
    pub endpoint: Url,
    /// The path + query string to store; never carries the fragment.
    pub target: String,
    /// The link's origin, root-pathed.
    origin: Url,
    /// Fragment to re-attach to the short link (without `#`).
    fragment: Option<String>,
}

impl ShortcutRequest {
    /// Derive the shortcut request for a long URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL doesn't parse or has no usable
    /// origin.
    pub fn new(url: &str) -> Result<Self> {
        let parsed = Url::parse(url).context("shortcut source is not a valid URL")?;
        let origin = parsed
            .join("/")
            .context("shortcut source has no usable origin")?;
        let target = match parsed.query() {
            Some(query) => format!("{}?{}", parsed.path(), query),
            None => parsed.path().to_string(),
        };
        let endpoint = origin
            .join("@")
            .context("failed to derive the shortcut endpoint")?;
        Ok(Self {
            endpoint,
            target,
            origin,
            fragment: parsed.fragment().map(str::to_string),
        })
    }

    /// The hash a conforming shortcut service must answer with: the
    /// store is content-addressed (`base58(blake3(target))`), so the
    /// client knows the only correct answer before asking.
    pub fn expected_hash(&self) -> String {
        bs58::encode(blake3::hash(self.target.as_bytes()).as_bytes()).into_string()
    }

    /// The fragment-free probe URL for a stored shortcut: `GET`ting it
    /// against a conforming service answers with a redirect whose
    /// resolved location [`Self::verify_resolved`] accepts. A host that
    /// merely stored the `PUT /@` bytes (a content-addressed blob store
    /// answers with the same blake3 hash a shortener does) serves the
    /// bytes back instead of redirecting, and the probe is what tells
    /// the two apart — such a host "does not provide shortening" and
    /// the caller falls back to the full URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the origin and hash fail to join into a URL.
    pub fn probe_url(&self, hash: &str) -> Result<String> {
        self.origin
            .join(&format!("@/{}", hash.trim()))
            .map(String::from)
            .context("failed to assemble the shortcut probe URL")
    }

    /// Whether `resolved` — the URL a probe's redirect landed on — is
    /// this shortcut's own target on its own origin.
    ///
    /// # Errors
    ///
    /// Returns an error if `resolved` doesn't parse, is on a different
    /// origin, or names a different path + query than the stored target.
    pub fn verify_resolved(&self, resolved: &str) -> Result<()> {
        let parsed = Url::parse(resolved).context("the resolved shortcut is not a valid URL")?;
        anyhow::ensure!(
            parsed.origin() == self.origin.origin(),
            "the shortcut redirected off-origin, to '{resolved}'"
        );
        let landed = match parsed.query() {
            Some(query) => format!("{}?{}", parsed.path(), query),
            None => parsed.path().to_string(),
        };
        anyhow::ensure!(
            landed == self.target,
            "the shortcut redirected to '{landed}', not the stored target"
        );
        Ok(())
    }

    /// Assemble the short link from the hash the service returned,
    /// re-attaching the source URL's fragment.
    ///
    /// # Errors
    ///
    /// Returns an error unless `hash` is exactly [`Self::expected_hash`]
    /// — the store is content-addressed, so any other answer means the
    /// host is not a shortcut service (a storage backend happily
    /// 200-ing a `PUT /@` it never understood, say) and a link built
    /// from its reply would never redirect. Callers treat this like any
    /// other shortening failure and fall back to the full URL.
    pub fn short_url(&self, hash: &str) -> Result<String> {
        let hash = hash.trim();
        anyhow::ensure!(
            hash == self.expected_hash(),
            "shortcut service answered '{hash}' where the content address \
             of the stored target is '{}' — not a conforming shortener",
            self.expected_hash(),
        );
        let mut url = self
            .origin
            .join(&format!("@/{hash}"))
            .context("failed to assemble the short URL")?;
        url.set_fragment(self.fragment.as_deref());
        Ok(url.into())
    }
}

/// Whether a URL names a shortcut (`/@/{hash}`) that must be resolved
/// to the long form before parsing as an invite.
pub fn is_shortcut(url: &str) -> bool {
    Url::parse(url)
        .map(|parsed| parsed.path().starts_with("/@/"))
        .unwrap_or(false)
}

/// Resolve a redirect `Location` against the short link, re-attaching
/// the short link's fragment — RFC 7231 fragment inheritance, done by
/// hand for claimers that aren't browsers.
///
/// # Errors
///
/// Returns an error if the short link or the resolved reference fails
/// to parse.
pub fn resolve_location(short_url: &str, location: &str) -> Result<String> {
    let link = Url::parse(short_url).context("shortcut link is not a valid URL")?;
    let mut resolved = link
        .join(location)
        .context("shortcut redirect Location did not resolve")?;
    if resolved.fragment().is_none() {
        resolved.set_fragment(link.fragment());
    }
    Ok(resolved.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const HASH: &str = "2eyEBFxYVkAy4zRTAtpJEeXAWyzScUYDkxhaizAgZgcF";

    #[dialog_common::test]
    fn it_splits_a_long_url_into_shortcut_parts() {
        let request =
            ShortcutRequest::new("https://tonk.network/join?access=abc&remote=r#seed123").unwrap();
        assert_eq!(request.endpoint.as_str(), "https://tonk.network/@");
        assert_eq!(request.target, "/join?access=abc&remote=r");

        // The store is content-addressed: only the target's own blake3
        // hash assembles a link, exactly as a conforming service answers.
        let hash = request.expected_hash();
        let short = request.short_url(&hash).unwrap();
        assert_eq!(short, format!("https://tonk.network/@/{hash}#seed123"));
    }

    #[dialog_common::test]
    fn it_keeps_fragmentless_urls_fragmentless() {
        let request = ShortcutRequest::new("https://tonk.network/join?access=abc").unwrap();
        let short = request.short_url(&request.expected_hash()).unwrap();
        assert!(!short.contains('#'), "{short}");
    }

    #[dialog_common::test]
    fn it_rejects_any_answer_but_the_targets_content_address() {
        let request = ShortcutRequest::new("https://tonk.network/join?access=abc").unwrap();
        // Not base58, wrong length, and — the case that bites in the
        // field — a well-formed 32-byte hash of something else, which is
        // what a storage backend blindly 200-ing a `PUT /@` produces.
        assert!(request.short_url("!!!").is_err());
        assert!(request.short_url("3vQB7B6MdGQZcSvtzcXAyC").is_err());
        assert!(request.short_url(HASH).is_err());
    }

    #[dialog_common::test]
    fn it_recognizes_shortcut_links() {
        assert!(is_shortcut(&format!("https://tonk.network/@/{HASH}#s")));
        assert!(!is_shortcut("https://tonk.network/join?access=abc#s"));
        assert!(!is_shortcut("not a url"));
    }

    #[dialog_common::test]
    fn it_resolves_locations_with_fragment_inheritance() {
        let short = format!("https://tonk.network/@/{HASH}#seed123");
        let resolved = resolve_location(&short, "/join?access=abc&remote=r").unwrap();
        assert_eq!(
            resolved,
            "https://tonk.network/join?access=abc&remote=r#seed123"
        );

        // An explicit fragment in Location wins, mirroring browsers.
        let resolved = resolve_location(&short, "/join?access=abc#other").unwrap();
        assert_eq!(resolved, "https://tonk.network/join?access=abc#other");
    }
}
