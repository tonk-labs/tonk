//! Shared notation-template machinery: parse a `source` attribute
//! into a [`tonk_schema::query::Query`], split a `{field}`-bearing
//! template into a chrome/repeat plan, and substitute conclusions
//! into it.
//!
//! These modules back `<tonk-display>`'s rendering; the crate no
//! longer ships a custom element of its own.

#![warn(missing_docs)]

pub mod error;
pub mod resolve;
pub mod template;
