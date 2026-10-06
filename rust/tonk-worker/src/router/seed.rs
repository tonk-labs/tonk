//! Seeding a new space from a URL.
//!
//! `space/create` may carry a `seed`: the URL of a notation document to
//! evaluate into the new space on top of the standard library, the way the
//! standard library itself is seeded. The document is parsed at its URL, so
//! its `!include`s resolve beside it, and it is fetched, parsed, expanded
//! and checked before the space exists: a seed that cannot be used fails the
//! create instead of leaving a half-seeded space behind.
//!
//! A seed is somebody else's content, often code (element methods, rules),
//! so it is fetched conservatively. Only `https` is accepted, or `http` to a
//! loopback host for development. No credentials are sent, redirects are
//! refused rather than followed to wherever they point, the size is capped,
//! and an include may only name a file under the seed's own directory. The
//! consent for all of it is the person's: the page that dispatches a seeded
//! `space/create` shows where the definitions come from and asks first.

use tonk_notation::{Load, Syntax, Url, expand, parse_at};

/// The most a seed document, or one file it includes, may weigh. The
/// standard library's largest document is about a megabyte.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Fetch, parse and expand the seed at `reference`, and check it analyzes
/// on top of `core`, the standard library a new space is seeded with first.
/// Returns the document ready to evaluate, or why it cannot be used, worded
/// for the person who asked for it.
pub(super) async fn prepare(reference: &str, core: &Syntax) -> Result<Syntax, String> {
    let url = Url::parse(reference.trim())
        .map_err(|error| format!("`{reference}` is not a URL: {error}"))?;
    admit(&url)?;
    let bytes = fetch(&url).await?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{url} is not UTF-8 text"))?;

    let syntax = parse_source(url.clone(), &text).await?;
    check(core, &syntax).map_err(|error| format!("{url} does not fit a new space: {error}"))?;
    Ok(syntax)
}

async fn parse_source(url: Url, text: &str) -> Result<Syntax, String> {
    let parsed = parse_at(url.clone(), text);
    if let Some(first) = parsed.diagnostics.first() {
        return Err(format!(
            "{url} does not parse at {}:{}: {}",
            first.range.start.line + 1,
            first.range.start.character + 1,
            first.message
        ));
    }
    let mut syntax = parsed
        .syntax
        .filter(|syntax| !syntax.expressions.is_empty())
        .ok_or_else(|| format!("{url} defines nothing"))?;

    let root = url
        .join("./")
        .map_err(|error| format!("{url} has no directory: {error}"))?;
    let unexpanded = expand(&mut syntax, &Scoped { root }).await;
    if let Some(first) = unexpanded.first() {
        return Err(first.message.clone());
    }

    Ok(syntax)
}

/// Fetch a published catalog and install its selected template. The fragment
/// identifies a slug; the catalog owns file ordering, hashes and entrypoint.
/// Nothing is allocated until every required file has been checked.
pub(super) async fn prepare_template(reference: &str, core: &Syntax) -> Result<Syntax, String> {
    use sha2_0_10::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Catalog {
        schema_version: u32,
        templates: Vec<Template>,
    }
    #[derive(serde::Deserialize)]
    struct Template {
        slug: String,
        entrypoint: String,
        files: Vec<File>,
    }
    #[derive(serde::Deserialize)]
    struct File {
        url: String,
        sha256: String,
        #[serde(default)]
        optional: bool,
    }

    let mut url = Url::parse(reference).map_err(|e| e.to_string())?;
    let slug = url
        .fragment()
        .filter(|s| !s.is_empty())
        .ok_or("Choose a template from the catalog")?
        .to_owned();
    url.set_fragment(None);
    admit(&url)?;
    let catalog: Catalog = serde_json::from_slice(&fetch(&url).await?)
        .map_err(|e| format!("Invalid template catalog: {e}"))?;
    if catalog.schema_version != 1 {
        return Err("Unsupported template catalog version".into());
    }
    let template = catalog
        .templates
        .into_iter()
        .find(|t| t.slug == slug)
        .ok_or("This template is no longer in the catalog")?;
    if template.entrypoint.is_empty()
        || !template
            .entrypoint
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-/_:".contains(c))
    {
        return Err("Invalid template entrypoint".into());
    }
    let root = url.join("./").map_err(|e| e.to_string())?;
    let mut result: Option<Syntax> = None;
    let mut total = 0;
    for file in template.files.into_iter().filter(|f| !f.optional) {
        let source_url = url.join(&file.url).map_err(|e| e.to_string())?;
        if !super::library::within(&root, &source_url) {
            return Err("Template files must be under the catalog directory".into());
        }
        admit(&source_url)?;
        let bytes = fetch(&source_url).await?;
        total += bytes.len();
        if total > MAX_BYTES {
            return Err("Template is too large".into());
        }
        if hex::encode(Sha256::digest(&bytes)) != file.sha256.to_ascii_lowercase() {
            return Err("Template files changed since the catalog was published. Try again after its update completes.".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "Template source is not UTF-8")?;
        let syntax = parse_source(source_url, &text).await?;
        match &mut result {
            Some(combined) => combined.expressions.extend(syntax.expressions),
            None => result = Some(syntax),
        }
    }
    let mut syntax = result.ok_or("Template has no required source files")?;
    // Starter space already defines its home. Other catalog entries rely on
    // the installer's --home option; supply the same home recipe here.
    if slug != "starter-space" {
        let home = format!(
            r#"concept!: &space-home
  this: space:home
  description: The space home page.
  with:
    subject:
      description: The repository's subject DID.
      the: dialog.replica/subject
      as: entity
      cardinality: one

view!:
  this: space:home
  show:
    ui: |
      <tonk-display model={} />

name!:
  this: id:tonk/space
  entity: space:home
"#,
            template.entrypoint
        );
        syntax
            .expressions
            .extend(parse_source(url.clone(), &home).await?.expressions);
    }
    check(core, &syntax).map_err(|e| format!("Template does not fit a new space: {e}"))?;
    Ok(syntax)
}

/// Whether a seed may be fetched from `url` at all.
fn admit(url: &Url) -> Result<(), String> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        scheme => return Err(format!("a seed must be an https URL, not `{scheme}:`")),
    }
    // The worker's own data plane is not a seed: a request to it from here
    // would read the person's data, not somebody's published definitions.
    if let Some(origin) = super::repository::worker_origin()
        && url.origin().ascii_serialization() == origin
        && url.path().starts_with("/api/")
    {
        return Err("a seed cannot be read from this site's API".to_owned());
    }
    Ok(())
}

/// Check that `seed` analyzes on top of `core`, the way it will be applied:
/// after the standard library, whose concepts it may use. The seed's includes
/// are already inlined, so the two documents' locations no longer matter.
fn check(core: &Syntax, seed: &Syntax) -> Result<(), String> {
    let mut combined = core.clone();
    combined
        .expressions
        .extend(seed.expressions.iter().cloned());
    tonk_analyzer::analyzer::analyze_local(&combined)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Loads what a seed includes, from under the seed's own directory.
struct Scoped {
    root: Url,
}

impl Load for Scoped {
    async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
        if !super::library::within(&self.root, uri) {
            return Err(format!(
                "a seed can only include files under `{}`",
                self.root
            ));
        }
        fetch(uri).await
    }
}

/// Fetch `url` the way a seed is fetched: no credentials, no redirects, at
/// most [`MAX_BYTES`].
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch(url: &Url) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        Request, RequestCache, RequestCredentials, RequestInit, RequestMode, RequestRedirect,
        Response, ServiceWorkerGlobalScope,
    };

    let init = RequestInit::new();
    init.set_mode(RequestMode::Cors);
    init.set_credentials(RequestCredentials::Omit);
    init.set_redirect(RequestRedirect::Error);
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(url.as_str(), &init)
        .map_err(|error| format!("cannot request {url}: {error:?}"))?;
    let global: ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| "seeds are fetched by the service worker".to_owned())?;
    // A cross-origin server that does not allow Tonk to read it fails here
    // the same way an unreachable one does: the browser does not say which.
    let response: Response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .and_then(|response| response.dyn_into())
        .map_err(|_| {
            format!(
                "could not read {url}: it is unreachable, redirects, or does not allow \
                 Tonk to read it (CORS)"
            )
        })?;
    if !response.ok() {
        return Err(format!("{url} returned HTTP {}", response.status()));
    }
    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|error| format!("cannot read {url}: {error:?}"))?,
    )
    .await
    .map_err(|error| format!("cannot read {url}: {error:?}"))?;
    let bytes = js_sys::Uint8Array::new(&buffer);
    if bytes.length() as usize > MAX_BYTES {
        return Err(format!("{url} is larger than a seed may be"));
    }
    Ok(bytes.to_vec())
}

/// Fetch `url` the way a seed is fetched: no credentials, no redirects, at
/// most [`MAX_BYTES`].
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn fetch(url: &Url) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("cannot request {url}: {error}"))?;
    let mut response = client
        .get(url.as_str())
        .send()
        .await
        .map_err(|error| format!("could not read {url}: {error}"))?;
    let status = response.status();
    if status.is_redirection() {
        return Err(format!("{url} redirects, and a seed must not"));
    }
    if !status.is_success() {
        return Err(format!("{url} returned HTTP {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("could not read {url}: {error}"))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return Err(format!("{url} is larger than a seed may be"));
        }
    }
    Ok(bytes)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// Opt-in smoke check against today's public catalog. CI uses the tiny
    /// deterministic fixtures in the command and browser regressions instead.
    #[tokio::test]
    #[ignore = "requires the public Honky Tonks service"]
    async fn it_prepares_the_live_discover_catalog() {
        let url =
            super::Url::parse("https://goblinoats.github.io/honky-tonks/catalog.json").unwrap();
        let catalog: serde_json::Value =
            serde_json::from_slice(&super::fetch(&url).await.unwrap()).unwrap();
        let core = crate::router::library::parse(include_str!(
            "../../../tonk-core/assets/library/core.yaml"
        ))
        .await
        .unwrap();
        for template in catalog["templates"].as_array().unwrap() {
            let slug = template["slug"].as_str().unwrap();
            super::prepare_template(&format!("{url}#{slug}"), &core)
                .await
                .unwrap_or_else(|error| panic!("{slug}: {error}"));
        }
    }
}
