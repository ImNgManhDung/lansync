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
                        let mut filtered_ips: Vec<_> = info
                            .get_addresses()
                            .iter()
                            .filter(|a| is_lan_routable_address(a))
                            .copied()
                            .collect();

                        // Fallback if all were filtered out
                        if filtered_ips.is_empty() {
                            filtered_ips = info
                                .get_addresses()
                                .iter()
                                .filter(|a| !a.is_loopback() && !a.is_unspecified())
                                .copied()
                                .collect();
                        }

                        // Sort so primary Wi-Fi/LAN addresses (192.168.*, 10.*) are tried first
                        filtered_ips.sort_by_key(address_priority);

                        let mut addresses = Vec::new();
                        for addr in filtered_ips {
                            addresses.push(SocketAddr::new(addr, port));
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

pub fn is_lan_routable_address(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            if ipv4.is_loopback() || ipv4.is_unspecified() || ipv4.is_broadcast() {
                return false;
            }
            let octets = ipv4.octets();
            // Filter CGNAT used by Tailscale and carrier-grade NAT (100.64.0.0/10)
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return false;
            }
            // Filter Docker default bridge network (172.17.0.0/16)
            if octets[0] == 172 && octets[1] == 17 {
                return false;
            }
            true
        }
        std::net::IpAddr::V6(ipv6) => {
            if ipv6.is_loopback() || ipv6.is_unspecified() {
                return false;
            }
            true
        }
    }
}

pub fn address_priority(ip: &std::net::IpAddr) -> u8 {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // Highest priority: standard Home/Office LAN 192.168.x.x
            if octets[0] == 192 && octets[1] == 168 {
                return 0;
            }
            // Second priority: 10.x.x.x
            if octets[0] == 10 {
                return 1;
            }
            // Third priority: 172.16-31.x.x (excluding 172.17)
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return 2;
            }
            3
        }
        std::net::IpAddr::V6(_) => 4,
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

    #[test]
    fn test_lan_routable_address_filtering_and_priority() {
        use std::net::{IpAddr, Ipv4Addr};

        let loopback: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let docker: IpAddr = Ipv4Addr::new(172, 17, 0, 1).into();
        let tailscale: IpAddr = Ipv4Addr::new(100, 80, 1, 2).into();
        let home_lan: IpAddr = Ipv4Addr::new(192, 168, 1, 50).into();
        let office_lan: IpAddr = Ipv4Addr::new(10, 0, 0, 20).into();

        assert!(!is_lan_routable_address(&loopback));
        assert!(!is_lan_routable_address(&docker));
        assert!(!is_lan_routable_address(&tailscale));
        assert!(is_lan_routable_address(&home_lan));
        assert!(is_lan_routable_address(&office_lan));

        let mut ips = vec![office_lan, home_lan];
        ips.sort_by_key(address_priority);
        assert_eq!(ips[0], home_lan);
        assert_eq!(ips[1], office_lan);
    }
}
