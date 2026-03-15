use std::sync::Arc;
use tracing::{error, info};

use arp_common::config::ClientConfig;
use arp_common::Result;

use crate::control::Control;
use crate::proxy::ProxyManager;

pub struct Service {
    config: Arc<ClientConfig>,
    control: Arc<Control>,
    proxy_manager: Arc<ProxyManager>,
}

impl Service {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let config = Arc::new(config);
        let proxy_manager = Arc::new(ProxyManager::new());

        let control = Arc::new(Control::new(config.clone(), proxy_manager.clone()).await?);

        Ok(Self {
            config,
            control,
            proxy_manager,
        })
    }

    pub async fn run(self) -> Result<()> {
        info!("Starting ARP client...");

        let result = self.control.run().await;

        if let Err(e) = result {
            error!("Control connection error: {}", e);
            return Err(e);
        }

        Ok(())
    }
}
