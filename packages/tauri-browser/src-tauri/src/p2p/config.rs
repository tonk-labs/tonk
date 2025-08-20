use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPreferences {
    pub mdns: MdnsPreferences,
    pub connection: ConnectionPreferences,
    pub network: NetworkPreferences,
    pub security: SecurityPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsPreferences {
    pub enabled: bool,
    pub announce_interval_secs: u64,
    pub browse_interval_secs: u64,
    pub service_ttl_secs: u64,
    pub max_peers: usize,
    pub preferred_interface: Option<String>,
    pub custom_service_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPreferences {
    pub max_attempts: u32,
    pub initial_backoff_secs: u64,
    pub max_backoff_secs: u64,
    pub connection_timeout_secs: u64,
    pub retry_interval_secs: u64,
    pub auto_connect_discovered: bool,
    pub preferred_connection_types: Vec<ConnectionType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPreferences {
    pub monitor_interfaces: bool,
    pub preferred_address_types: Vec<AddressType>,
    pub bind_to_interface: Option<String>,
    pub prefer_ipv4: bool,
    pub allow_loopback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPreferences {
    pub require_bundle_match: bool,
    pub allowed_bundle_ids: Vec<String>,
    pub blocked_peer_ids: Vec<String>,
    pub max_concurrent_connections: usize,
    pub require_encryption: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionType {
    Direct,
    Relay,
    Dht,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AddressType {
    IPv4,
    IPv6,
    Private,
    Public,
}

impl Default for DiscoveryPreferences {
    fn default() -> Self {
        Self {
            mdns: MdnsPreferences::default(),
            connection: ConnectionPreferences::default(),
            network: NetworkPreferences::default(),
            security: SecurityPreferences::default(),
        }
    }
}

impl Default for MdnsPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            announce_interval_secs: 30,
            browse_interval_secs: 10,
            service_ttl_secs: 120,
            max_peers: 50,
            preferred_interface: None,
            custom_service_name: None,
        }
    }
}

impl Default for ConnectionPreferences {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff_secs: 1,
            max_backoff_secs: 300,
            connection_timeout_secs: 30,
            retry_interval_secs: 60,
            auto_connect_discovered: true,
            preferred_connection_types: vec![ConnectionType::Direct, ConnectionType::Relay],
        }
    }
}

impl Default for NetworkPreferences {
    fn default() -> Self {
        Self {
            monitor_interfaces: true,
            preferred_address_types: vec![AddressType::IPv4, AddressType::Private],
            bind_to_interface: None,
            prefer_ipv4: true,
            allow_loopback: false,
        }
    }
}

impl Default for SecurityPreferences {
    fn default() -> Self {
        Self {
            require_bundle_match: true,
            allowed_bundle_ids: vec![],
            blocked_peer_ids: vec![],
            max_concurrent_connections: 100,
            require_encryption: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    config_path: PathBuf,
    preferences: DiscoveryPreferences,
    custom_settings: HashMap<String, serde_json::Value>,
}

impl ConfigManager {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            preferences: DiscoveryPreferences::default(),
            custom_settings: HashMap::new(),
        }
    }

    pub async fn load(&mut self) -> Result<()> {
        if !self.config_path.exists() {
            // Create default config if it doesn't exist
            self.save().await?;
            return Ok(());
        }

        let config_content = tokio::fs::read_to_string(&self.config_path).await?;

        #[derive(Deserialize)]
        struct ConfigFile {
            preferences: DiscoveryPreferences,
            #[serde(default)]
            custom_settings: HashMap<String, serde_json::Value>,
        }

        let config: ConfigFile = serde_json::from_str(&config_content)?;
        self.preferences = config.preferences;
        self.custom_settings = config.custom_settings;

        Ok(())
    }

    pub async fn save(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        #[derive(Serialize)]
        struct ConfigFile<'a> {
            preferences: &'a DiscoveryPreferences,
            custom_settings: &'a HashMap<String, serde_json::Value>,
        }

        let config = ConfigFile {
            preferences: &self.preferences,
            custom_settings: &self.custom_settings,
        };

        let config_content = serde_json::to_string_pretty(&config)?;
        tokio::fs::write(&self.config_path, config_content).await?;

        Ok(())
    }

    pub fn get_preferences(&self) -> &DiscoveryPreferences {
        &self.preferences
    }

    pub fn get_preferences_mut(&mut self) -> &mut DiscoveryPreferences {
        &mut self.preferences
    }

    pub async fn update_preferences(&mut self, preferences: DiscoveryPreferences) -> Result<()> {
        self.preferences = preferences;
        self.save().await?;
        Ok(())
    }

    pub async fn update_mdns_preferences(&mut self, mdns: MdnsPreferences) -> Result<()> {
        self.preferences.mdns = mdns;
        self.save().await?;
        Ok(())
    }

    pub async fn update_connection_preferences(
        &mut self,
        connection: ConnectionPreferences,
    ) -> Result<()> {
        self.preferences.connection = connection;
        self.save().await?;
        Ok(())
    }

    pub async fn update_network_preferences(&mut self, network: NetworkPreferences) -> Result<()> {
        self.preferences.network = network;
        self.save().await?;
        Ok(())
    }

    pub async fn update_security_preferences(
        &mut self,
        security: SecurityPreferences,
    ) -> Result<()> {
        self.preferences.security = security;
        self.save().await?;
        Ok(())
    }

    pub fn get_custom_setting<T>(&self, key: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.custom_settings
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub async fn set_custom_setting<T>(&mut self, key: String, value: T) -> Result<()>
    where
        T: Serialize,
    {
        let json_value = serde_json::to_value(value)?;
        self.custom_settings.insert(key, json_value);
        self.save().await?;
        Ok(())
    }

    pub async fn remove_custom_setting(&mut self, key: &str) -> Result<()> {
        self.custom_settings.remove(key);
        self.save().await?;
        Ok(())
    }

    pub fn validate_preferences(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate mDNS preferences
        if self.preferences.mdns.announce_interval_secs == 0 {
            errors.push("mDNS announce interval must be greater than 0".to_string());
        }
        if self.preferences.mdns.browse_interval_secs == 0 {
            errors.push("mDNS browse interval must be greater than 0".to_string());
        }
        if self.preferences.mdns.service_ttl_secs < 30 {
            errors.push("mDNS service TTL should be at least 30 seconds".to_string());
        }
        if self.preferences.mdns.max_peers == 0 {
            errors.push("Maximum peers must be greater than 0".to_string());
        }

        // Validate connection preferences
        if self.preferences.connection.max_attempts == 0 {
            errors.push("Maximum connection attempts must be greater than 0".to_string());
        }
        if self.preferences.connection.connection_timeout_secs == 0 {
            errors.push("Connection timeout must be greater than 0".to_string());
        }
        if self.preferences.connection.max_backoff_secs
            < self.preferences.connection.initial_backoff_secs
        {
            errors.push(
                "Maximum backoff must be greater than or equal to initial backoff".to_string(),
            );
        }

        // Validate security preferences
        if self.preferences.security.max_concurrent_connections == 0 {
            errors.push("Maximum concurrent connections must be greater than 0".to_string());
        }

        errors
    }

    // Helper methods to convert preferences to runtime configs
    pub fn to_mdns_config(&self) -> super::mdns::MdnsConfig {
        super::mdns::MdnsConfig {
            enabled: self.preferences.mdns.enabled,
            announce_interval: Duration::from_secs(self.preferences.mdns.announce_interval_secs),
            browse_interval: Duration::from_secs(self.preferences.mdns.browse_interval_secs),
            service_ttl: Duration::from_secs(self.preferences.mdns.service_ttl_secs),
            max_peers: self.preferences.mdns.max_peers,
            interface: self.preferences.mdns.preferred_interface.clone(),
        }
    }

    pub fn to_connection_config(&self) -> super::connection::ConnectionConfig {
        super::connection::ConnectionConfig {
            max_attempts: self.preferences.connection.max_attempts,
            initial_backoff: Duration::from_secs(self.preferences.connection.initial_backoff_secs),
            max_backoff: Duration::from_secs(self.preferences.connection.max_backoff_secs),
            connection_timeout: Duration::from_secs(
                self.preferences.connection.connection_timeout_secs,
            ),
            retry_interval: Duration::from_secs(self.preferences.connection.retry_interval_secs),
        }
    }

    pub fn reset_to_defaults(&mut self) {
        self.preferences = DiscoveryPreferences::default();
        self.custom_settings.clear();
    }

    pub async fn export_config(&self) -> Result<String> {
        #[derive(Serialize)]
        struct ExportConfig<'a> {
            version: &'static str,
            exported_at: String,
            preferences: &'a DiscoveryPreferences,
            custom_settings: &'a HashMap<String, serde_json::Value>,
        }

        let export = ExportConfig {
            version: "1.0",
            exported_at: chrono::Utc::now().to_rfc3339(),
            preferences: &self.preferences,
            custom_settings: &self.custom_settings,
        };

        Ok(serde_json::to_string_pretty(&export)?)
    }

    pub async fn import_config(&mut self, config_json: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct ImportConfig {
            #[serde(default)]
            version: Option<String>,
            preferences: DiscoveryPreferences,
            #[serde(default)]
            custom_settings: HashMap<String, serde_json::Value>,
        }

        let import: ImportConfig = serde_json::from_str(config_json)?;

        // Validate imported preferences
        let temp_config = ConfigManager {
            config_path: self.config_path.clone(),
            preferences: import.preferences.clone(),
            custom_settings: import.custom_settings.clone(),
        };

        let validation_errors = temp_config.validate_preferences();
        if !validation_errors.is_empty() {
            return Ok(validation_errors);
        }

        // Apply imported config
        self.preferences = import.preferences;
        self.custom_settings = import.custom_settings;
        self.save().await?;

        Ok(vec![])
    }
}
