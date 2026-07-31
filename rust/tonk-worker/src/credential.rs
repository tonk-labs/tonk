//! Provider-agnostic credential-site absence.

use dialog_effects::credential::CredentialError;

/// Every native `std::io::Error` renders its raw code as `(os error N)`, and
/// `2` is `ENOENT` on unix and `ERROR_FILE_NOT_FOUND` on Windows. Matching the
/// rendered code is the only signal left after the provider stringifies the
/// error.
const NOT_FOUND_OS_ERROR: &str = "(os error 2)";

/// Whether `error` means the credential site was never written.
///
/// Absence is the ordinary state of every optional site the worker reads —
/// no local root, no account provider, no guest record — so it has to read
/// the same on every storage provider. The IndexedDB and in-memory providers
/// report it as [`CredentialError::NotFound`], but the native filesystem
/// provider never constructs that variant: it folds the underlying I/O error
/// into a string-only [`CredentialError::Storage`].
pub(crate) fn is_missing(error: &CredentialError) -> bool {
    match error {
        CredentialError::NotFound(_) => true,
        CredentialError::Storage(message) => message.contains(NOT_FOUND_OS_ERROR),
        CredentialError::Corrupted(_) => false,
    }
}
