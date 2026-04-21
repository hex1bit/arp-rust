use std::sync::Arc;
use tracing::{error, info, warn};

use arp_common::config::ClientConfig;
use arp_common::Result;

use crate::admin;
use crate::control::Control;
use crate::proxy::ProxyManager;

pub struct Service {
    config: Arc<ClientConfig>,
    config_path: String,
}

impl Service {
    pub async fn new(config: ClientConfig, config_path: String) -> Result<Self> {
        let config = Arc::new(config);
        Ok(Self { config, config_path })
    }

    pub async fn run(self) -> Result<()> {
        info!("Starting ARP client...");

        let mut retry_count: u32 = 0;
        let max_backoff_secs: u64 = 30;

        loop {
            let proxy_manager = Arc::new(ProxyManager::new());

            let control = match Control::new(self.config.clone(), proxy_manager.clone()).await {
                Ok(c) => Arc::new(c),
                Err(e) if e.is_retriable() => {
                    retry_count += 1;
                    let backoff = std::cmp::min(
                        max_backoff_secs,
                        1u64 << retry_count.min(5),
                    );
                    error!(
                        "Failed to connect to server: {} (retry #{} in {}s)",
                        e, retry_count, backoff
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                    continue;
                }
                Err(e) => {
                    error!("Permanent error during connect, not retrying: {}", e);
                    return Err(e);
                }
            };

            // Reset backoff on successful connection
            retry_count = 0;

            // Start admin API server if configured
            if self.config.admin_port > 0 {
                let admin_addr = if self.config.admin_addr.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    self.config.admin_addr.clone()
                };
                admin::start_admin_server(
                    admin_addr,
                    self.config.admin_port,
                    admin::AdminState {
                        control: control.clone(),
                        config: self.config.clone(),
                    },
                );
            }

            // Start SIGHUP listener for config hot-reload
            #[cfg(unix)]
            {
                let ctrl = control.clone();
                let path = self.config_path.clone();
                tokio::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sighup = match signal(SignalKind::hangup()) {
                        Ok(s) => s,
                        Err(e) => {
                            warn!("Failed to register SIGHUP handler: {}", e);
                            return;
                        }
                    };
                    loop {
                        sighup.recv().await;
                        info!("Received SIGHUP, reloading config from {}", path);
                        match ctrl.reload_proxies(&path).await {
                            Ok((added, removed)) => {
                                info!("SIGHUP reload: {} added, {} removed", added, removed);
                            }
                            Err(e) => {
                                error!("SIGHUP reload failed: {}", e);
                            }
                        }
                    }
                });
            }

            match control.run().await {
                Ok(()) => {
                    info!("Control connection closed gracefully");
                    return Ok(());
                }
                Err(e) if e.is_retriable() => {
                    retry_count += 1;
                    let backoff = std::cmp::min(
                        max_backoff_secs,
                        1u64 << retry_count.min(5),
                    );
                    warn!(
                        "Control connection lost: {} — reconnecting in {}s (attempt #{})",
                        e, backoff, retry_count
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                }
                Err(e) => {
                    error!("Permanent error, not retrying: {}", e);
                    return Err(e);
                }
            }
        }
    }
}
