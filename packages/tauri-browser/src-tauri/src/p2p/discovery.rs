use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::RwLock;
use tokio::time::sleep;

use super::mdns::{MdnsConfig, MdnsDiscovery, TonkServiceInfo};
use super::network::{NetworkInterface, NetworkMonitor};

pub struct DiscoveryManager {
    bundle_id: String,
    node_id: String,
    port: u16,
    app_handle: AppHandle,
    mdns: Arc<RwLock<Option<MdnsDiscovery>>>,
    network_monitor: Option<NetworkMonitor>,
    enabled: bool,
}

impl DiscoveryManager {
    pub fn new(bundle_id: String, node_id: String, port: u16, app_handle: AppHandle) -> Self {
        Self {
            bundle_id,
            node_id,
            port,
            app_handle: app_handle.clone(),
            mdns: Arc::new(RwLock::new(None)),
            network_monitor: Some(NetworkMonitor::new(app_handle)),
            enabled: true,
        }
    }

    pub async fn start_mdns_discovery(&mut self) -> Result<()> {
        if !self.enabled {
            println!("mDNS discovery is disabled");
            return Ok(());
        }

        // Start network monitoring first
        if let Some(network_monitor) = &mut self.network_monitor {
            network_monitor.start_monitoring().await?;
            println!("Network monitoring started");
        }

        println!("Starting mDNS discovery for bundle: {}", self.bundle_id);

        let mut mdns_discovery = MdnsDiscovery::new(
            self.bundle_id.clone(),
            self.node_id.clone(),
            self.port,
            self.app_handle.clone(),
        )
        .await?;

        // Start announcing our service
        mdns_discovery.start_announcing().await?;

        // Start browsing for other services
        mdns_discovery.start_browsing().await?;

        // Store the discovery instance
        *self.mdns.write().await = Some(mdns_discovery);

        println!("mDNS discovery started successfully");
        Ok(())
    }

    pub async fn start_dht_discovery(&self) -> Result<()> {
        println!("Starting DHT discovery for bundle: {}", self.bundle_id);

        // TODO: Implement DHT discovery using Iroh gossip
        let bundle_id = self.bundle_id.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(15)).await;
                println!("DHT discovery heartbeat for bundle: {}", bundle_id);
            }
        });

        Ok(())
    }

    pub async fn stop_mdns_discovery(&mut self) -> Result<()> {
        let mut mdns_lock = self.mdns.write().await;
        if let Some(mdns) = mdns_lock.as_mut() {
            mdns.stop_announcing().await?;
            mdns.stop_browsing().await?;
            println!("mDNS discovery stopped");
        }
        *mdns_lock = None;

        // Stop network monitoring
        if let Some(network_monitor) = &mut self.network_monitor {
            network_monitor.stop_monitoring().await?;
            println!("Network monitoring stopped");
        }

        Ok(())
    }

    pub async fn get_discovered_peers(&self) -> Vec<TonkServiceInfo> {
        let mdns_lock = self.mdns.read().await;
        if let Some(mdns) = mdns_lock.as_ref() {
            mdns.get_discovered_peers().await
        } else {
            Vec::new()
        }
    }

    pub async fn update_mdns_config(&self, config: MdnsConfig) -> Result<()> {
        let mut mdns_lock = self.mdns.write().await;
        if let Some(mdns) = mdns_lock.as_mut() {
            mdns.update_config(config);
        }
        Ok(())
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub async fn restart_discovery(&mut self) -> Result<()> {
        println!("Restarting discovery services...");

        // Stop current discovery
        self.stop_mdns_discovery().await?;

        // Small delay to ensure cleanup
        sleep(Duration::from_millis(100)).await;

        // Restart mDNS discovery
        self.start_mdns_discovery().await?;

        // Restart DHT discovery
        self.start_dht_discovery().await?;

        Ok(())
    }

    pub async fn get_network_interfaces(
        &self,
    ) -> Option<std::collections::HashMap<String, NetworkInterface>> {
        if let Some(network_monitor) = &self.network_monitor {
            Some(network_monitor.get_interfaces().await)
        } else {
            None
        }
    }

    pub async fn get_primary_interface(&self) -> Option<NetworkInterface> {
        if let Some(network_monitor) = &self.network_monitor {
            network_monitor.get_primary_interface().await
        } else {
            None
        }
    }

    pub async fn has_network_connectivity(&self) -> bool {
        if let Some(network_monitor) = &self.network_monitor {
            network_monitor.has_network_connectivity().await
        } else {
            false
        }
    }

    pub async fn refresh_network_info(&self) -> Result<()> {
        if let Some(network_monitor) = &self.network_monitor {
            network_monitor.refresh_interfaces().await?;
        }
        Ok(())
    }
}
