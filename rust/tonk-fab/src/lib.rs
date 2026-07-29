//! `tonk-fab` — floating action button element.
//!
//! `logic` contains the pure geometry calculations (DOM-free, native-testable).
//! The DOM element and `register()` live here. The element measures its own
//! content box on connect and posts a `{ __tonkFab: { type: "resize", w, h } }`
//! message to its parent window so the `<tonk-fab-portal>` host can resize the
//! iframe to fit.

pub mod logic;
pub mod markup;
pub mod retry;

#[cfg(target_arch = "wasm32")]
mod element;

#[cfg(target_arch = "wasm32")]
mod invitations;
#[cfg(target_arch = "wasm32")]
mod member_roster;

#[cfg(target_arch = "wasm32")]
mod profile_name;

#[cfg(target_arch = "wasm32")]
mod share;

#[cfg(target_arch = "wasm32")]
mod space_name;

#[cfg(target_arch = "wasm32")]
mod space_switcher;

#[cfg(target_arch = "wasm32")]
mod subscribing;

/// Register `<tonk-fab>` with the page. Idempotent — safe to call multiple times.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
    share::register();
    space_name::register();
    profile_name::register();
    member_roster::register();
    invitations::register();
    space_switcher::register();
}

/// No-op on non-wasm targets (tests / native build checks).
#[cfg(not(target_arch = "wasm32"))]
pub fn register() {}
