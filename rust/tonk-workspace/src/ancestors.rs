//! Shared helpers for recovering workspace context from light-DOM
//! ancestors. Views carry their routing context on a `with`
//! attribute (`branch@repo`), so any control rendered inside a view
//! can find the repo it belongs to with `closest`, regardless of
//! which custom element it is.

use tonk_host::location::Location;
use web_sys::HtmlElement;

/// Read the repository of the nearest `with` ancestor (including
/// `this` itself), or `None` when absent, empty, or targeting the
/// profile endpoint. Inside the sealed guest the routing context may
/// live OUTSIDE the iframe, so fall back to the repo the host
/// supplied in the bridge context (`window.tonk.context.repo`).
pub(crate) fn repo_from_ancestor(this: &HtmlElement) -> Option<String> {
    this.closest("[with]")
        .ok()
        .flatten()
        .and_then(|el| el.get_attribute("with"))
        .filter(|value| !value.is_empty())
        // A `{…}` value is an unsubstituted template placeholder (e.g. a
        // repeat prototype `with="main@{subject}"` upgraded before the row
        // is stamped). It is not a real context, so ignore it rather than
        // fetch `/api/repository/{subject}/…` (a guaranteed 404).
        .filter(|value| !value.contains('{'))
        .and_then(|value| value.parse::<Location>().ok())
        .and_then(|location| location.space().map(str::to_owned))
        .or_else(|| tonk_host::bridge::context_field("repo"))
}
