//! Where the standard library lives, so its documents can `!include`.
//!
//! The service worker already fetches the library from its own origin
//! (`/library/core.yaml`), so a library document has a real location:
//! the served `/library/` directory. Documents are parsed there and an
//! `!include` resolves against it the way a relative URL in a served
//! page would, so `./table/connected.js` beside `table.yaml` is fetched
//! from `/library/table/connected.js`, the same served asset set.
//!
//! The location is the directory rather than one file because seeds are
//! composed: a scaffold, a generated snippet naming the repository, an
//! agent supplement, concatenated into one body. Every library document
//! sits in that one directory, so relative references resolve alike
//! whichever file they were written in.
//!
//! Natively there is no origin, and the library documents are compiled
//! in, so they live at [`tonk_library::ROOT`] and what they include is
//! served from the copies `tonk-library` bundles. Reading the source
//! tree instead would work on a developer's machine and nowhere else:
//! CI runs native suites from a test archive that does not carry it.
//!
//! Only the library is loaded this way. The loader refuses anything
//! outside the library directory, so a library document cannot reach
//! the data plane (`/api/...`) or another origin by including it.

use tonk_notation::{Load, Syntax, Url, expand, parse_at};

use crate::TonkWorkerError;

/// Where library documents live, or `None` where the library has no
/// location: a wasm test harness is not a service worker, so it has no
/// origin to serve from. A document parsed without a location still
/// seeds; it just cannot include.
pub(super) fn location() -> Option<Url> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        // The directory the service worker serves the library from.
        let origin = super::repository::worker_origin()?;
        Url::parse(&format!("{origin}/library/")).ok()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Url::parse(tonk_library::ROOT).ok()
    }
}

/// Loads what a library document includes, from the library directory
/// and nowhere else.
pub(super) struct Library {
    root: Url,
}

impl Library {
    fn new(root: Url) -> Self {
        Self { root }
    }

    fn contains(&self, uri: &Url) -> bool {
        within(&self.root, uri)
    }
}

impl Load for Library {
    async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
        if !self.contains(uri) {
            return Err(format!(
                "a library document can only include files from `{}`",
                self.root
            ));
        }
        read(uri).await
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn read(uri: &Url) -> Result<Vec<u8>, String> {
    super::repository::fetch_library_bytes(uri.path())
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn read(uri: &Url) -> Result<Vec<u8>, String> {
    tonk_library::Bundled.load(uri).await
}

/// Whether `uri` is inside the directory `root`: same origin, a path
/// under the root's, and no query. `Url` has already normalized `..`
/// segments away, so a prefix test cannot be walked out of.
pub(super) fn within(root: &Url, uri: &Url) -> bool {
    uri.scheme() == root.scheme()
        && uri.host_str() == root.host_str()
        && uri.port_or_known_default() == root.port_or_known_default()
        && uri.path().starts_with(root.path())
        && uri.query().is_none()
}

/// Parse a library body at the library's location and inline what it
/// includes. A parse or include failure is a deployment fault, not a
/// client one, and surfaces as an internal error naming the first
/// diagnostic.
pub(super) async fn parse(text: &str) -> Result<Syntax, TonkWorkerError> {
    let Some(root) = location() else {
        return parsed(tonk_notation::parse(text));
    };
    let mut syntax = parsed(parse_at(root.clone(), text))?;
    let unexpanded = expand(&mut syntax, &Library::new(root)).await;
    if let Some(first) = unexpanded.first() {
        return Err(TonkWorkerError::Internal(format!(
            "library include failed at {}:{}: {}",
            first.range.start.line + 1,
            first.range.start.character + 1,
            first.message
        )));
    }
    Ok(syntax)
}

fn parsed(parsed: tonk_notation::Parsed) -> Result<Syntax, TonkWorkerError> {
    if let Some(first) = parsed.diagnostics.first() {
        return Err(TonkWorkerError::Internal(format!(
            "library does not parse at {}:{}: {}",
            first.range.start.line + 1,
            first.range.start.character + 1,
            first.message
        )));
    }
    parsed
        .syntax
        .ok_or_else(|| TonkWorkerError::Internal("library is empty".to_owned()))
}

/// The components this release ships (`library/install`): every library
/// document but the two each branch is seeded with, `core.yaml` (every
/// space) and `profile.yaml` (the profile branch). They are served beside
/// the library and compiled into native builds, so installing one reads
/// no network.
pub(super) const COMPONENTS: [&str; 6] = [
    "issue",
    "meta",
    "notebook",
    "prose",
    "table",
    "version-control",
];

/// Where the shipped components are published. A component is named by
/// its URL, and one under this directory naming a shipped component is
/// read from the copy this release ships rather than fetched: every
/// deployment serves the same files at its own `/library/`.
pub(super) const PUBLISHED: &str = "https://tonk.foundation/library/";

/// Whether `source`, an install's recorded source, is a component's: a
/// shipped one's library file (`/library/<name>.yaml`) or a published
/// document's URL (see [`remote`]).
pub(super) fn is_component_url(source: &str) -> bool {
    shipped(source).is_some() || remote(source).is_some()
}

/// The shipped component `/library/<name>.yaml` names, if it is one.
fn shipped(source: &str) -> Option<&str> {
    source
        .strip_prefix("/library/")
        .and_then(|file| file.strip_suffix(".yaml"))
        .filter(|name| COMPONENTS.contains(name))
}

/// `source` as the URL of a component published somewhere: an absolute
/// `http(s)` URL. Whether it may be fetched is the fetch's to decide.
pub(super) fn remote(source: &str) -> Option<Url> {
    let url = Url::parse(source).ok()?;
    matches!(url.scheme(), "https" | "http").then_some(url)
}

/// Where to read the component a `library/install` names, which is its
/// URL. A URL under [`PUBLISHED`] naming a shipped component, or the
/// `tonk:library/<name>` a release before named one by, reads the shipped
/// copy (`/library/<name>.yaml`); any other `http(s)` URL is read from
/// where it points.
pub(super) fn component_url(component: &str) -> Option<String> {
    let local = |name: &str| {
        COMPONENTS
            .contains(&name)
            .then(|| format!("/library/{name}.yaml"))
    };
    if let Some(name) = component.strip_prefix("tonk:library/") {
        return local(name);
    }
    if let Some(file) = component.strip_prefix(PUBLISHED)
        && let Some(found) = file.strip_suffix(".yaml").and_then(local)
    {
        return Some(found);
    }
    remote(component).map(|url| url.to_string())
}

/// Parse a component's document where it was read from: a shipped one in
/// the library directory, as [`parse`] does, and a published one at its
/// URL, so it includes from beside itself and nowhere else.
pub(super) async fn parse_component(source: &str, text: &str) -> Result<Syntax, TonkWorkerError> {
    match remote(source) {
        Some(url) => super::seed::parse_source(url, text)
            .await
            .map_err(TonkWorkerError::Internal),
        None => parse(text).await,
    }
}

/// Install the component at a URL into the space it was asked in, or bring
/// the installed one up to the version that URL now serves. Like the space's seed, a
/// component is recorded and upgraded on mount; unlike it, a space can have
/// several (see `repository::install_component`).
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::InstallComponent>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::InstallComponent) {
        let component = command.component.0.to_string();
        let Some(source) = component_url(&component) else {
            tonk_common::log!("library/install: '{component}' is not a component's URL");
            return;
        };
        let (repo, branch) = (self.origin().repo.clone(), self.origin().branch.clone());
        let tonk = self.state().read().await;
        match super::repository::install_component(&tonk, &repo, &branch, &source).await {
            Ok(true) => tonk_common::log!("library/install: {source} installed in {repo}"),
            Ok(false) => tonk_common::log!("library/install: {source} is current in {repo}"),
            Err(error) => {
                tonk_common::log!("library/install: {source} into {repo} failed: {error}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    fn library() -> Library {
        Library::new(Url::parse("https://tonk.test/library/").unwrap())
    }

    #[dialog_common::test]
    fn it_admits_files_inside_the_library() {
        let root = library();
        let base = Url::parse("https://tonk.test/library/table.yaml").unwrap();
        assert!(root.contains(&base.join("./table/connected.js").unwrap()));
        assert!(root.contains(&base.join("/library/core.yaml").unwrap()));
    }

    #[dialog_common::test]
    fn it_refuses_anything_outside_the_library() {
        let root = library();
        let base = Url::parse("https://tonk.test/library/table.yaml").unwrap();
        for reference in [
            "../api/repository/x",
            "/api/repository/x",
            "https://elsewhere.test/library/core.yaml",
            "./core.yaml?x=1",
        ] {
            assert!(
                !root.contains(&base.join(reference).unwrap()),
                "{reference} must not load"
            );
        }
    }

    /// A component is named by its URL. One the release ships, under the
    /// published directory or by the name a release before used, reads the
    /// shipped copy; any other `http(s)` URL is read from where it points.
    #[dialog_common::test]
    fn it_reads_a_component_from_its_url() {
        for named in [
            "https://tonk.foundation/library/notebook.yaml",
            "tonk:library/notebook",
        ] {
            assert_eq!(
                component_url(named).as_deref(),
                Some("/library/notebook.yaml"),
                "{named}"
            );
        }
        for published in [
            "https://example.test/library/notebook.yaml",
            "https://tonk.foundation/library/elsewhere.yaml",
            "http://localhost:8080/kanban.yaml",
        ] {
            assert_eq!(
                component_url(published).as_deref(),
                Some(published),
                "{published}"
            );
            assert!(is_component_url(published));
        }
        for refused in [
            "tonk:library/core",
            "tonk:library/elsewhere",
            "file:///etc/passwd",
            "notebook",
        ] {
            assert_eq!(component_url(refused), None, "{refused}");
        }
        assert!(is_component_url("/library/table.yaml"));
        assert!(!is_component_url("/library/core.yaml"));
    }

    /// Natively the library is the bundled copy: a library body parses
    /// at the bundled root, and an include it has no business making is
    /// refused rather than read from wherever the process happens to be.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_parses_a_library_body_at_the_bundled_root() {
        let syntax = parse("xyz.test!:\n  this: id:x\n  body: \"text\"\n")
            .await
            .unwrap();
        assert_eq!(syntax.base.as_str(), tonk_library::ROOT);

        let error = parse("xyz.test!:\n  this: id:x\n  body: !include ../../etc/passwd\n")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("include"), "{error}");
    }
}
