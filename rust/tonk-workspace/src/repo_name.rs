//! `<tonk-repo-name>` — renders the local name of the enclosing space.
//!
//! The topbar lives at the space (directory) level so it shows on every
//! space, including empty ones with no workspace instance. Its title is
//! the repository's local name (the `/space/{name}` segment), which
//! lives on the `<tonk-repository name=…>` route ancestor — not in any
//! view's data model (the workspace concept's own `name` is a display
//! label, and absent at the directory level). Like [`super::share`] this
//! dumb element resolves that ancestor and writes the name as its text
//! content.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use custom_elements::CustomElement;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_sys::{HtmlElement, window};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::ancestors::repo_from_ancestor;

/// Per-element state. The element holds nothing — it renders once on
/// connect and carries no listeners.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Default)]
pub(crate) struct TonkRepoName;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl CustomElement for TonkRepoName {
    fn shadow() -> bool {
        // Light DOM: the consuming view styles the title text and the
        // element must see its `<tonk-repository>` ancestor via `closest`.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let name = repo_from_ancestor(this).unwrap_or_default();
        this.set_text_content(Some(&name));
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}
}

/// Register `<tonk-repo-name>`. Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn register() {
    let Some(elements) = window().map(|w| w.custom_elements()) else {
        return;
    };
    if elements.get("tonk-repo-name").is_undefined() {
        TonkRepoName::define("tonk-repo-name");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// The element writes the nearest `<tonk-repository>` ancestor's
    /// `name` as its text content.
    #[dialog_common::test]
    async fn it_renders_the_ancestor_repo_name() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let repo = document.create_element("tonk-repository").unwrap();
        repo.set_attribute("name", "pictures").unwrap();
        let name = document.create_element("tonk-repo-name").unwrap();
        repo.append_child(&name).unwrap();
        // A defined element runs connectedCallback synchronously on append.
        body.append_child(&repo).unwrap();

        assert_eq!(name.text_content().as_deref(), Some("pictures"));
        let _ = name.dyn_ref::<HtmlElement>();

        repo.remove();
    }

    /// With no `<tonk-repository>` ancestor it renders empty rather than
    /// erroring.
    #[dialog_common::test]
    async fn it_renders_empty_without_a_repository_ancestor() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let name = document.create_element("tonk-repo-name").unwrap();
        body.append_child(&name).unwrap();

        assert_eq!(name.text_content().as_deref(), Some(""));

        name.remove();
    }
}
