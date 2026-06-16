//! `<tonk-tree>` — a custom element that inspects the
//! **dialog-search-tree** index behind a branch, in the style of
//! `dialog-diagnose`. A pure-Rust web component like the other tonk
//! elements (`tonk-display`, `tonk-board`, `tonk-sigil`): it resolves
//! its repository from the `<tonk-repository>` routing ancestor, queries
//! the worker's `tree/*` formulas, and renders a `<wa-tree>` outline of
//! index/segment nodes plus a node inspector.
//!
//! Visit `/space/{repo}/dialog:diagnose` in the app to use it.

mod dom;
mod inspector;
mod key;
mod model;
mod web;

pub use web::register;
