#![warn(missing_docs)]
//! Typed schema for facts stored on a repository's meta branch.
//!
//! Each Tonk repository has a `meta` branch alongside its content branches.
//! The meta branch is a normal dialog-db branch — it participates in sync
//! like any other branch — but its artifacts describe the repository's
//! own configuration: the replicas that hold it, the branches it has,
//! the remotes it tracks, and so on.
//!
//! This crate defines the [`dialog_query::Concept`]s and
//! [`dialog_query::Attribute`]s that make up that schema. It is shared
//! between the service worker (which reads and writes meta facts) and
//! any future client that needs to query the same shape.
//!
//! # Entity identity
//!
//! Entities in this schema are identified as `did:key:z6Mk<base58>` URIs
//! — the same format dialog-db uses elsewhere. The base58-encoded bytes
//! come from one of two sources:
//!
//! - **Intrinsic** — the bytes are real cryptographic key material.
//!   Profile DIDs and repository subject DIDs fall here. The entity is
//!   whoever holds the keypair.
//!
//! - **Content-derived** — the bytes are the blake3 hash of a CBOR
//!   encoding of the entity's defining inputs. Two parties independently
//!   describing "the same thing" converge on the same entity, which
//!   makes the resulting artifacts merge cleanly when the meta branch
//!   syncs across devices.
//!
//! The URI scheme is the same in both cases; the difference lives in how
//! the bytes are produced, not in how they are formatted. For
//! content-derived entities, import [`prelude::EntityExt`] and call
//! [`Entity::of`][dialog_artifacts::Entity] with any serializable
//! value.

pub mod prelude;

pub mod domain;

/// Re-export of the operation-type primitives from [`tonk_core`], so that
/// existing `tonk_schema::{claim, transact, conclusion, effect, meta}`
/// paths continue to resolve after the crate split.
pub use tonk_core::{claim, conclusion, effect, meta, transact};

pub mod concept;

pub mod resolution;

pub mod rule_query;

pub mod rule;

pub mod builtin;

pub mod query;

pub mod query_source;

pub mod replica;
pub use replica::*;

pub mod branch;
pub use branch::*;

pub mod remote;
pub use remote::*;

pub mod tracking_branch;
pub use tracking_branch::*;
