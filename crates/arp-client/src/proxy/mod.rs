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
        let proxy: Arc<dyn ClientProxy> = match config.proxy_type.as_str() {
            "tcp" => Arc::new(TcpProxy::new(config)),
            "stcp" => Arc::new(TcpProxy::new(config)),
            // HTTP/HTTPS proxy on client side still dials local TCP service.
            "http" => Arc::new(TcpProxy::new(config)),
            "https" => Arc::new(TcpProxy::new(config)),
            "udp" => Arc::new(UdpProxy::new(config)),
            "sudp" => Arc::new(UdpProxy::new(config)),
            "xtcp" => Arc::new(XtcpProxy::new(config)),
            _ => {
                tracing::warn!("Unsupported proxy type: {}", config.proxy_type);
                return;
            }
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

    pub async fn handle_nat_hole_client(&self, name: &str, visitor_addr: &str) -> Result<String> {
        let proxy = self
            .get_proxy(name)
            .ok_or_else(|| Error::Proxy(format!("Unknown proxy: {}", name)))?;
        proxy.handle_nat_hole_client(visitor_addr).await
    }
}
