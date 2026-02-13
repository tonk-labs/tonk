use std::path::PathBuf;

/// Get the tonk data directory (`~/.tonk`).
///
/// If the `TONK_HOME` environment variable is set, its value is used directly
/// as the tonk data directory. When unset, defaults to `~/.tonk`.
///
/// This allows integration tests (or custom deployments) to redirect all CLI
/// state to an arbitrary directory.
pub fn tonk_dir() -> Option<PathBuf> {
    if let Ok(tonk_home) = std::env::var("TONK_HOME") {
        return Some(PathBuf::from(tonk_home));
    }
    dirs::home_dir().map(|h| h.join(".tonk"))
}

/// Get the access directory (`~/.tonk/access`).
///
/// Delegations are stored under this directory, keyed by audience and issuer
/// DIDs.
pub fn access_dir() -> Option<PathBuf> {
    tonk_dir().map(|d| d.join("access"))
}
