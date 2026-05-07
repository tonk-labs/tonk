//! `slide guide` — the asserted-notation reference, baked into
//! the binary so an agent in a sandbox without repo access can
//! still discover the syntax by running one command.
//!
//! `include_str!` resolves at compile time relative to the source
//! file, so the path follows the workspace layout
//! (`rust/tonk-notation/guide.md` from `rust/slide/src/guide.rs`).

/// The full notation guide as a static string.
pub const GUIDE: &str = include_str!("../../tonk-notation/guide.md");
