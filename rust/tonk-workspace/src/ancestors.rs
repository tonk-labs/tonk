//! Shared helper for recovering a workspace control's repository. A
//! control reads its OWN `with` attribute (`branch@repo`), forwarded onto
//! it by the mounting `<tonk-display>` — routing is never inferred from
//! DOM ancestors. Absent that, it falls back to the guest's pinned site
//! context from the bridge.

use tonk_host::location::Location;
use web_sys::HtmlElement;

/// Read the repository from this element's OWN `with` attribute, or `None`
/// when absent, empty, or targeting the profile endpoint. Inside the
/// sealed guest a control with no `with` of its own inherits the site's
/// context, so fall back to the repo the host supplied in the bridge
/// context (`window.tonk.context.repo`).
pub(crate) fn repo_from_context(this: &HtmlElement) -> Option<String> {
    this.get_attribute("with")
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
