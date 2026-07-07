//! Sealed-iframe guest runtime.
//!
//! The element-registration surface the sealed (opaque-origin) guest loads:
//! a real `<tonk-display>` and friends, plus [`guest_host`] — the real host
//! IO surface installed on the guest document (fetch-relayed through the
//! portal bootstrap's `window.fetch` override) with the guest-only
//! navigation relay. Deliberately depends only on the custom-element
//! crates — never the service worker or query engine, which live across
//! the iframe boundary and are reached over relayed HTTP.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

pub mod guest_host;
