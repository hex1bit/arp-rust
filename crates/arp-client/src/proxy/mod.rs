mod tcp;
mod udp;
mod xtcp;

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

use arp_common::config::ProxyConfig;
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

pub use tcp::TcpProxy;
pub use udp::UdpProxy;
pub use xtcp::XtcpProxy;

#[async_trait]
pub trait ClientProxy: Send + Sync {
    async fn handle_work_conn(&self, transport: MessageTransport) -> Result<()>;
    async fn handle_nat_hole_client(&self, _visitor_addr: &str) -> Result<String> {
        Err(Error::Proxy(
            "nat hole not supported for this proxy".to_string(),
        ))
    }
    fn is_available(&self) -> bool {
        true
    }
    fn name(&self) -> &str;
}

pub struct ProxyManager {
    proxies: DashMap<String, Arc<dyn ClientProxy>>,
}

impl ProxyManager {
    pub fn new() -> Self {
        Self {
            proxies: DashMap::new(),
        }
    }

    pub async fn register_proxy(&self, config: ProxyConfig) {
        use arp_common::config::ProxyType;
        let proxy: Arc<dyn ClientProxy> = match config.proxy_type {
            ProxyType::Tcp | ProxyType::Stcp | ProxyType::Http | ProxyType::Https => {
                Arc::new(TcpProxy::new(config))
            }
            ProxyType::Udp | ProxyType::Sudp => Arc::new(UdpProxy::new(config)),
            ProxyType::Xtcp => Arc::new(XtcpProxy::new(config)),
        };

        self.proxies.insert(proxy.name().to_string(), proxy);
    }

    pub fn get_proxy(&self, name: &str) -> Option<Arc<dyn ClientProxy>> {
        self.proxies.get(name).map(|entry| entry.clone())
    }

    pub fn is_proxy_available(&self, name: &str) -> bool {
        self.proxies
            .get(name)
            .map(|entry| entry.is_available())
            .unwrap_or(false)
    }

    pub fn unregister_proxy(&self, name: &str) {
        self.proxies.remove(name);
    }

    pub fn list_proxy_names(&self) -> Vec<String> {
        self.proxies.iter().map(|e| e.key().clone()).collect()
    }

    pub async fn handle_nat_hole_client(&self, name: &str, visitor_addr: &str) -> Result<String> {
        let proxy = self
            .get_proxy(name)
            .ok_or_else(|| Error::Proxy(format!("Unknown proxy: {}", name)))?;
        proxy.handle_nat_hole_client(visitor_addr).await
    }
}
