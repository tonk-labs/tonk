//! Re-export of [`tonk_schema::runtime`] under the historical
//! `crate::schema` path.
//!
//! The concepts & claims runtime moved to `tonk-schema` so the
//! tonk-worker (and any future client — language server, browser
//! UI) can build on the same primitives. Carry's call sites
//! continue to reach it as `crate::schema` to keep the lift pure
//! and to avoid touching every command's imports in the same PR.
//!
//! Future work: drop this shim and have call sites import
//! `tonk_schema::runtime` directly.

pub use tonk_schema::runtime::*;
