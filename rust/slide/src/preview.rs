//! `slide preview` — a capability-routed localhost daemon plus a
//! browser harness bridge that renders candidate `<tonk-view>`
//! templates with the *real* renderer against live branch data.
//!
//! - [`protocol`] — wire types shared by client, daemon, and page.
//! - [`diagnostics`] — native template footgun analysis.
//! - [`project`] — native live-data projection that reproduces the
//!   conclusions `<tonk-display>` subscribes to.
//! - [`daemon`] — capability-routed broker between CLI clients and
//!   the connected browser harness page.
//! - [`client`] — one-shot CLI client that posts a render request
//!   to the daemon and decodes the reply.

pub mod client;
pub mod daemon;
pub mod diagnostics;
pub mod project;
pub mod protocol;
