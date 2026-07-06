//! Routing-context resolution from the `with` attribute.
//!
//! A consumer's context is the nearest ancestor (including itself)
//! carrying a `with="branch@repo"` attribute, innermost wins. Absent
//! `with` means the enclosing pinned context (in a sealed guest, the
//! site's handshake context; at the top page, none).

use crate::location::{Location, ParseError};
use web_sys::Element;

/// Resolve the routing context for `consumer`: walk up from the
/// element itself looking for the nearest non-empty `with`
/// attribute and parse it.
///
/// A `with` whose value still contains `{…}` is an unsubstituted
/// template placeholder (a repeat prototype upgraded before its row
/// is stamped). That is "no context yet", not a context — return
/// `None` rather than walking further up, so a pending row never
/// resolves against an outer scope it doesn't belong to. The
/// re-stamp mutates `with`, which re-triggers resolution.
pub fn resolve_with(consumer: &Element) -> Result<Option<Location>, ParseError> {
    let mut node = Some(consumer.clone());
    while let Some(el) = node {
        if let Some(value) = el.get_attribute("with").filter(|v| !v.is_empty()) {
            if value.contains('{') {
                return Ok(None);
            }
            return value.parse().map(Some);
        }
        node = el.parent_element();
    }
    Ok(None)
}

/// The `(space, branch, profile)` route triple the URL builders and
/// event details use, from a resolved location.
pub fn route_of(location: &Location) -> (Option<String>, Option<String>, bool) {
    (
        location.space().map(str::to_owned),
        location.branch().map(str::to_owned),
        location.profile(),
    )
}
