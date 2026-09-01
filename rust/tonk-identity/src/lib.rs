#![warn(missing_docs)]
//! Account identity under the custody envelope.
//!
//! The account is a random secret; every custody passkey is an
//! interchangeable wrapping of it, published as a raw cell in the
//! passkey-derived custody space. No key material is ever stored: the
//! secret materializes only inside a ceremony, behind a fresh
//! user-verified assertion, and is zeroized when the ceremony ends.
//! Devices act through a subject-open `root → device` UCAN delegation;
//! day-to-day operation never touches the root key.

/// The error every fallible call here answers with.
///
/// Re-exported so a caller can name it -- and reach the
/// [`custody::CustodyDenial`] inside one -- without taking `anyhow` as a
/// dependency of its own just to spell the type.
pub type Error = anyhow::Error;

pub mod account;
pub mod ceremony;
pub mod clearance;
pub mod custodian;
pub mod custody;
pub mod delegation;
pub mod envelope;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod install;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod passkey;
pub mod request;
pub mod revocation;
pub mod sealed;
pub mod session;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod webcrypto_kek;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use install::install;
