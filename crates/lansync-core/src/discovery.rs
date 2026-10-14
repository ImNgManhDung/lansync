use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::broadcast;
use tracing::info;

pub const SERVICE_TYPE: &str = "_lansync._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub device_name: String,
    pub port: u16,
    pub addresses: Vec<SocketAddr>,
}

pub struct DiscoveryService {
    daemon: ServiceDaemon,
    self_id: String,
    service_fullname: Option<String>,
}

impl DiscoveryService {
    pub fn new(self_id: impl Into<String>) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("Failed to initialize mDNS daemon")?;
        Ok(Self {
            daemon,
            self_id: self_id.into(),
            service_fullname: None,
        })
    }

    pub fn register(&mut self, device_name: &str, port: u16) -> Result<()> {
        let instance_name = &self.self_id;
        let host_name = format!("{}.local.", self.self_id);

        let mut properties = HashMap::new();
        properties.insert("device_id".to_string(), self.self_id.clone());
        properties.insert("device_name".to_string(), device_name.to_string());
        properties.insert("proto".to_string(), "1".to_string());

        let my_service = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host_name,
            "", // Empty string allows mdns-sd to auto-detect local network IPs
            port,
            properties,
        )
        .context("Failed to create mDNS ServiceInfo")?;

        let fullname = my_service.get_fullname().to_string();
        self.daemon
            .register(my_service)
            .context("Failed to register mDNS service")?;
        self.service_fullname = Some(fullname);
        info!("Registered mDNS service for {}", self.self_id);
        Ok(())
    }

    pub fn browse(&self) -> Result<broadcast::Receiver<DiscoveredPeer>> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .context("Failed to browse mDNS service")?;

        let (tx, rx) = broadcast::channel(32);
        let self_id = self.self_id.clone();

        tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let dev_id = info
                            .get_property_val_str("device_id")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                info.get_fullname()
                                    .split('.')
                                    .next()
                                    .unwrap_or_default()
                                    .to_string()
                            });

                        // Ignore self
                        if dev_id == self_id || dev_id.is_empty() {
                            continue;
                        }

                        let dev_name = info
                            .get_property_val_str("device_name")
                            .unwrap_or("Unknown")
                            .to_string();
                        let port = info.get_port();
                        let mut addresses = Vec::new();
                        for addr in info.get_addresses() {
                            addresses.push(SocketAddr::new(*addr, port));
                        }

                        let peer = DiscoveredPeer {
                            device_id: dev_id,
                            device_name: dev_name,
                            port,
                            addresses,
                        };

                        let _ = tx.send(peer);
                    }
                    ServiceEvent::ServiceRemoved(_, _) => {}
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    pub fn shutdown(&self) -> Result<()> {
        if let Some(ref fullname) = self.service_fullname {
            let _ = self.daemon.unregister(fullname);
        }
        let _ = self.daemon.shutdown();
        Ok(())
    }
}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_service_lifecycle() {
        let mut service = DiscoveryService::new("DEVTEST123456789").unwrap();
        service.register("TestDevice", 42424).unwrap();
        let _rx = service.browse().unwrap();
        service.shutdown().unwrap();
    }
}
