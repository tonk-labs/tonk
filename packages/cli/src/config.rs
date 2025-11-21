use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Global configuration for the CLI
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    /// The currently active space ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_space: Option<String>,
}

impl GlobalConfig {
    /// Load global config from ~/.tonk/config.json
    pub fn load() -> Result<Self> {
        let path: PathBuf = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path).context("Failed to read global config")?;

        let config: GlobalConfig =
            serde_json::from_str(&content).context("Failed to parse global config")?;

        Ok(config)
    }

    /// Save global config to ~/.tonk/config.json
    pub fn save(&self) -> Result<()> {
        let path: PathBuf = Self::config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create .tonk directory")?;
        }

        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize global config")?;

        fs::write(&path, json).context("Failed to write global config")?;

        Ok(())
    }

    /// Get the path to the global config file
    fn config_path() -> Result<PathBuf> {
        let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

        Ok(home.join(".tonk").join("config.json"))
    }
}
