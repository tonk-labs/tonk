#![warn(missing_docs)]
//! Operation-type primitives shared across the Tonk workspace.
//!
//! This crate is the leaf of the Tonk crate graph: it depends on no other
//! `tonk-*` crate. It defines the operation-type cluster — the values that
//! describe *what to do* to a repository and *what came of it*:
//!
//! - [`claim`] — the typed assert/retract write-unit over a concept.
//! - [`conclusion`] — the result shape produced by evaluating a concept.
//! - [`effect`] — inductive-rule effects and their descriptors.
//! - [`meta`] — names and attributes for facts on a repository's meta branch.

pub mod claim;
pub mod command;
pub mod conclusion;
pub mod effect;
pub mod meta;
