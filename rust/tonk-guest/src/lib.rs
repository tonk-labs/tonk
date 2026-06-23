//! Sealed-iframe guest runtime.
//!
//! The element-registration surface the sealed (opaque-origin) guest loads:
//! a real `<tonk-display>` and friends, plus the [`guest_host`] proxy
//! `<tonk-host>` that relays their consumer events to the portal's
//! `window.tonk` bridge. Deliberately depends only on the custom-element
//! crates — never the service worker or query engine, which live across the
//! iframe boundary and are reached through the bridge.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

pub mod guest_host;
