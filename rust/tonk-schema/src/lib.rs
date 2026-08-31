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

/// Re-export of the wire-shape primitives from [`tonk_core`].
pub use tonk_core::{claim, conclusion, meta};

/// Analyzer-IR types: `Application`, `Statement`, `Planner`, etc.
/// These live here (not in `tonk-core`) because they reference
/// schema-aware types like [`crate::rule::Rule`].
pub mod transact;

pub mod concept;

pub mod resolution;

pub mod rule_query;

pub mod rule;

pub mod builtin;

pub mod query;

pub mod query_source;

pub mod sync;
pub use sync::*;

pub mod account;
pub use account::{
    AccountActive, AccountDisplayName, AccountRegistered, AccountSealedInbox, AccountSuspended,
    EmailStatus, email_state,
};

pub mod custody;
pub use custody::{Replacement, SecretMessage, SecretPrincipal, SeedKind};

pub mod device_link;
pub mod replica;
pub use device_link::*;
pub use replica::*;

pub mod repository;
pub use repository::*;

pub mod membership;
pub use membership::*;

pub mod invitation;
pub use invitation::*;

pub mod invitation_execution;
pub use invitation_execution::*;

pub mod command;

mod branch;
pub mod directory;
pub use branch::*;

pub mod remote;
pub use remote::*;

pub mod remote_execution;
pub use remote_execution::*;

pub mod tracking_branch;
pub use tracking_branch::*;

pub mod space;
pub use space::{DEFAULT_BRANCH, RouteTarget, SpaceRef, parse_space, resolve_path};

pub mod site;
pub use site::{Route, Site};

mod identity;
pub use identity::{ProfileIdentity, ProfileName};

mod recovery;
mod roster;
pub use recovery::RecoveryPasskey;
pub use roster::DeviceProfile;

mod petname;
pub use petname::petname;
