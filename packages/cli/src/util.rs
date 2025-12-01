use std::path::PathBuf;

/// Get the home directory
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}
