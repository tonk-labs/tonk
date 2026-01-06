//! Schema definitions for tonk-space attributes.
//!
//! This module contains typed attribute definitions using dialog-query's
//! `#[derive(Attribute)]` macro. Attributes are organized by namespace:
//!
//! - `schema::ucan` - UCAN delegation attributes (issuer, audience, subject, cmd, blob)
//! - `schema::space` - Space management attributes (owner)

pub mod space;
pub mod ucan;
