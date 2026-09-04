//! The install receipt (`install.json`): a snapshot of the release
//! manifest taken at install time, plus where it landed.
//!
//! `install.sh` writes it best-effort — a failed manifest fetch must
//! never fail an install, so a missing receipt is normal. Self-update
//! uses a matching receipt to preserve the selected release channel and
//! answer "already current" without downloading an archive to find out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the last install put on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Channel label (`stable` / `staging`) this copy came from.
    /// Self-update trusts it only when `install_dir` matches the running copy.
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
    load_from(&path()?)
}

/// Persist the receipt, creating the parent directory if needed.
pub fn store(receipt: &Receipt) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    store_at(&path, receipt)
}

/// [`load`] against an explicit file, with no environment lookup.
/// Tests drive this directly — a test that pointed the environment at
/// a temp dir would mutate process-global state that every other test
/// in the binary reads concurrently.
fn load_from(path: &Path) -> Option<Receipt> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// [`store`] against an explicit file, with no environment lookup.
fn store_at(path: &Path, receipt: &Receipt) -> std::io::Result<()> {
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
        let path = dir.path().join("install.json");
        store_at(&path, &sample()).expect("store");
        assert_eq!(load_from(&path), Some(sample()));
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(load_from(&dir.path().join("install.json")), None);
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("install.json");
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(load_from(&path), None);
    }

    #[dialog_common::test]
    fn it_creates_the_parent_directory_on_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("install.json");
        store_at(&path, &sample()).expect("store");
        assert_eq!(load_from(&path), Some(sample()));
    }
}
