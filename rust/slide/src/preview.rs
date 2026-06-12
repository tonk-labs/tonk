//! `slide preview` — a capability-routed localhost daemon plus a
//! browser harness bridge that renders candidate `<tonk-view>`
//! templates with the *real* renderer against live branch data.
//!
//! - [`protocol`] — wire types shared by client, daemon, and page.
//! - [`diagnostics`] — native template footgun analysis.
//! - [`project`] — native live-data projection that reproduces the
//!   conclusions `<tonk-display>` subscribes to.

pub mod diagnostics;
pub mod project;
pub mod protocol;
