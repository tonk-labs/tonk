//! Routing-context resolution from the `with` attribute.
//!
//! A consumer's context is its OWN optional `with="branch@repo"`
//! attribute. Absent `with` means the enclosing pinned context — in a
//! sealed guest, the site's handshake context; at the top page, none.
//! Routing is never inferred from arbitrary DOM ancestors: a `with` on
//! one element does not silently re-scope its descendants. Context flows
//! only two ways — an element's own `with`, or the site's implicit
//! context (which `<tonk-display>` forwards onto the views it mounts).

use crate::location::{Location, ParseError};
use web_sys::Element;

/// Resolve the routing context for `consumer`: read its OWN non-empty
/// `with` attribute and parse it. No ancestor walk — an element without
/// a `with` of its own inherits the site's pinned context (handled by
/// the caller's fallback), not some enclosing element's.
///
/// A `with` whose value still contains `{…}` is an unsubstituted
/// template placeholder (a repeat prototype upgraded before its row is
/// stamped). That is "no context yet", not a context — return `None`;
/// the re-stamp mutates `with`, which re-triggers resolution.
pub fn resolve_with(consumer: &Element) -> Result<Option<Location>, ParseError> {
    match consumer.get_attribute("with").filter(|v| !v.is_empty()) {
        Some(value) if value.contains('{') => Ok(None),
        Some(value) => value.parse().map(Some),
        None => Ok(None),
    }
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
