use anyhow::{anyhow, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;

const SERVICE_TYPE: &str = "_tonk-sync._tcp.local.";
const SERVICE_TTL: u32 = 120; // 2 minutes
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonkServiceInfo {
    pub bundle_id: String,
    pub node_id: String,
    pub protocol_version: u16,
    pub port: u16,
    pub capabilities: Vec<String>,
    pub instance_name: String,
    pub addresses: Vec<IpAddr>,
    pub discovered_at: u64,
}

impl TonkServiceInfo {
    pub fn new(bundle_id: String, node_id: String, port: u16, addresses: Vec<IpAddr>) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            bundle_id: bundle_id.clone(),
            node_id: node_id.clone(),
            protocol_version: 1,
            port,
            capabilities: vec!["sync".to_string(), "relay".to_string()],
            instance_name: format!("tonk-{}", &bundle_id[..8]),
            addresses,
            discovered_at: timestamp,
        }
    }

    pub fn to_txt_properties(&self) -> HashMap<String, String> {
        let mut properties = HashMap::new();
        properties.insert("bundle_id".to_string(), self.bundle_id.clone());
        properties.insert("node_id".to_string(), self.node_id.clone());
        properties.insert(
            "protocol_version".to_string(),
            self.protocol_version.to_string(),
        );
        properties.insert("capabilities".to_string(), self.capabilities.join(","));
        properties.insert("port".to_string(), self.port.to_string());
        properties
    }

    pub fn from_service_info(service_info: &ServiceInfo) -> Result<Self> {
        let properties = service_info.get_properties();
        println!("Parsing service info from: {} with properties: {:?}", 
            service_info.get_fullname(), properties);

        let mut bundle_id = properties
            .get("bundle_id")
            .ok_or_else(|| anyhow!("Missing bundle_id in service properties"))?
            .to_string();
        println!("Raw bundle_id: '{}'", bundle_id);

        // Strip "bundle_id=" prefix if present
        if bundle_id.starts_with("bundle_id=") {
            bundle_id = bundle_id.strip_prefix("bundle_id=").unwrap().to_string();
        }

        let mut node_id = properties
            .get("node_id")
            .ok_or_else(|| anyhow!("Missing node_id in service properties"))?
            .to_string();
        println!("Raw node_id: '{}'", node_id);

        // Strip "node_id=" prefix if present
        if node_id.starts_with("node_id=") {
            node_id = node_id.strip_prefix("node_id=").unwrap().to_string();
        }

        let protocol_version = properties
            .get("protocol_version")
            .and_then(|v| v.to_string().parse().ok())
            .unwrap_or(1);

        let port = properties
            .get("port")
            .and_then(|v| v.to_string().parse().ok())
            .unwrap_or(service_info.get_port());

        let capabilities = properties
            .get("capabilities")
            .map(|c| c.to_string().split(',').map(|s| s.to_string()).collect())
            .unwrap_or_else(|| vec!["sync".to_string()]);

        let addresses = service_info.get_addresses().iter().copied().collect();
        let instance_name = service_info.get_fullname().to_string();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(Self {
            bundle_id,
            node_id,
            protocol_version,
            port,
            capabilities,
            instance_name,
            addresses,
            discovered_at: timestamp,
        })
    }

    pub fn is_compatible(&self, our_bundle_id: &str) -> bool {
        self.bundle_id == our_bundle_id && self.protocol_version >= 1
    }

    pub fn is_stale(&self, max_age: Duration) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        (now - self.discovered_at) > max_age.as_secs()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MdnsConfig {
    pub enabled: bool,
    pub announce_interval: Duration,
    pub browse_interval: Duration,
    pub service_ttl: Duration,
    pub max_peers: usize,
    pub interface: Option<String>,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            announce_interval: Duration::from_secs(30),
            browse_interval: Duration::from_secs(10),
            service_ttl: Duration::from_secs(120),
            max_peers: 50,
            interface: None,
        }
    }
}

pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    bundle_id: String,
    node_id: String,
    port: u16,
    config: MdnsConfig,
    discovered_peers: Arc<RwLock<HashMap<String, TonkServiceInfo>>>,
    app_handle: AppHandle,
    service_registered: Arc<RwLock<bool>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl MdnsDiscovery {
    pub async fn new(
        bundle_id: String,
        node_id: String,
        port: u16,
        app_handle: AppHandle,
    ) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let config = MdnsConfig::default();

        Ok(Self {
            daemon,
            bundle_id,
            node_id,
            port,
            config,
            discovered_peers: Arc::new(RwLock::new(HashMap::new())),
            app_handle,
            service_registered: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
        })
    }

    pub async fn start_announcing(&mut self) -> Result<()> {
        let local_addresses = self.get_local_addresses().await?;

        let service_info = TonkServiceInfo::new(
            self.bundle_id.clone(),
            self.node_id.clone(),
            self.port,
            local_addresses,
        );

        let mdns_service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &service_info.instance_name,
            &format!("{}.local.", &service_info.instance_name),
            service_info
                .addresses
                .first()
                .ok_or_else(|| anyhow!("No local addresses available"))?,
            self.port,
            service_info.to_txt_properties(),
        )?;

        self.daemon
            .register(mdns_service_info)
            .map_err(|e| anyhow!("Failed to register mDNS service: {}", e))?;

        *self.service_registered.write().await = true;

        println!("mDNS service registered for bundle: {}", self.bundle_id);

        self.emit_discovery_event("service_announced", &service_info)
            .await?;

        Ok(())
    }

    pub async fn start_browsing(&mut self) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| anyhow!("Failed to start browsing: {}", e))?;

        let bundle_id = self.bundle_id.clone();
        let node_id = self.node_id.clone();
        let discovered_peers = Arc::clone(&self.discovered_peers);
        let app_handle = self.app_handle.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            let mut cleanup_interval = interval(config.browse_interval);

            loop {
                tokio::select! {
                    event = receiver.recv_async() => {
                        match event {
                            Ok(ServiceEvent::ServiceResolved(info)) => {
                                println!("mDNS ServiceResolved event received for: {}", info.get_fullname());
                                if let Err(e) = Self::handle_service_resolved(
                                    &info,
                                    &bundle_id,
                                    &node_id,
                                    &discovered_peers,
                                    &app_handle,
                                ).await {
                                    eprintln!("Error handling resolved service: {}", e);
                                }
                            }
                            Ok(ServiceEvent::ServiceRemoved(_service_type, instance_name)) => {
                                println!("mDNS ServiceRemoved event for: {}", instance_name);
                                Self::handle_service_removed(
                                    &instance_name,
                                    &discovered_peers,
                                    &app_handle,
                                ).await;
                            }
                            Ok(event) => {
                                println!("mDNS event: {:?}", event);
                            }
                            Err(e) => {
                                eprintln!("mDNS browse error: {}", e);
                            }
                        }
                    }
                    _ = cleanup_interval.tick() => {
                        Self::cleanup_stale_peers(&discovered_peers, &config, &app_handle).await;
                    }
                    _ = shutdown_rx.recv() => {
                        println!("Shutting down mDNS browsing");
                        break;
                    }
                }
            }
        });

        println!("Started mDNS browsing for service: {}", SERVICE_TYPE);
        Ok(())
    }

    async fn handle_service_resolved(
        info: &ServiceInfo,
        bundle_id: &str,
        node_id: &str,
        discovered_peers: &Arc<RwLock<HashMap<String, TonkServiceInfo>>>,
        app_handle: &AppHandle,
    ) -> Result<()> {
        println!("Received mDNS service resolution for: {}", info.get_fullname());
        
        let service_info = TonkServiceInfo::from_service_info(info)?;
        println!("Parsed service info: bundle_id={}, node_id={}, port={}", 
            service_info.bundle_id, service_info.node_id, service_info.port);

        // Don't discover ourselves
        if service_info.node_id == node_id {
            println!("Ignoring our own service (node_id: {})", node_id);
            return Ok(());
        }

        // Check bundle compatibility
        if !service_info.is_compatible(bundle_id) {
            println!("Service incompatible: their bundle_id='{}', our bundle_id='{}', protocol_version={}", 
                service_info.bundle_id, bundle_id, service_info.protocol_version);
            return Ok(());
        }

        // Check if this is a new peer or updated info
        let mut peers = discovered_peers.write().await;
        let is_new_peer = !peers.contains_key(&service_info.node_id);

        peers.insert(service_info.node_id.clone(), service_info.clone());
        drop(peers);

        if is_new_peer {
            println!(
                "Discovered new peer: {} for bundle: {}",
                service_info.node_id, service_info.bundle_id
            );

            app_handle.emit(
                "peer_discovered",
                serde_json::json!({
                    "peerId": service_info.node_id,
                    "bundleId": service_info.bundle_id,
                    "addresses": service_info.addresses,
                    "port": service_info.port,
                    "capabilities": service_info.capabilities
                }),
            )?;
        }

        Ok(())
    }

    async fn handle_service_removed(
        instance_name: &str,
        discovered_peers: &Arc<RwLock<HashMap<String, TonkServiceInfo>>>,
        app_handle: &AppHandle,
    ) {
        let mut peers = discovered_peers.write().await;

        // Find and remove peer by instance name
        let mut removed_peer_id = None;
        peers.retain(|peer_id, service_info| {
            if service_info.instance_name == instance_name {
                removed_peer_id = Some(peer_id.clone());
                false
            } else {
                true
            }
        });

        if let Some(peer_id) = removed_peer_id {
            println!("Peer removed: {}", peer_id);

            if let Err(e) = app_handle.emit(
                "peer_lost",
                serde_json::json!({
                    "peerId": peer_id
                }),
            ) {
                eprintln!("Failed to emit peer_lost event: {}", e);
            }
        }
    }

    async fn cleanup_stale_peers(
        discovered_peers: &Arc<RwLock<HashMap<String, TonkServiceInfo>>>,
        config: &MdnsConfig,
        app_handle: &AppHandle,
    ) {
        let mut peers = discovered_peers.write().await;
        let mut stale_peers = Vec::new();

        peers.retain(|peer_id, service_info| {
            if service_info.is_stale(config.service_ttl) {
                stale_peers.push(peer_id.clone());
                false
            } else {
                true
            }
        });

        for peer_id in stale_peers {
            println!("Removing stale peer: {}", peer_id);

            if let Err(e) = app_handle.emit(
                "peer_lost",
                serde_json::json!({
                    "peerId": peer_id
                }),
            ) {
                eprintln!("Failed to emit peer_lost event for stale peer: {}", e);
            }
        }
    }

    async fn get_local_addresses(&self) -> Result<Vec<IpAddr>> {
        let mut addresses = Vec::new();

        // Get primary local IP
        if let Ok(ip) = local_ip_address::local_ip() {
            addresses.push(ip);
        }

        // Get all local IPs if we have none
        if addresses.is_empty() {
            if let Ok(all_ips) = local_ip_address::list_afinet_netifas() {
                for (_, ip) in all_ips {
                    if !ip.is_loopback() {
                        addresses.push(ip);
                    }
                }
            }
        }

        if addresses.is_empty() {
            return Err(anyhow!("No local IP addresses found"));
        }

        Ok(addresses)
    }

    async fn emit_discovery_event(
        &self,
        event_type: &str,
        service_info: &TonkServiceInfo,
    ) -> Result<()> {
        self.app_handle.emit(
            "discovery_event",
            serde_json::json!({
                "type": event_type,
                "service": service_info
            }),
        )?;
        Ok(())
    }

    pub async fn stop_announcing(&mut self) -> Result<()> {
        if *self.service_registered.read().await {
            self.daemon.shutdown()?;
            *self.service_registered.write().await = false;
            println!("mDNS service unregistered");
        }
        Ok(())
    }

    pub async fn stop_browsing(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }
        Ok(())
    }

    pub async fn get_discovered_peers(&self) -> Vec<TonkServiceInfo> {
        let peers = self.discovered_peers.read().await;
        peers.values().cloned().collect()
    }

    pub fn update_config(&mut self, config: MdnsConfig) {
        self.config = config;
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        // Best effort cleanup
        let _ = self.daemon.shutdown();
    }
}

