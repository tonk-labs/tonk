//! Shortcut service core: validation and addressing.
//!
//! The shortcut service is a same-origin URL shortener. `PUT /@` stores
//! a **path + query** string (never an absolute URL) under its blake3
//! hash and returns the hash; `GET /@/{hash}` answers with a permanent
//! redirect whose `Location` is the stored string, verbatim and
//! relative. Because a relative `Location` can only resolve within the
//! requesting origin, the service is useless as an open redirector —
//! which is what makes it safe to leave **permissionless**. The first
//! consumer is invite links: the long `?access=` URL's path + query is
//! stored, and the secret seed rides the short link's `#fragment`,
//! which redirects inherit per RFC 7231 §7.1.2 and which never reaches
//! the server at all.
//!
//! Keys are content-addressed (`tonk/link/{base58(blake3(target))}`),
//! so re-shortening is idempotent and an existing shortcut can never be
//! repointed — a colliding PUT can only carry the identical content.
//!
//! This module is the platform-independent core shared by the
//! Cloudflare Worker handler and the native test server in `helpers`;
//! storage IO stays with the callers.

use url::Url;

/// Maximum stored target size in bytes. Generous headroom for a
/// multi-hop `?access=` chain plus launcher parameters, useless as a
/// general paste bin.
pub const MAX_TARGET_SIZE: usize = 8 * 1024;

/// Storage key prefix for stored shortcuts.
pub const KEY_PREFIX: &str = "tonk/link/";

/// Default shortcut lifetime in days, when `PUT /@` carries no `ttl`
/// parameter.
pub const DEFAULT_TTL_DAYS: u64 = 7;

/// Maximum shortcut lifetime in days; requested TTLs are capped here.
/// The physical-cleanup lifecycle rule on the storage prefix assumes
/// this bound.
pub const MAX_TTL_DAYS: u64 = 20;

/// Seconds per day, for converting the day-granular `ttl` parameter
/// into the stored unix-seconds expiry.
pub const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// Custom-metadata key on stored shortcut objects carrying the
/// unix-seconds expiry. Reads treat objects past it as gone; the
/// bucket lifecycle rule on [`KEY_PREFIX`] does the physical cleanup.
pub const EXPIRES_METADATA_KEY: &str = "expires";

/// Effective lifetime in seconds for a `PUT /@` request: the `ttl`
/// query parameter, expressed in **days** — mirroring the day
/// granularity of the R2 lifecycle rules that do the physical
/// cleanup — defaulted to [`DEFAULT_TTL_DAYS`] when absent and capped
/// at [`MAX_TTL_DAYS`].
pub fn requested_ttl(query: Option<&str>) -> Result<u64, String> {
    let Some(query) = query else {
        return Ok(DEFAULT_TTL_DAYS * SECONDS_PER_DAY);
    };
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "ttl" {
            let days: u64 = value
                .parse()
                .map_err(|_| format!("ttl {value:?} is not a number of days"))?;
            return Ok(days.min(MAX_TTL_DAYS) * SECONDS_PER_DAY);
        }
    }
    Ok(DEFAULT_TTL_DAYS * SECONDS_PER_DAY)
}

/// A validated shortcut: the target string and its content hash.
#[derive(Debug, Clone, PartialEq)]
pub struct Shortcut {
    /// The stored path + query string, emitted verbatim as `Location`.
    pub target: String,
    /// blake3 hash of the target bytes.
    pub hash: blake3::Hash,
}

impl Shortcut {
    /// Validate a `PUT /@` body into a shortcut.
    ///
    /// The target must be a same-origin path + query reference:
    /// UTF-8, `/`-rooted, no scheme or authority (protocol-relative
    /// `//host` references resolve to another origin and are
    /// rejected), no fragment (an explicit fragment in `Location`
    /// would override the inheritance the seed relies on), no control
    /// characters (the string is emitted as a header), and within
    /// [`MAX_TARGET_SIZE`].
    pub fn new(body: &[u8]) -> Result<Self, String> {
        if body.len() > MAX_TARGET_SIZE {
            return Err(format!(
                "target is {} bytes, exceeding the {MAX_TARGET_SIZE}-byte limit",
                body.len()
            ));
        }
        let target = str::from_utf8(body).map_err(|_| "target is not valid UTF-8".to_string())?;
        if !target.starts_with('/') {
            return Err("target must be a /-rooted path".to_string());
        }
        if target.contains('#') {
            return Err("target must not carry a fragment".to_string());
        }
        if target.chars().any(char::is_control) {
            return Err("target must not contain control characters".to_string());
        }

        // Resolve against a throwaway base and require the origin to
        // survive: catches protocol-relative references (`//evil.tld`)
        // and any backslash trickery the WHATWG parser folds into
        // authority changes.
        let base = Url::parse("https://shortcut.invalid/").expect("static base URL parses");
        let joined = base
            .join(target)
            .map_err(|e| format!("target does not resolve as a URL reference: {e}"))?;
        if joined.origin() != base.origin() {
            return Err("target must not name a host; only path + query is stored".to_string());
        }

        Ok(Self {
            target: target.to_string(),
            hash: blake3::hash(body),
        })
    }

    /// base58 string form of the hash — the `{hash}` segment of
    /// `/@/{hash}` and the tail of the storage key.
    pub fn hash_str(&self) -> String {
        hash_str(&self.hash)
    }

    /// Storage object key for this shortcut.
    pub fn object_key(&self) -> String {
        format!("{KEY_PREFIX}{}", self.hash_str())
    }
}

/// base58 string form of a shortcut hash.
pub fn hash_str(hash: &blake3::Hash) -> String {
    bs58::encode(hash.as_bytes()).into_string()
}

/// Parse the `{hash}` path segment of `GET /@/{hash}`, returning the
/// storage key. Rejects anything that is not base58 of 32 bytes, so
/// lookups can never escape the shortcut prefix.
pub fn object_key_for(hash: &str) -> Result<String, String> {
    let bytes = bs58::decode(hash)
        .into_vec()
        .map_err(|e| format!("hash is not valid base58: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("hash must be 32 bytes, got {}", bytes.len()));
    }
    Ok(format!("{KEY_PREFIX}{hash}"))
}
