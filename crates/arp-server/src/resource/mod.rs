use dashmap::DashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use tracing::info;

use arp_common::config::ServerConfig;
use arp_common::{Error, Result};

pub struct ResourceController {
    tcp_ports: DashMap<u16, String>,
    next_dynamic_port: AtomicU16,
    allow_ports: Vec<(u16, u16)>,
}

impl ResourceController {
    pub fn new(config: &ServerConfig) -> Self {
        let allow_ports = if config.allow_ports.is_empty() {
            vec![(10000, 65535)]
        } else {
            config
                .allow_ports
                .iter()
                .map(|range| {
                    if range.single > 0 {
                        (range.single, range.single)
                    } else {
                        (range.start, range.end)
                    }
                })
                .collect()
        };

        Self {
            tcp_ports: DashMap::new(),
            next_dynamic_port: AtomicU16::new(10000),
            allow_ports,
        }
    }

    pub async fn allocate_tcp_port(&self, port: u16) -> Result<u16> {
        if !self.is_port_allowed(port) {
            return Err(Error::Proxy(format!(
                "Port {} is not in allowed range",
                port
            )));
        }

        if self.tcp_ports.contains_key(&port) {
            return Err(Error::Proxy(format!("Port {} is already allocated", port)));
        }

        self.tcp_ports.insert(port, String::new());
        info!("Allocated TCP port: {}", port);
        Ok(port)
    }

    pub async fn allocate_random_tcp_port(&self) -> Result<u16> {
        for _ in 0..1000 {
            let port = self.next_dynamic_port.fetch_add(1, Ordering::SeqCst);
            if port >= 65535 {
                self.next_dynamic_port.store(10000, Ordering::SeqCst);
                continue;
            }

            if self.is_port_allowed(port) && !self.tcp_ports.contains_key(&port) {
                self.tcp_ports.insert(port, String::new());
                info!("Allocated random TCP port: {}", port);
                return Ok(port);
            }
        }

        Err(Error::Proxy(
            "Failed to allocate random port after 1000 attempts".to_string(),
        ))
    }

    pub async fn release_tcp_port(&self, port: u16) -> Result<()> {
        self.tcp_ports.remove(&port);
        info!("Released TCP port: {}", port);
        Ok(())
    }

    fn is_port_allowed(&self, port: u16) -> bool {
        for (start, end) in &self.allow_ports {
            if port >= *start && port <= *end {
                return true;
            }
        }
        false
    }
}
