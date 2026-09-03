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

/// Public attribution parameters that a caller may add to a short URL.
///
/// The stored invite target owns all capability and space-attribution fields.
/// Only these campaign/source fields may override their stored counterparts
/// when `GET /@/{hash}` builds its redirect.
const REFERRAL_PARAMETERS: &[&str] = &[
    "tonk_source",
    "tonk_channel",
    "utm_source",
    "utm_medium",
    "utm_campaign",
];

/// Maximum decoded length of one forwarded attribution value.
const MAX_REFERRAL_VALUE_SIZE: usize = 128;

/// Standalone recovery page for a missing or expired invite shortcut.
///
/// The shortcut is the only part available at this boundary: an invite's
/// private seed stays in the URL fragment and never reaches the service. That
/// means a missing object and one removed after expiry are deliberately
/// presented with the same non-sensitive explanation.
pub fn unavailable_invite_html() -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <link rel="icon" href="data:,">
  <title>Share link unavailable · Tonk</title>
  <style>
    @font-face {{
      font-family: "GestalteName";
      font-style: normal;
      font-weight: 500;
      font-display: swap;
      src: url("/fonts/gestaltename-500.ttf") format("truetype");
    }}
    @font-face {{
      font-family: "Gestalte";
      font-style: normal;
      font-weight: 400;
      font-display: swap;
      src: url("/fonts/gestalte-400.otf") format("opentype");
    }}
    @font-face {{
      font-family: "IBM Plex Sans";
      font-style: normal;
      font-weight: 400;
      font-display: swap;
      src: url("/fonts/ibm-plex-sans-400-normal.woff2") format("woff2");
    }}
    :root {{
      /* DESIGN.md core palette: warm stone page, aubergine ink, card
         surface, and a one-pixel ink ring instead of elevation shadows. */
      color-scheme: light dark;
      --invite-ink: light-dark(#38182a, #e2dfdd);
      --invite-muted: light-dark(#5b4953, #c8c3bf);
      --invite-surface: light-dark(#fcfbfb, #261f20);
      --invite-page: light-dark(#e8e6e4, #161313);
      --invite-ring: light-dark(rgb(56 24 42 / 85%), rgb(226 223 221 / 55%));
      --invite-on-ink: light-dark(#f7f6f5, #221c1d);
      --invite-wash-on: light-dark(rgb(247 246 245 / 16%), rgb(34 28 29 / 14%));
      -webkit-font-smoothing: antialiased;
      -moz-osx-font-smoothing: grayscale;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      min-height: 100dvh;
      margin: 0;
      display: grid;
      place-items: center;
      padding: 40px 24px;
      background: var(--invite-page);
      color: var(--invite-ink);
      font-family: "IBM Plex Sans", Helvetica, Arial, sans-serif;
    }}
    main {{
      width: min(100%, 760px);
      padding: 40px;
      background: var(--invite-surface);
      box-shadow: 0 0 0 1px var(--invite-ring);
    }}
    .masthead {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 24px;
    }}
    .brand {{
      min-height: 44px;
      display: inline-flex;
      align-items: center;
      gap: 9px;
      color: inherit;
      text-decoration: none;
    }}
    .wordmark {{
      font-family: "GestalteName", "Gestalte", sans-serif;
      font-size: 34px;
      font-weight: 500;
      line-height: 1;
      text-transform: lowercase;
    }}
    .badge {{
      padding: 2px 8px;
      background: var(--invite-ink);
      color: var(--invite-on-ink);
      font-size: 14px;
      font-weight: 600;
    }}
    h1 {{
      max-width: 16ch;
      margin: 44px 0 28px;
      font-family: "Gestalte", Georgia, serif;
      font-size: clamp(36px, 7vw, 52px);
      font-weight: 400;
      line-height: 1;
      letter-spacing: -0.035em;
      text-wrap: balance;
    }}
    .lede {{
      max-width: 48ch;
      margin: 0 0 30px;
      color: var(--invite-muted);
      font-size: 16px;
      line-height: 1.55;
      text-wrap: pretty;
    }}
    .actions {{
      width: min(100%, 208px);
    }}
    .button {{
      min-height: 46px;
      padding: 11px 16px;
      display: grid;
      place-items: center;
      background: var(--invite-ink);
      box-shadow: 0 0 0 1px var(--invite-ink);
      color: var(--invite-on-ink);
      font-weight: 600;
      text-decoration: none;
      transition-property: scale, box-shadow;
      transition-duration: 150ms;
      transition-timing-function: ease-out;
    }}
    .button:hover {{
      background: linear-gradient(var(--invite-wash-on), var(--invite-wash-on)),
        var(--invite-ink);
    }}
    .button:active {{ scale: 0.96; }}
    .button:focus-visible,
    .brand:focus-visible {{
      outline: 2px solid var(--invite-ink);
      outline-offset: 3px;
    }}
    @media (max-width: 620px) {{
      body {{
        place-items: stretch;
        padding: 0;
      }}
      main {{
        min-height: 100dvh;
        padding: 24px 20px 32px;
        box-shadow: none;
      }}
      h1 {{ margin: 36px 0 24px; }}
      .actions {{ width: 100%; }}
      .button {{ width: 100%; }}
    }}
    @media (prefers-reduced-motion: reduce) {{
      .button {{ transition-duration: 0s; }}
    }}
  </style>
</head>
<body>
  <main>
    <header class="masthead">
      <a class="brand" href="/" aria-label="Tonk home">
        <span class="wordmark">tonk</span>
        <span class="badge">invite</span>
      </a>
    </header>
    <h1>This share link is no longer available</h1>
    <p class="lede">Short share links normally expire after {DEFAULT_TTL_DAYS} days. Ask the person who shared it with you to create a new one.</p>
    <div class="actions">
      <a class="button" href="/">Back to Tonk</a>
    </div>
  </main>
</body>
</html>"#
    )
}

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

/// Merge public campaign/source tags from a short-link request into its stored
/// redirect target.
///
/// This lets `https://.../@/{hash}?tonk_source=reddit#seed` retain the source
/// tag when it expands to `/join?...#seed`. Capability parameters and the
/// stored `tonk_space` token can never be supplied or replaced here. If the
/// request carries no accepted values, the stored target is returned byte for
/// byte so existing shortcut behaviour stays unchanged.
pub fn referral_redirect_target(target: &str, request_query: Option<&str>) -> String {
    let overrides: Vec<(String, String)> = request_query
        .into_iter()
        .flat_map(|query| url::form_urlencoded::parse(query.as_bytes()))
        .filter(|(key, value)| {
            REFERRAL_PARAMETERS.contains(&key.as_ref()) && value.len() <= MAX_REFERRAL_VALUE_SIZE
        })
        .fold(Vec::new(), |mut values, (key, value)| {
            if !values
                .iter()
                .any(|(existing, _): &(String, String)| existing == key.as_ref())
            {
                values.push((key.into_owned(), value.into_owned()));
            }
            values
        });
    if overrides.is_empty() {
        return target.to_owned();
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let overridden: Vec<&str> = overrides.iter().map(|(key, _)| key.as_str()).collect();
    let existing = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| !overridden.contains(&key.as_ref()));
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(existing);
    serializer.extend_pairs(&overrides);
    let query = serializer.finish();
    format!("{path}?{query}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referral_redirects_override_only_public_attribution_fields() {
        let target = "/join?access=SECRET&tonk_channel=reshare&tonk_space=0123456789abcdef";
        assert_eq!(referral_redirect_target(target, None), target);

        let redirected = referral_redirect_target(
            target,
            Some(
                "tonk_source=reddit&tonk_channel=outreach&tonk_space=ffffffffffffffff&access=EVIL",
            ),
        );
        let url = Url::parse(&format!("https://tonk.invalid{redirected}")).unwrap();
        let value = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        assert_eq!(value("access").as_deref(), Some("SECRET"));
        assert_eq!(value("tonk_space").as_deref(), Some("0123456789abcdef"));
        assert_eq!(value("tonk_source").as_deref(), Some("reddit"));
        assert_eq!(value("tonk_channel").as_deref(), Some("outreach"));
    }

    #[test]
    fn referral_redirects_drop_duplicate_and_oversized_values() {
        let oversized = "x".repeat(MAX_REFERRAL_VALUE_SIZE + 1);
        let redirected = referral_redirect_target(
            "/join?access=abc",
            Some(&format!(
                "tonk_source=slack&tonk_source=discord&utm_campaign={oversized}"
            )),
        );
        assert_eq!(redirected, "/join?access=abc&tonk_source=slack");
    }
}
