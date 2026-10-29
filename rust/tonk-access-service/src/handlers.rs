//! HTTP request handlers.

pub mod config;
pub mod deletion;
pub mod health;
pub mod info;
pub mod lookup;
#[cfg(target_arch = "wasm32")]
pub mod oauth;
pub mod object;
pub mod registration;
pub mod revoke;
pub mod shortcut;
pub mod ucan;

pub mod invitation;
