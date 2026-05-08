#![warn(missing_docs)]
//! Slide — a local-only CLI for reading and writing tonk facts
//! via the asserted-notation DSL.
//!
//! The crate's public surface is small on purpose: it exists so
//! integration tests (which exercise commands without spawning
//! the binary) and a future SDK consumer can drive the same code
//! paths the CLI does.
//!
//! - [`site`] — `.tonk/` discovery, repo+branch open/init.
//! - [`identity`] — local profile management.
//! - [`eval`] — read source, drive [`tonk_schema::evaluate`],
//!   render output.
//! - [`output`] — render an [`tonk_schema::evaluate::EvaluateResponse`]
//!   as YAML notation or JSON.
//! - [`schema`] — branch introspection: dump every named
//!   attribute and concept as a re-submittable notation document.
//! - [`migrate`] — copy a `.carry/` directory to `.tonk/`.
//! - [`guide`] — the asserted-notation reference, baked in.

pub mod eval;
pub mod guide;
pub mod identity;
pub mod invite;
pub mod migrate;
pub mod output;
pub mod schema;
pub mod site;
pub mod sync;

/// CLI exit codes.
///
/// Each is a small u8 so a [`std::process::exit`] call lands the
/// right value on the shell. Agents can branch on these without
/// parsing stderr.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// All operations succeeded.
    Success = 0,
    /// Source failed to parse — diagnostics on stderr.
    ParseError = 1,
    /// Analyzer rejected the document — diagnostics on stderr.
    AnalyzeError = 2,
    /// Dialog rejected the transaction (planner / commit failure).
    CommitError = 3,
    /// I/O, repo-not-found, or identity error.
    IoError = 4,
}

impl ExitCode {
    /// Numeric value, ready for [`std::process::exit`].
    pub fn into_raw(self) -> i32 {
        self as i32
    }
}
