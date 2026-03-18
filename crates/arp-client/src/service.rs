use std::sync::Arc;
use tracing::{error, info, warn};

use arp_common::config::ClientConfig;
use arp_common::Result;

use crate::control::Control;
use crate::proxy::ProxyManager;

pub struct Service {
    config: Arc<ClientConfig>,
}

impl Service {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let config = Arc::new(config);
        Ok(Self { config })
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
