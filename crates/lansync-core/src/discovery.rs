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

        // Gather all physical non-loopback IP addresses for explicit mDNS advertisement
        let ip_addrs: Vec<String> = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| !i.is_loopback())
            .map(|i| i.addr.ip().to_string())
            .collect();
        let ip_csv = ip_addrs.join(",");

        if !ip_csv.is_empty() {
            properties.insert("ip".to_string(), ip_csv.clone());
        }

        let my_service = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host_name,
            ip_csv.as_str(),
            port,
            properties,
        )
        .context("Failed to create mDNS ServiceInfo")?
        .enable_addr_auto();

        let fullname = my_service.get_fullname().to_string();
        self.daemon
            .register(my_service)
            .context("Failed to register mDNS service")?;
        self.service_fullname = Some(fullname);
        info!("Registered mDNS service for {} (IPs: {})", self.self_id, ip_csv);
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

                        let mut raw_ips: Vec<std::net::IpAddr> = info.get_addresses().iter().copied().collect();

                        // Fallback: If mDNS A-record resolution returned no addresses, extract from TXT "ip" property
                        if let (true, Some(ip_prop)) = (raw_ips.is_empty(), info.get_property_val_str("ip")) {
                            for s in ip_prop.split(',') {
                                if let Ok(ip) = s.trim().parse::<std::net::IpAddr>() {
                                    raw_ips.push(ip);
                                }
                            }
                        }

                        let mut filtered_ips: Vec<_> = raw_ips
                            .iter()
                            .filter(|a| is_lan_routable_address(a))
                            .copied()
                            .collect();

                        // Fallback if all were filtered out
                        if filtered_ips.is_empty() {
                            filtered_ips = raw_ips
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

                        info!("Discovered LAN peer: {} ({}) addrs: {:?}", peer.device_name, peer.device_id, peer.addresses);
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

