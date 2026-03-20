mod tcp;
mod udp;
mod vhost;

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use arp_common::protocol::NewProxyMsg;
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::resource::ResourceController;

pub use tcp::TcpProxy;
pub use udp::UdpProxy;
use vhost::{HttpProxy, HttpsProxy, VhostManager};

pub struct WorkConnRequest {
    pub proxy_name: String,
    pub reply_tx: tokio::sync::oneshot::Sender<MessageTransport>,
}

#[async_trait]
pub trait Proxy: Send + Sync {
    async fn run(&self) -> Result<()>;
    async fn close(&self) -> Result<()>;
    fn name(&self) -> &str;
    fn proxy_type(&self) -> &str;
}

#[derive(Clone, serde::Serialize)]
pub struct ProxyRecord {
    pub run_id: String,
    pub proxy_name: String,
    pub proxy_type: String,
    pub remote_addr: String,
}

pub struct ProxyManager {
    proxies: DashMap<String, Arc<dyn Proxy>>,
    records: DashMap<String, ProxyRecord>,
    vhost_manager: Arc<VhostManager>,
    tcp_groups: Arc<DashMap<String, Arc<tcp::GroupedTcpProxy>>>,
    xtcp_registry: Arc<DashMap<String, XtcpProxyInfo>>,
}

#[derive(Clone)]
pub struct XtcpProxyInfo {
    pub run_id: String,
    pub sk: String,
    pub relay_addr: String,
}

impl ProxyManager {
    pub fn new(
        bind_addr: String,
        subdomain_host: String,
        vhost_http_port: u16,
        vhost_https_port: u16,
    ) -> Self {
        Self {
            proxies: DashMap::new(),
            records: DashMap::new(),
            vhost_manager: Arc::new(VhostManager::new(
                bind_addr,
                subdomain_host,
                vhost_http_port,
                vhost_https_port,
            )),
            tcp_groups: Arc::new(DashMap::new()),
            xtcp_registry: Arc::new(DashMap::new()),
        }
    }

    pub async fn register_proxy(
        &self,
        run_id: &str,
        msg: NewProxyMsg,
        resource_controller: Arc<ResourceController>,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<String> {
        let proxy_key = format!("{}:{}", run_id, msg.proxy_name);

        if self.proxies.contains_key(&proxy_key) {
            return Err(Error::Proxy(format!(
                "Proxy {} already exists",
                msg.proxy_name
            )));
        }

        let proxy_name_meta = msg.proxy_name.clone();
        let proxy_type_meta = msg.proxy_type.clone();
        let (proxy, remote_addr): (Arc<dyn Proxy>, String) = match msg.proxy_type.as_str() {
            "tcp" | "stcp" => {
                let (lb_group, lb_group_key) = parse_lb_group(&msg);
                if lb_group.is_empty() {
                    let tcp_proxy =
                        TcpProxy::new(msg, resource_controller, work_conn_req_tx).await?;
                    let remote_port = tcp_proxy.remote_port();
                    let remote_addr = format!("0.0.0.0:{}", remote_port);
                    (Arc::new(tcp_proxy), remote_addr)
                } else {
                    if msg.remote_port == 0 {
                        return Err(Error::Proxy(
                            "tcp/stcp load_balancer.group requires fixed remote_port".to_string(),
                        ));
                    }

                    let group_id = format!(
                        "{}:{}:{}:{}",
                        msg.proxy_type, msg.remote_port, lb_group, lb_group_key
                    );
                    let grouped = if let Some(existing) = self.tcp_groups.get(&group_id) {
                        existing.clone()
                    } else {
                        let created = Arc::new(
                            tcp::GroupedTcpProxy::new(
                                group_id.clone(),
                                msg.remote_port,
                                resource_controller.clone(),
                            )
                            .await?,
                        );
                        self.tcp_groups.insert(group_id.clone(), created.clone());
                        created
                    };

                    let backend_id = proxy_key.clone();
                    grouped
                        .add_backend(tcp::GroupedBackend {
                            backend_id: backend_id.clone(),
                            proxy_name: msg.proxy_name.clone(),
                            work_conn_req_tx: work_conn_req_tx.clone(),
                            stcp: msg.proxy_type == "stcp",
                            secret: msg.sk.clone(),
                        })
                        .await;
                    grouped.ensure_started();
                    let remote_addr = format!("0.0.0.0:{}", grouped.remote_port());
                    let handle = tcp::GroupedMemberProxy::new(
                        msg.proxy_name.clone(),
                        grouped,
                        backend_id,
                        self.tcp_groups.clone(),
                    );
                    (Arc::new(handle), remote_addr)
                }
            }
            "http" => {
                let remote_addr =
                    self.vhost_manager
                        .register_http(run_id, &msg, work_conn_req_tx.clone())?;
                let route_key = format!("{}:{}", run_id, msg.proxy_name);
                (
                    Arc::new(HttpProxy::new(route_key, Arc::clone(&self.vhost_manager))),
                    remote_addr,
                )
            }
            "https" => {
                let remote_addr =
                    self.vhost_manager
                        .register_https(run_id, &msg, work_conn_req_tx.clone())?;
                let route_key = format!("{}:{}", run_id, msg.proxy_name);
                (
                    Arc::new(HttpsProxy::new(route_key, Arc::clone(&self.vhost_manager))),
                    remote_addr,
                )
            }
            "udp" | "sudp" => {
                let udp_proxy = UdpProxy::new(msg, resource_controller, work_conn_req_tx).await?;
                let remote_port = udp_proxy.remote_port();
                let remote_addr = format!("0.0.0.0:{}", remote_port);
                (Arc::new(udp_proxy), remote_addr)
            }
            "xtcp" => {
                if msg.sk.trim().is_empty() {
                    return Err(Error::Proxy("xtcp proxy requires sk".to_string()));
                }
                if self.xtcp_registry.contains_key(&msg.proxy_name) {
                    return Err(Error::Proxy(format!(
                        "xtcp proxy {} already exists",
                        msg.proxy_name
                    )));
                }
                let relay_proxy = if msg.fallback_to_relay {
                    let mut relay_msg = msg.clone();
                    relay_msg.proxy_name = format!("{}__relay", msg.proxy_name);
                    relay_msg.proxy_type = "tcp".to_string();
                    relay_msg.sk.clear();
                    relay_msg.multiplexer.clear();
                    Some(Arc::new(
                        TcpProxy::new(
                            relay_msg,
                            resource_controller.clone(),
                            work_conn_req_tx.clone(),
                        )
                        .await?,
                    ))
                } else {
                    None
                };
                let relay_addr = relay_proxy
                    .as_ref()
                    .map(|proxy| format!("0.0.0.0:{}", proxy.remote_port()))
                    .unwrap_or_default();

                self.xtcp_registry.insert(
                    msg.proxy_name.clone(),
                    XtcpProxyInfo {
                        run_id: run_id.to_string(),
                        sk: msg.sk.clone(),
                        relay_addr: relay_addr.clone(),
                    },
                );
                let remote_addr = if relay_addr.is_empty() {
                    "xtcp".to_string()
                } else {
                    format!("xtcp(relay={})", relay_addr)
                };
                (
                    Arc::new(XtcpProxy::new(
                        msg.proxy_name.clone(),
                        self.xtcp_registry.clone(),
                        relay_proxy,
                    )),
                    remote_addr,
                )
            }
            _ => {
                return Err(Error::Proxy(format!(
                    "Unsupported proxy type: {}",
                    msg.proxy_type
                )));
            }
        };

        let proxy_clone = proxy.clone();
        let proxy_name = proxy.name().to_string();
        tokio::spawn(async move {
            info!("Starting proxy: {}", proxy_name);
            if let Err(e) = proxy_clone.run().await {
                error!("Proxy {} error: {}", proxy_name, e);
            }
        });

        self.proxies.insert(proxy_key.clone(), proxy.clone());
        self.records.insert(
            proxy_key.clone(),
            ProxyRecord {
                run_id: run_id.to_string(),
                proxy_name: proxy_name_meta,
                proxy_type: proxy_type_meta,
                remote_addr: remote_addr.clone(),
            },
        );
        info!("Proxy registered: {} -> {}", proxy_key, remote_addr);

        Ok(remote_addr)
    }

    pub async fn unregister_proxy(&self, run_id: &str, proxy_name: &str) -> Result<()> {
        let proxy_key = format!("{}:{}", run_id, proxy_name);

        if let Some((_, proxy)) = self.proxies.remove(&proxy_key) {
            info!("Unregistering proxy: {}", proxy_key);
            self.records.remove(&proxy_key);
            proxy.close().await?;
        }

        Ok(())
    }

    /// Evict any existing proxy with the given name (from any run_id).
    /// Returns the run_id of the evicted proxy's owner if one was found.
    pub async fn evict_proxy_by_name(&self, proxy_name: &str) -> Option<String> {
        // Find any existing proxy with this name (format: "{run_id}:{proxy_name}")
        let existing_key = self.proxies.iter().find_map(|entry| {
            let key = entry.key();
            if key.ends_with(&format!(":{}", proxy_name)) {
                Some(key.clone())
            } else {
                None
            }
        });

        if let Some(key) = existing_key {
            let owner_run_id = key.splitn(2, ':').next().unwrap_or("").to_string();
            if let Some((_, proxy)) = self.proxies.remove(&key) {
                self.records.remove(&key);
                warn!(
                    "Evicting stale proxy {} (owned by run_id {})",
                    proxy_name, owner_run_id
                );
                if let Err(e) = proxy.close().await {
                    error!("Failed to close evicted proxy {}: {}", key, e);
                }
            }
            // Also clean xtcp registry if applicable
            self.xtcp_registry.remove(proxy_name);
            Some(owner_run_id)
        } else {
            None
        }
    }

    pub async fn unregister_run_proxies(&self, run_id: &str) -> Result<()> {
        let prefix = format!("{}:", run_id);
        let keys: Vec<String> = self
            .proxies
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                if key.starts_with(&prefix) {
                    Some(key.clone())
                } else {
                    None
                }
            })
            .collect();

        for key in keys {
            if let Some((_, proxy)) = self.proxies.remove(&key) {
                self.records.remove(&key);
                info!("Unregistering proxy: {}", key);
                proxy.close().await?;
            }
        }

        Ok(())
    }

    pub fn count(&self) -> usize {
        self.proxies.len()
    }

    pub fn list_proxy_keys(&self) -> Vec<String> {
        self.proxies
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn list_records(&self) -> Vec<ProxyRecord> {
        let mut items: Vec<ProxyRecord> = self.records.iter().map(|entry| entry.value().clone()).collect();
        items.sort_by(|a, b| a.proxy_name.cmp(&b.proxy_name).then_with(|| a.run_id.cmp(&b.run_id)));
        items
    }

    pub fn get_record(&self, proxy_name: &str) -> Option<ProxyRecord> {
        self.records
            .iter()
            .find_map(|entry| {
                let value = entry.value();
                if value.proxy_name == proxy_name {
                    Some(value.clone())
                } else {
                    None
                }
            })
    }

    pub fn count_by_type(&self) -> Vec<(String, usize)> {
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for record in self.records.iter() {
            *counts.entry(record.proxy_type.clone()).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }

    pub fn get_xtcp_owner(&self, proxy_name: &str) -> Option<XtcpProxyInfo> {
        self.xtcp_registry
            .get(proxy_name)
            .map(|entry| entry.value().clone())
    }
}

fn parse_lb_group(msg: &NewProxyMsg) -> (String, String) {
    let mut group = String::new();
    let mut group_key = String::new();

    if let Some(obj) = msg.extra.as_object() {
        if let Some(v) = obj.get("group").and_then(|v| v.as_str()) {
            group = v.trim().to_string();
        }
        if let Some(v) = obj.get("group_key").and_then(|v| v.as_str()) {
            group_key = v.trim().to_string();
        }
        if let Some(lb) = obj.get("load_balancer").and_then(|v| v.as_object()) {
            if group.is_empty() {
                if let Some(v) = lb.get("group").and_then(|v| v.as_str()) {
                    group = v.trim().to_string();
                }
            }
            if group_key.is_empty() {
                if let Some(v) = lb.get("group_key").and_then(|v| v.as_str()) {
                    group_key = v.trim().to_string();
                }
            }
        }
    }

    (group, group_key)
}

struct XtcpProxy {
    name: String,
    xtcp_registry: Arc<DashMap<String, XtcpProxyInfo>>,
    relay_proxy: Option<Arc<TcpProxy>>,
}

impl XtcpProxy {
    fn new(
        name: String,
        xtcp_registry: Arc<DashMap<String, XtcpProxyInfo>>,
        relay_proxy: Option<Arc<TcpProxy>>,
    ) -> Self {
        Self {
            name,
            xtcp_registry,
            relay_proxy,
        }
    }
}

#[async_trait]
impl Proxy for XtcpProxy {
    async fn run(&self) -> Result<()> {
        if let Some(relay) = &self.relay_proxy {
            relay.run().await?;
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if let Some(relay) = &self.relay_proxy {
            relay.close().await?;
        }
        self.xtcp_registry.remove(&self.name);
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn proxy_type(&self) -> &str {
        "xtcp"
    }
}
