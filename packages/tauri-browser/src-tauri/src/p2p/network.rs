use anyhow::Result;
// use if_watch::{IfEvent, IfWatcher}; // Temporarily disabled due to API changes
use local_ip_address::{list_afinet_netifas, local_ip};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub is_up: bool,
    pub is_loopback: bool,
}

#[derive(Debug, Clone)]
pub struct NetworkEvent {
    pub event_type: NetworkEventType,
    pub interface: Option<NetworkInterface>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum NetworkEventType {
    InterfaceUp,
    InterfaceDown,
    AddressAdded,
    AddressRemoved,
    NetworkChanged,
}

pub struct NetworkMonitor {
    interfaces: Arc<RwLock<HashMap<String, NetworkInterface>>>,
    app_handle: AppHandle,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl NetworkMonitor {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            interfaces: Arc::new(RwLock::new(HashMap::new())),
            app_handle,
            shutdown_tx: None,
        }
    }

    pub async fn start_monitoring(&mut self) -> Result<()> {
        // Initialize current network state
        self.refresh_interfaces().await?;

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        let interfaces = Arc::clone(&self.interfaces);
        let app_handle = self.app_handle.clone();

        tokio::spawn(async move {
            // TODO: Implement proper network interface monitoring when if-watch API is clarified
            // For now, just periodically refresh interface info
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Refresh interface information periodically
                        if let Err(e) = Self::refresh_interface_cache(&interfaces).await {
                            eprintln!("Error refreshing interfaces: {}", e);
                        }

                        // Emit a generic network change event
                        if let Err(e) = app_handle.emit("network_changed", serde_json::json!({
                            "eventType": "periodic_refresh",
                            "timestamp": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        })) {
                            eprintln!("Failed to emit network change event: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        println!("Network monitor shutting down");
                        break;
                    }
                }
            }
        });

        println!("Network monitoring started (periodic mode)");
        Ok(())
    }

    // Temporarily disabled due to if-watch API changes
    // async fn handle_interface_event(...) { ... }

    async fn refresh_interface_cache(
        interfaces: &Arc<RwLock<HashMap<String, NetworkInterface>>>,
    ) -> Result<()> {
        let new_interfaces = Self::get_current_interfaces().await?;

        let mut interfaces_lock = interfaces.write().await;
        *interfaces_lock = new_interfaces;

        Ok(())
    }

    async fn get_current_interfaces() -> Result<HashMap<String, NetworkInterface>> {
        let mut interfaces = HashMap::new();

        // Get all network interfaces
        if let Ok(ifas) = list_afinet_netifas() {
            for (name, ip) in ifas {
                let interface = interfaces.entry(name.clone()).or_insert_with(|| {
                    NetworkInterface {
                        name: name.clone(),
                        addresses: Vec::new(),
                        is_up: true, // Assume up if we can get address
                        is_loopback: ip.is_loopback(),
                    }
                });

                if !interface.addresses.contains(&ip) {
                    interface.addresses.push(ip);
                }
            }
        }

        Ok(interfaces)
    }

    pub async fn refresh_interfaces(&self) -> Result<()> {
        Self::refresh_interface_cache(&self.interfaces).await
    }

    pub async fn get_interfaces(&self) -> HashMap<String, NetworkInterface> {
        let interfaces = self.interfaces.read().await;
        interfaces.clone()
    }

    pub async fn get_primary_interface(&self) -> Option<NetworkInterface> {
        // Try to get primary IP first
        if let Ok(primary_ip) = local_ip() {
            let interfaces = self.interfaces.read().await;
            for interface in interfaces.values() {
                if interface.addresses.contains(&primary_ip) {
                    return Some(interface.clone());
                }
            }
        }

        // Fall back to first non-loopback interface
        let interfaces = self.interfaces.read().await;
        for interface in interfaces.values() {
            if !interface.is_loopback && !interface.addresses.is_empty() {
                return Some(interface.clone());
            }
        }

        None
    }

    pub async fn get_all_addresses(&self) -> Vec<IpAddr> {
        let interfaces = self.interfaces.read().await;
        let mut addresses = Vec::new();

        for interface in interfaces.values() {
            if !interface.is_loopback {
                addresses.extend(interface.addresses.iter().copied());
            }
        }

        addresses
    }

    pub async fn has_network_connectivity(&self) -> bool {
        let interfaces = self.interfaces.read().await;
        interfaces
            .values()
            .any(|iface| iface.is_up && !iface.is_loopback && !iface.addresses.is_empty())
    }

    pub async fn stop_monitoring(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }
        Ok(())
    }

    // Helper method to check if network configuration changed significantly
    pub async fn network_config_changed(
        &self,
        old_interfaces: &HashMap<String, NetworkInterface>,
    ) -> bool {
        let current_interfaces = self.interfaces.read().await;

        // Check if number of interfaces changed
        if old_interfaces.len() != current_interfaces.len() {
            return true;
        }

        // Check if any interface addresses changed
        for (name, old_iface) in old_interfaces {
            if let Some(current_iface) = current_interfaces.get(name) {
                if old_iface.addresses != current_iface.addresses {
                    return true;
                }
                if old_iface.is_up != current_iface.is_up {
                    return true;
                }
            } else {
                // Interface was removed
                return true;
            }
        }

        // Check for new interfaces
        for name in current_interfaces.keys() {
            if !old_interfaces.contains_key(name) {
                return true;
            }
        }

        false
    }
}

// Helper functions for network operations
pub async fn get_local_addresses() -> Result<Vec<IpAddr>> {
    let mut addresses = Vec::new();

    // Get primary local IP
    if let Ok(ip) = local_ip() {
        addresses.push(ip);
    }

    // Get all local IPs
    if let Ok(all_ips) = list_afinet_netifas() {
        for (_, ip) in all_ips {
            if !ip.is_loopback() && !addresses.contains(&ip) {
                addresses.push(ip);
            }
        }
    }

    if addresses.is_empty() {
        return Err(anyhow::anyhow!("No local IP addresses found"));
    }

    Ok(addresses)
}

pub async fn is_local_address(addr: &IpAddr) -> bool {
    if let Ok(addresses) = get_local_addresses().await {
        addresses.contains(addr)
    } else {
        false
    }
}

pub fn is_private_address(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            octets[0] == 10
                || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                || (octets[0] == 192 && octets[1] == 168)
                || v4.is_loopback()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7
        }
    }
}

