//! Shared helpers for recovering workspace context from light-DOM
//! ancestors. The display route wraps every view in
//! `<tonk-repository name=…>`, so any control rendered inside a view
//! can find the repo it belongs to with `closest`, regardless of which
//! custom element it is.

use web_sys::HtmlElement;

/// Read the `name` of the nearest `<tonk-repository>` ancestor, or
/// `None` when it is absent or empty.
pub(crate) fn repo_from_ancestor(this: &HtmlElement) -> Option<String> {
    this.closest("tonk-repository")
        .ok()
        .flatten()
        .and_then(|repo| repo.get_attribute("name"))
        .filter(|name| !name.is_empty())
}
