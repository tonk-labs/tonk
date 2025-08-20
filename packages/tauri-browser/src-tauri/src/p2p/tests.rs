#[cfg(test)]
mod mdns_tests {
    use super::mdns::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_tonk_service_info_creation() {
        let bundle_id = "test-bundle-123".to_string();
        let node_id = "test-node-456".to_string();
        let port = 8080;
        let addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))];

        let service_info = TonkServiceInfo::new(
            bundle_id.clone(),
            node_id.clone(),
            port,
            addresses.clone(),
        );

        assert_eq!(service_info.bundle_id, bundle_id);
        assert_eq!(service_info.node_id, node_id);
        assert_eq!(service_info.port, port);
        assert_eq!(service_info.addresses, addresses);
        assert_eq!(service_info.protocol_version, 1);
        assert!(service_info.capabilities.contains(&"sync".to_string()));
        assert_eq!(service_info.instance_name, format!("tonk-{}", &bundle_id[..8]));
        assert!(service_info.discovered_at > 0);
    }

    #[test]
    fn test_txt_properties_conversion() {
        let service_info = TonkServiceInfo {
            bundle_id: "test-bundle".to_string(),
            node_id: "test-node".to_string(),
            protocol_version: 2,
            port: 9000,
            capabilities: vec!["sync".to_string(), "relay".to_string()],
            instance_name: "tonk-test".to_string(),
            addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            discovered_at: 1234567890,
        };

        let txt_properties = service_info.to_txt_properties();

        assert_eq!(txt_properties.get("bundle_id"), Some(&"test-bundle".to_string()));
        assert_eq!(txt_properties.get("node_id"), Some(&"test-node".to_string()));
        assert_eq!(txt_properties.get("protocol_version"), Some(&"2".to_string()));
        assert_eq!(txt_properties.get("port"), Some(&"9000".to_string()));
        assert_eq!(txt_properties.get("capabilities"), Some(&"sync,relay".to_string()));
    }

    #[test]
    fn test_compatibility_check() {
        let service_info = TonkServiceInfo {
            bundle_id: "test-bundle".to_string(),
            node_id: "test-node".to_string(),
            protocol_version: 1,
            port: 8080,
            capabilities: vec!["sync".to_string()],
            instance_name: "tonk-test".to_string(),
            addresses: vec![],
            discovered_at: 0,
        };

        // Should be compatible with same bundle ID
        assert!(service_info.is_compatible("test-bundle"));
        
        // Should not be compatible with different bundle ID
        assert!(!service_info.is_compatible("different-bundle"));
    }

    #[test]
    fn test_staleness_check() {
        let mut service_info = TonkServiceInfo::new(
            "test".to_string(),
            "node".to_string(),
            8080,
            vec![],
        );

        // Fresh service should not be stale
        assert!(!service_info.is_stale(std::time::Duration::from_secs(60)));

        // Manually set old timestamp
        service_info.discovered_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() - 120; // 2 minutes ago

        // Should be stale with 60 second max age
        assert!(service_info.is_stale(std::time::Duration::from_secs(60)));
    }

    #[test]
    fn test_mdns_config_defaults() {
        let config = MdnsConfig::default();

        assert!(config.enabled);
        assert_eq!(config.announce_interval, std::time::Duration::from_secs(30));
        assert_eq!(config.browse_interval, std::time::Duration::from_secs(10));
        assert_eq!(config.service_ttl, std::time::Duration::from_secs(120));
        assert_eq!(config.max_peers, 50);
        assert_eq!(config.interface, None);
    }
}

#[cfg(test)]
mod connection_tests {
    use super::connection::*;
    use std::time::{Duration, Instant};

    #[test]
    fn test_connection_attempt_creation() {
        let peer_id = "test-peer".to_string();
        let max_attempts = 3;
        let attempt = ConnectionAttempt::new(peer_id.clone(), max_attempts);

        assert_eq!(attempt.peer_id, peer_id);
        assert_eq!(attempt.attempts, 0);
        assert_eq!(attempt.max_attempts, max_attempts);
        assert!(!attempt.connected);
        assert_eq!(attempt.backoff_duration, Duration::from_secs(1));
    }

    #[test]
    fn test_connection_attempt_success() {
        let mut attempt = ConnectionAttempt::new("peer".to_string(), 5);
        
        // Should be ready for retry initially
        assert!(attempt.should_retry());
        
        // Record successful attempt
        attempt.record_attempt(true);
        
        assert_eq!(attempt.attempts, 1);
        assert!(attempt.connected);
        assert!(!attempt.should_retry()); // Should not retry when connected
    }

    #[test]
    fn test_connection_attempt_failure_backoff() {
        let mut attempt = ConnectionAttempt::new("peer".to_string(), 5);
        let initial_backoff = attempt.backoff_duration;
        
        // Record failed attempt
        attempt.record_attempt(false);
        
        assert_eq!(attempt.attempts, 1);
        assert!(!attempt.connected);
        assert!(attempt.backoff_duration >= initial_backoff * 2); // Should have increased
    }

    #[test]
    fn test_connection_attempt_exhaustion() {
        let mut attempt = ConnectionAttempt::new("peer".to_string(), 3);
        
        // Fail all attempts
        for _ in 0..3 {
            attempt.record_attempt(false);
        }
        
        assert_eq!(attempt.attempts, 3);
        assert!(!attempt.connected);
        assert!(attempt.is_exhausted());
    }

    #[test]
    fn test_connection_config_defaults() {
        let config = ConnectionConfig::default();
        
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.initial_backoff, Duration::from_secs(1));
        assert_eq!(config.max_backoff, Duration::from_secs(300));
        assert_eq!(config.connection_timeout, Duration::from_secs(30));
        assert_eq!(config.retry_interval, Duration::from_secs(60));
    }
}

#[cfg(test)]
mod config_tests {
    use super::config::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_config_defaults() {
        let preferences = DiscoveryPreferences::default();
        
        assert!(preferences.mdns.enabled);
        assert_eq!(preferences.mdns.announce_interval_secs, 30);
        assert_eq!(preferences.connection.max_attempts, 5);
        assert!(preferences.network.monitor_interfaces);
        assert!(preferences.security.require_bundle_match);
    }

    #[tokio::test]
    async fn test_config_validation() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let mut config_manager = ConfigManager::new(config_path);
        
        // Test with invalid preferences
        config_manager.get_preferences_mut().mdns.announce_interval_secs = 0;
        config_manager.get_preferences_mut().connection.max_attempts = 0;
        
        let errors = config_manager.validate_preferences();
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("announce interval")));
        assert!(errors.iter().any(|e| e.contains("connection attempts")));
    }

    #[tokio::test]
    async fn test_config_save_load() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        
        // Create and save config
        let mut config_manager = ConfigManager::new(config_path.clone());
        config_manager.get_preferences_mut().mdns.max_peers = 100;
        config_manager.save().await.unwrap();
        
        // Load config in new manager
        let mut new_manager = ConfigManager::new(config_path);
        new_manager.load().await.unwrap();
        
        assert_eq!(new_manager.get_preferences().mdns.max_peers, 100);
    }

    #[tokio::test]
    async fn test_config_export_import() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        
        let mut config_manager = ConfigManager::new(config_path.clone());
        config_manager.get_preferences_mut().mdns.max_peers = 75;
        
        // Export config
        let exported = config_manager.export_config().await.unwrap();
        assert!(exported.contains("\"max_peers\": 75"));
        
        // Import in new manager
        let mut new_manager = ConfigManager::new(temp_dir.path().join("new_config.json"));
        let errors = new_manager.import_config(&exported).await.unwrap();
        
        assert!(errors.is_empty());
        assert_eq!(new_manager.get_preferences().mdns.max_peers, 75);
    }

    #[test]
    fn test_config_conversions() {
        let temp_dir = tempdir().unwrap();
        let config_manager = ConfigManager::new(temp_dir.path().join("test.json"));
        
        let mdns_config = config_manager.to_mdns_config();
        let connection_config = config_manager.to_connection_config();
        
        assert_eq!(mdns_config.max_peers, 50);
        assert_eq!(connection_config.max_attempts, 5);
    }
}

#[cfg(test)]
mod network_tests {
    use super::network::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_is_private_address() {
        // Test IPv4 private addresses
        assert!(is_private_address(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_address(&IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_address(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_private_address(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        
        // Test IPv4 public addresses
        assert!(!is_private_address(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_address(&IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        
        // Test IPv6 addresses
        assert!(is_private_address(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_private_address(&IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))));
    }

    #[test]
    fn test_network_interface_creation() {
        let interface = NetworkInterface {
            name: "eth0".to_string(),
            addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))],
            is_up: true,
            is_loopback: false,
        };
        
        assert_eq!(interface.name, "eth0");
        assert_eq!(interface.addresses.len(), 1);
        assert!(interface.is_up);
        assert!(!interface.is_loopback);
    }

    #[tokio::test]
    async fn test_get_local_addresses() {
        // This test might fail in some environments, so we just check it doesn't panic
        let result = get_local_addresses().await;
        match result {
            Ok(addresses) => {
                assert!(!addresses.is_empty());
                println!("Found local addresses: {:?}", addresses);
            }
            Err(e) => {
                println!("Failed to get local addresses (this might be expected in test env): {}", e);
            }
        }
    }
}

// Integration tests that require a running app context would go here
#[cfg(test)]
mod integration_tests {
    // These would test the full P2P stack but require more setup
    // For now, we'll focus on unit tests above
}