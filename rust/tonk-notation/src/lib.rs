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

pub mod diagnostics;
pub mod parse;
pub mod shape;

pub use diagnostics::{NOTATION_LANGUAGE_ID, SERVER_INFO, ServerInfo, document_diagnostics};
pub use parse::{Parsed, parse};
