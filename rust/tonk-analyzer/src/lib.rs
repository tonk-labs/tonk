#![warn(missing_docs)]
// Tests commit with the one-step publish, which fails on a lost race:
// a test's branch has no other writer to lose one to.
#![cfg_attr(test, allow(deprecated))]
//! Analyzer for Tonk notation documents.
//!
//! This crate turns a parsed [`tonk_notation::Syntax`] tree into a typed
//! [`Analysis<Syntax>`][analysis::Analysis] tree, resolving every name
//! against a [`tonk_schema`] schema and minting the synthesized queries
//! and applications that downstream evaluation runs.
//!
//! It sits above [`tonk_schema`] in the dependency graph: the analyzer
//! resolves notation against the schema's concepts and attributes, and
//! `tonk_evaluator` drives the analyzer's output against a repository.

pub mod analysis;

pub mod analyzer;
/// A back end for `dialog-notation`: a notation library lowered to the
/// claims tonk stores for it.
pub mod notation;
