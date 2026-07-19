//! The install receipt (`install.json`): a snapshot of the release
//! manifest taken at install time, plus where it landed.
//!
//! `install.sh` writes it best-effort — a failed manifest fetch must
//! never fail an install, so a missing receipt is normal and means
//! "assume stable". It is not the basis of detection: it records
//! which channel to check, and lets `tonk update` answer "already
//! current" without downloading an archive to find out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What the last install put on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Channel label (`stable` / `staging`) this copy came from.
    pub channel: String,
    /// Version of the installed build.
    pub version: String,
    /// Git SHA of the installed build.
    pub commit: String,
    /// Directory the binary was installed into.
    pub install_dir: String,
    /// RFC3339 install time, for humans reading the file.
    pub installed_at: String,
}

/// Path to `install.json`. [`crate::update::STATE_ENV`] overrides the
/// directory so tests isolate state.
pub fn path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(crate::update::STATE_ENV) {
        return Some(PathBuf::from(dir).join("install.json"));
    }
    Some(dirs::data_dir()?.join("tonk").join("install.json"))
}

/// Load the receipt; missing or corrupt means `None`.
pub fn load() -> Option<Receipt> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the receipt, creating the parent directory if needed.
pub fn store(receipt: &Receipt) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(receipt).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Receipt {
        Receipt {
            channel: "staging".to_owned(),
            version: "0.4.0".to_owned(),
            commit: "abc1234".to_owned(),
            install_dir: "/usr/local/bin".to_owned(),
            installed_at: "2026-07-16T00:00:00Z".to_owned(),
        }
    }

    #[dialog_common::test]
    fn it_round_trips_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: tests in this mod run on one thread per process
        // invocation; nothing else reads this var concurrently.
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        store(&sample()).expect("store");
        assert_eq!(load(), Some(sample()));
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        assert_eq!(load(), None);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        std::fs::write(dir.path().join("install.json"), "{ not json").expect("write");
        assert_eq!(load(), None);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }
}
