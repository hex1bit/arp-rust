use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::{timeout, Duration};
use tracing::warn;

use arp_common::config::ProxyConfig;
use arp_common::transport::{copy_bidirectional, MessageTransport};
use arp_common::{Error, Result};

use crate::proxy::ClientProxy;

const XTCP_ACCEPT_TIMEOUT_SECS: u64 = 20;

pub struct XtcpProxy {
    name: String,
    local_ip: String,
    local_port: u16,
}

impl XtcpProxy {
    pub fn new(config: ProxyConfig) -> Self {
        Self {
            name: config.name,
            local_ip: config.local_ip,
            local_port: config.local_port,
        }
    }
}

#[async_trait]
impl ClientProxy for XtcpProxy {
    async fn handle_work_conn(&self, _transport: MessageTransport) -> Result<()> {
        Err(Error::Proxy(
            "xtcp does not handle server work connection directly".to_string(),
        ))
    }

    async fn handle_nat_hole_client(&self, visitor_addr: &str) -> Result<String> {
        let listener = TcpListener::bind("0.0.0.0:0").await.map_err(Error::Io)?;
        let listen_addr = listener.local_addr().map_err(Error::Io)?;
        let local_target = format!("{}:{}", self.local_ip, self.local_port);

        // Best-effort UDP punch packets to create NAT mapping.
        if let Ok(udp) = UdpSocket::bind("0.0.0.0:0").await {
            for _ in 0..5 {
                let _ = udp.send_to(b"arp-xtcp-punch", visitor_addr).await;
            }
        }

        let visitor_addr = visitor_addr.to_string();
        tokio::spawn(async move {
            let mut peer_stream = match race_peer_stream(listener, &visitor_addr).await {
                Ok(s) => s,
                Err(_) => return,
            };
            let Ok(mut local_stream) = TcpStream::connect(&local_target).await else {
                return;
            };
            if let Err(e) = copy_bidirectional(&mut peer_stream, &mut local_stream).await {
                warn!("xtcp relay failed: {}", e);
            }
        });

        Ok(listen_addr.to_string())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

async fn race_peer_stream(listener: TcpListener, visitor_addr: &str) -> Result<TcpStream> {
    let mut connect_task = tokio::spawn({
        let visitor_addr = visitor_addr.to_string();
        async move {
            timeout(
                Duration::from_secs(XTCP_ACCEPT_TIMEOUT_SECS),
                TcpStream::connect(&visitor_addr),
            )
            .await
        }
    });

    let mut accept_task = tokio::spawn(async move {
        timeout(
            Duration::from_secs(XTCP_ACCEPT_TIMEOUT_SECS),
            listener.accept(),
        )
        .await
    });

    tokio::select! {
        conn = &mut connect_task => {
            accept_task.abort();
            match conn {
                Ok(Ok(Ok(stream))) => Ok(stream),
                Ok(Ok(Err(e))) => Err(Error::Io(e)),
                Ok(Err(_)) => Err(Error::Timeout("xtcp provider connect timeout".to_string())),
                Err(e) => Err(Error::Transport(format!("xtcp provider connect task failed: {}", e))),
            }
        }
        acc = &mut accept_task => {
            connect_task.abort();
            match acc {
                Ok(Ok(Ok((stream, _)))) => Ok(stream),
                Ok(Ok(Err(e))) => Err(Error::Io(e)),
                Ok(Err(_)) => Err(Error::Timeout("xtcp provider accept timeout".to_string())),
                Err(e) => Err(Error::Transport(format!("xtcp provider accept task failed: {}", e))),
            }
        }
    }
}
