//! `slide preview` — a capability-routed localhost daemon plus a
//! browser harness bridge that renders candidate `<tonk-view>`
//! templates with the *real* renderer against live branch data.
//!
//! - [`protocol`] — wire types shared by client, daemon, and page.
//! - [`diagnostics`] — native template footgun analysis.

pub mod diagnostics;
pub mod protocol;
