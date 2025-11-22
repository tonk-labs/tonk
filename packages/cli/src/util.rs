use std::path::PathBuf;

/// Get the home directory, respecting TONK_HOME environment variable for testing
pub fn home_dir() -> Option<PathBuf> {
    // Check for TONK_HOME environment variable first (for testing/isolation)
    if let Ok(tonk_home) = std::env::var("TONK_HOME") {
        return Some(PathBuf::from(tonk_home));
    }

    // Fall back to actual home directory
    dirs::home_dir()
}
