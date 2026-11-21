//! The error the palette's intent readings report.

use dialog_corpus::DialogCorpusError;
use thiserror::Error;

/// Why a reading of the palette's intent failed.
#[derive(Debug, Error)]
pub enum FormulaError {
    /// What a reading was given could not be read.
    #[error("bad input for {formula}: {reason}")]
    BadInput {
        /// What was being read.
        formula: String,
        /// Why the input was refused.
        reason: String,
    },

    /// A read failed.
    #[error("read failed: {0}")]
    Read(String),

    /// What was read did not decode.
    #[error("decode failed: {0}")]
    Decode(String),
}

impl From<DialogCorpusError> for FormulaError {
    fn from(error: DialogCorpusError) -> Self {
        Self::Decode(error.to_string())
    }
}
