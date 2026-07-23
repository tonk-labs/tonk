#![warn(missing_docs)]
//! Root identity derived from a passkey.
//!
//! The user's root Ed25519 key is derived on demand from their passkey's
//! PRF output and exists in memory only for the seconds a ceremony needs
//! it. Devices act through a subject-open `root → device` UCAN
//! delegation; day-to-day operation never touches the root key.

pub mod ceremony;
pub mod delegation;
pub mod derive;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod install;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod passkey;
pub mod request;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use install::install;
