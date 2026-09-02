//! Asserted-notation parser.
//!
//! Pure parser for tonk's three-level notation. The crate
//! produces a typed [`Syntax`] tree from YAML input plus
//! [`lsp_types::Diagnostic`] values for any structural problems;
//! callers route the diagnostics to whichever editor / CLI
//! surface they use.
//!
//! Resolution against a branch (looking up concepts and
//! attributes by name, deriving entity URIs, building queries
//! and transactions) lives in
//! [`tonk-schema`](https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-schema).
//! See [`guide.md`](https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-notation/guide.md)
//! for the user-facing notation reference.

pub mod diagnostics;
pub mod format;
pub mod highlight;
pub mod parse;
pub mod syntax;

pub use diagnostics::{NOTATION_LANGUAGE_ID, SERVER_INFO, ServerInfo, document_diagnostics};
pub use parse::{Parsed, parse};
pub use syntax::{
    Anchor, Application, Effectful, Expression, Field, FieldValue, HeadName, Predicate, Premise,
    Scalar, Spanned, Syntax,
};
