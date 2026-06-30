//! Shared helpers for recovering workspace context from light-DOM
//! ancestors. The display route wraps every view in
//! `<tonk-repository name=…>`, so any control rendered inside a view
//! can find the repo it belongs to with `closest`, regardless of which
//! custom element it is.

use web_sys::HtmlElement;

/// Read the `name` of the nearest `<tonk-repository>` ancestor, or `None`
/// when it is absent or empty. Inside the sealed guest the routing ancestors
/// live OUTSIDE the iframe, so fall back to the repo the host supplied in the
/// bridge context (`window.tonk.context.repo`).
pub(crate) fn repo_from_ancestor(this: &HtmlElement) -> Option<String> {
    this.closest("tonk-repository")
        .ok()
        .flatten()
        .and_then(|repo| repo.get_attribute("name"))
        .filter(|name| !name.is_empty())
        // A `{…}` name is an unsubstituted template placeholder (e.g. a repeat
        // prototype `<tonk-repository name={subject}>` upgraded before the row
        // is stamped). It is not a real repo, so ignore it rather than fetch
        // `/api/repository/{subject}/…` (a guaranteed 404).
        .filter(|name| !name.contains('{'))
        .or_else(|| tonk_host::bridge::context_field("repo"))
}
