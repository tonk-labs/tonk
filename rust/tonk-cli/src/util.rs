use std::path::PathBuf;

/// Get the carry data directory (`~/.carry`).
///
/// If the `CARRY_HOME` environment variable is set, its value is used directly
/// as the carry data directory. When unset, defaults to `~/.carry`.
///
/// This allows integration tests (or custom deployments) to redirect all CLI
/// state to an arbitrary directory.
pub fn tonk_dir() -> Option<PathBuf> {
    if let Ok(carry_home) = std::env::var("CARRY_HOME") {
        return Some(PathBuf::from(carry_home));
    }
    dirs::home_dir().map(|h| h.join(".carry"))
}

/// Get the access directory (`~/.carry/access`).
///
/// Delegations are stored under this directory, keyed by audience and issuer
/// DIDs.
pub fn access_dir() -> Option<PathBuf> {
    tonk_dir().map(|d| d.join("access"))
}
