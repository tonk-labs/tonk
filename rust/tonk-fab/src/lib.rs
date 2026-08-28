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
pub mod skin;

mod banner;
mod cluster;
mod field;

#[cfg(target_arch = "wasm32")]
mod activation;

/// `<tonk-button>` — a block button.
#[cfg(target_arch = "wasm32")]
mod button;

/// The bar — responsive cells, overflow stacks and the mode action.
#[cfg(target_arch = "wasm32")]
mod bar;

/// `<tonk-dialog>` — a modal cluster of blocks.
#[cfg(target_arch = "wasm32")]
mod dialog;

#[cfg(target_arch = "wasm32")]
mod element;

/// `<tonk-menu>` — a stack of blocks.
#[cfg(target_arch = "wasm32")]
mod menu;

/// `<tonk-mi>` — one block in a stack.
#[cfg(target_arch = "wasm32")]
mod mi;

/// Shared scaffolding for the FABB component family — shadow attach, mode
/// plumbing, the event emitter, and the block-cursor editable.
#[cfg(target_arch = "wasm32")]
mod shadow;

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

/// Rendering subscription results as rows of a stack.
#[cfg(target_arch = "wasm32")]
mod stack_rows;

#[cfg(target_arch = "wasm32")]
mod subscribing;

/// Register `<tonk-fab>` with the page. Idempotent — safe to call multiple times.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
    menu::register();
    mi::register();
    dialog::register();
    button::register();
    field::register();
    cluster::register();
    banner::register();
    share::register();
    space_name::register();
    profile_name::register();
    member_roster::register();
    space_switcher::register();
}

/// No-op on non-wasm targets (tests / native build checks).
#[cfg(not(target_arch = "wasm32"))]
pub fn register() {}
