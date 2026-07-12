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

    /// Assemble the short link from the hash the service returned,
    /// re-attaching the source URL's fragment.
    ///
    /// # Errors
    ///
    /// Returns an error if `hash` is not base58 of 32 bytes — the
    /// response is validated rather than trusted blindly.
    pub fn short_url(&self, hash: &str) -> Result<String> {
        let bytes = bs58::decode(hash.trim())
            .into_vec()
            .context("shortcut service returned a non-base58 hash")?;
        anyhow::ensure!(
            bytes.len() == 32,
            "shortcut service returned a {}-byte hash, expected 32",
            bytes.len()
        );
        let mut url = self
            .origin
            .join(&format!("@/{}", hash.trim()))
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
            ShortcutRequest::new("https://hub.tonk.xyz/join?access=abc&remote=r#seed123").unwrap();
        assert_eq!(request.endpoint.as_str(), "https://hub.tonk.xyz/@");
        assert_eq!(request.target, "/join?access=abc&remote=r");

        let short = request.short_url(HASH).unwrap();
        assert_eq!(short, format!("https://hub.tonk.xyz/@/{HASH}#seed123"));
    }

    #[dialog_common::test]
    fn it_keeps_fragmentless_urls_fragmentless() {
        let request = ShortcutRequest::new("https://hub.tonk.xyz/join?access=abc").unwrap();
        let short = request.short_url(HASH).unwrap();
        assert!(!short.contains('#'), "{short}");
    }

    #[dialog_common::test]
    fn it_rejects_hashes_that_are_not_base58_of_32_bytes() {
        let request = ShortcutRequest::new("https://hub.tonk.xyz/join?access=abc").unwrap();
        assert!(request.short_url("!!!").is_err());
        assert!(request.short_url("3vQB7B6MdGQZcSvtzcXAyC").is_err());
    }

    #[dialog_common::test]
    fn it_recognizes_shortcut_links() {
        assert!(is_shortcut(&format!("https://hub.tonk.xyz/@/{HASH}#s")));
        assert!(!is_shortcut("https://hub.tonk.xyz/join?access=abc#s"));
        assert!(!is_shortcut("not a url"));
    }

    #[dialog_common::test]
    fn it_resolves_locations_with_fragment_inheritance() {
        let short = format!("https://hub.tonk.xyz/@/{HASH}#seed123");
        let resolved = resolve_location(&short, "/join?access=abc&remote=r").unwrap();
        assert_eq!(
            resolved,
            "https://hub.tonk.xyz/join?access=abc&remote=r#seed123"
        );

        // An explicit fragment in Location wins, mirroring browsers.
        let resolved = resolve_location(&short, "/join?access=abc#other").unwrap();
        assert_eq!(resolved, "https://hub.tonk.xyz/join?access=abc#other");
    }
}
