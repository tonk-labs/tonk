//! Attributes shared across concepts in the `xyz.tonk` domain.

// The `#[derive(Attribute)]` macro generates helper types and
// associated functions without doc comments. Suppress the
// crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Attribute;
use dialog_repository::SiteAddress;

/// A human-readable name.
///
/// `xyz.tonk/name` is a single attribute in the schema, reused across
/// every concept that has a display name (replicas, branches, and so
/// on). Keeping it as one attribute means a single query shape —
/// `?entity :name ?n` — can retrieve the name of anything that has
/// one, regardless of which concept it belongs to.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk")]
pub struct Name(pub String);

/// The repository a concept is about, as an entity reference.
///
/// `xyz.tonk/subject` points at a repository's subject DID interpreted
/// as an [`Entity`]. Shared across concepts that belong to a specific
/// repository (replicas, branches, remotes), so "all things about
/// repository X" is a single query.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk")]
pub struct Subject(pub Entity);

/// The profile that owns a replica, as an entity reference.
///
/// The value is the profile's DID interpreted as an [`Entity`]. Like
/// [`Subject`], it duplicates information that went into the `this`
/// hash, but keeping it queryable lets code find "all replicas that
/// belong to this profile" without scanning every replica and
/// recomputing hashes.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk")]
pub struct Profile(pub Entity);

/// The entity a concept lives on, as an entity reference.
///
/// `xyz.tonk/origin` points at the parent entity for concepts that
/// belong to something — a branch belongs to its replica (local
/// branch) or its remote (remote branch), a remote belongs to its
/// replica, and so on. A single attribute handles all cases; the
/// target type is whatever entity the concept is scoped under.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk")]
pub struct Origin(pub Entity);

/// The upstream branch a local branch is tracking, as an entity
/// reference.
///
/// `xyz.tonk.branch/upstream` is the direction-explicit counterpart
/// to [`Origin`]. Asserting `local -upstream-> remote_branch`
/// records that the local branch tracks the remote branch. A local
/// branch either has this attribute (it's tracking something) or
/// doesn't (it isn't) — the presence/absence stands in for what
/// would otherwise be an optional field.
///
/// Scoped to the `xyz.tonk.branch` sub-domain because unlike the
/// other attributes in this module it's meaningful only on
/// [`Branch`] entities, not cross-cutting.
///
/// [`Branch`]: crate::Branch
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.branch")]
pub struct Upstream(pub Entity);

/// A network address, as raw bytes.
///
/// `xyz.tonk.remote/address` stores a serialized [`SiteAddress`] — the
/// opaque bytes a remote uses to locate a peer. Keeping it as bytes
/// (rather than a parsed URL or enum) means dialog's `Attribute`
/// value type (which only supports scalar `Value`s) can carry it
/// directly; consumers deserialize back into `SiteAddress` when they
/// need to connect.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.remote")]
pub struct Address(pub Vec<u8>);

impl Address {
    /// Encode a [`SiteAddress`] as dag-cbor bytes.
    ///
    /// Note: we can't expose this as `From<SiteAddress>` or
    /// `From<&SiteAddress>`. `#[derive(Attribute)]` emits a
    /// blanket `impl<T: Into<Vec<u8>>> From<T> for Address`
    /// and Rust's coherence rules reject any further `From`
    /// impl whose argument type could ever implement
    /// `Into<Vec<u8>>`. `Address::encode` stays the canonical
    /// entry point; convenience methods like
    /// [`Replica::remote`][crate::Replica::remote] take a
    /// `SiteAddress` directly and call `encode` internally.
    pub fn encode(address: &SiteAddress) -> Self {
        let bytes = serde_ipld_dagcbor::to_vec(address)
            .expect("SiteAddress is serde-serializable and dag-cbor-compatible");
        Self(bytes)
    }

    /// Decode the stored dag-cbor bytes back into a
    /// [`SiteAddress`].
    ///
    /// Inverse of [`Address::encode`]. Produces an error when the
    /// stored bytes aren't a valid dag-cbor encoding of a
    /// `SiteAddress` — which today can only happen if the data
    /// was written by a different version of the format.
    pub fn decode(
        &self,
    ) -> Result<SiteAddress, serde_ipld_dagcbor::DecodeError<std::convert::Infallible>> {
        serde_ipld_dagcbor::from_slice(&self.0)
    }
}
