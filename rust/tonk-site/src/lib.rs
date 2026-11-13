//! `<tonk-site>` and the bridge to the page around it.
//!
//! `<tonk-site>` frames a site on an origin of its own and tells it which
//! path to show (see `site.rs`). The page that mounts it talks to the
//! framed guest only over the bridge (see `bridge.rs`): a `MessagePort`
//! the guest opens with a `hello`, through which it asks the trusted page
//! for what a sealed frame cannot do itself, such as navigation, titles,
//! tasks and fetches on the page's real origin.

#![warn(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod bridge;
/// Install what runs when a guest asks the host to register an account.
/// The dialog lives in the shell, which depends on this crate, so the
/// behaviour is supplied rather than named here.
#[cfg(target_arch = "wasm32")]
pub use bridge::{
    ContainedTaskReturn, DelegationRequest, DelegationReturn, RegisterFocusReturn, STAGE_EVENT,
    on_delegate, on_register, on_task,
};
#[cfg(target_arch = "wasm32")]
mod site;
#[cfg(target_arch = "wasm32")]
mod space_origin;
pub mod task;
#[cfg(target_arch = "wasm32")]
pub use site::register as register_site;
#[cfg(target_arch = "wasm32")]
pub use space_origin::{account, signed_in};
