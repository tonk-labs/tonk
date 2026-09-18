//! `<tonk-introspect>` — pop the hood on a running view.
//!
//! A page is stitched together out of `<tonk-display>` elements: each
//! resolves a concept, picks a view template off the branch, queries
//! the matching entities and interpolates their fields into the
//! template's `{field}` slots. That pipeline is invisible once it has
//! run — the page is just DOM. This module makes it visible again.
//!
//! The interaction, in one paragraph: hold Alt and the display under
//! the pointer outlines. Rest there and observation switches on — the
//! slots the template filled are boxed and labelled with the field
//! that fed them, and a change to any of them flashes where it landed.
//! Alt-click to pin the observation so you can move the pointer away;
//! alt-click again to let it go. [`mode`] holds that state machine,
//! free of the DOM so it can be tested without a browser.
//!
//! What the overlay paints comes from [`slot::Snapshot`], which the
//! renderer builds out of state it already keeps: the binding plan
//! says where every interpolation landed, and the mounted tree caches
//! the last string each one produced. Introspection reads that; it
//! does not re-parse the rendered DOM, which could not tell you
//! whether `with="main@x"` came from a literal or a field anyway.
//!
//! Scope, deliberately: this is the observability half. Editing the
//! matched concept or the view template from a panel is a later step
//! and would go through the ordinary transact path.
//!
//! ## One overlay per frame
//!
//! A `<tonk-display>` renders inside a sealed guest iframe, and a
//! nested `<tonk-site>` opens further iframes below it. Events do not
//! cross those boundaries and neither does hit-testing, so each frame
//! runs its own overlay over its own displays. That works out: the
//! frame under the pointer is the one that receives the pointer.
//! What it costs is that a panel drawn in a small nested frame is
//! clipped by that frame. Hoisting panels to the top document would
//! need the `__tonkRuntime` message relay the theme and press signals
//! already use.

pub mod command;
pub mod inspect;
pub mod mode;
pub mod recorder;
pub mod slot;
pub mod source;

#[cfg(target_arch = "wasm32")]
mod overlay;
#[cfg(target_arch = "wasm32")]
mod panel;
#[cfg(target_arch = "wasm32")]
pub mod registry;

#[cfg(target_arch = "wasm32")]
pub use overlay::register;
#[cfg(target_arch = "wasm32")]
pub use registry::{armed, note_change, note_dispatch};
