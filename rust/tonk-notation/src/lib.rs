//! Asserted-notation parser and validator.
//!
//! This crate is the language model for the carry CLI's three-level YAML
//! notation (entity → context → field). It is intentionally:
//!
//! - **Pure**. No editor, service-worker, or transport dependencies. The
//!   crate produces [`lsp_types::Diagnostic`] values; callers route them
//!   to whichever editor / CLI surface they use.
//! - **Reusable**. The same code that powers the in-browser language
//!   server (under `tonk-worker`) is intended to back a future `carry`
//!   CLI's lint mode without modification.
//!
//! # Phase scope
//!
//! Phase 0 (this revision) implements only the YAML well-formedness
//! check. The full asserted-notation grammar (entity-form rules,
//! reserved domains, concept-schema constraints) lands incrementally in
//! later phases.

pub mod diagnostics;
pub mod parse;

pub use diagnostics::{document_diagnostics, ServerInfo, NOTATION_LANGUAGE_ID, SERVER_INFO};
pub use parse::{parse, Parsed};
