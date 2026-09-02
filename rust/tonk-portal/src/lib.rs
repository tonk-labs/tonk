//! `<tonk-portal>` — an imperative rendering escape hatch.
//!
//! The declarative `<tonk-view>` / `<tonk-display>` stack paints a
//! template by interpolating an entity's fields into the page DOM.
//! That cannot express arbitrary imperative DOM work — canvas/WebGL
//! drawing, third-party widgets, custom state machines. `<tonk-portal>`
//! fills the gap: it writes an author-supplied HTML document (which
//! may run its own scripts) into a sandboxed `<iframe>`.
//!
//! Like `<tonk-view>`, the portal is a **painter, not a fetcher**. It
//! opens no subscription and resolves no descriptor; it receives an
//! already-fetched HTML string through its `content` attribute and does
//! one imperative thing — assign the iframe's `srcdoc`. The iframe
//! always fills its container. The
//! `content` is itself first-class dialog data: the `portal` concept
//! holds it, and a nested `<tonk-display model=portal>` fetches it,
//! exactly as a board column fetches its tiles.
//!
//! This crate ships the element. The `portal` concept and its canonical
//! view (resolved by model) that bridges it to the element live in the
//! standard library (`tonk-core/assets/library/core.yaml`), seeded by
//! the service worker at repository creation.

#![warn(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod bridge;
/// Install what runs when a guest asks the host to register an account.
/// The dialog lives in the shell, which depends on this crate, so the
/// behaviour is supplied rather than named here.
#[cfg(target_arch = "wasm32")]
pub use bridge::{RegisterFocusReturn, on_register, set_trusted_relay_fetch};
#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod query;
#[cfg(target_arch = "wasm32")]
mod shared;
#[cfg(target_arch = "wasm32")]
mod site;
mod site_content;
#[cfg(target_arch = "wasm32")]
mod title;

#[cfg(target_arch = "wasm32")]
pub use element::register;
#[cfg(target_arch = "wasm32")]
pub use site::register as register_site;
#[cfg(target_arch = "wasm32")]
pub use title::register as register_title;
