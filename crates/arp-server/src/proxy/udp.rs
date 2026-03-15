use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

use arp_common::crypto::{Compressor, PacketCipher};
use arp_common::protocol::{Message, NewProxyMsg, StartWorkConnMsg};
use arp_common::transport::prefixed::PrefixedStream;
use arp_common::transport::udp_mux::{read_udp_mux_frame, write_udp_mux_frame, UdpMuxFrame};
use arp_common::{Error, Result};

use crate::proxy::{Proxy, WorkConnRequest};
use crate::resource::ResourceController;

const UDP_PACKET_MAX: usize = 65535;
const WORK_CONN_TIMEOUT_SECS: u64 = 10;
const UDP_RESPONSE_TIMEOUT_SECS: u64 = 10;

pub struct UdpProxy {
    name: String,
    remote_port: u16,
    socket: Arc<UdpSocket>,
    resource_controller: Arc<ResourceController>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    use_compression: bool,
    use_encryption: bool,
    secret: String,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    tunnel: Arc<Mutex<Option<Arc<UdpTunnel>>>>,
}

struct UdpTunnel {
    frame_tx: mpsc::Sender<UdpMuxFrame>,
    pending: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>>,
    next_request_id: AtomicU32,
}

impl UdpTunnel {
    async fn new(work_transport: arp_common::transport::MessageTransport) -> Result<Arc<Self>> {
        let (io, pre_read) = work_transport.into_inner_with_read_buf();
        let io: arp_common::transport::BoxedStream = if pre_read.is_empty() {
            io
        } else {
            Box::new(PrefixedStream::new(pre_read, io))
        };

        let (mut reader, mut writer) = tokio::io::split(io);
        let (frame_tx, mut frame_rx) = mpsc::channel::<UdpMuxFrame>(1024);
        let pending: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                if let Err(e) = write_udp_mux_frame(&mut writer, &frame).await {
                    warn!("udp mux writer exited: {}", e);
                    break;
                }
            }
        });

        let pending_reader = Arc::clone(&pending);
        let frame_tx_reader = frame_tx.clone();
        tokio::spawn(async move {
            loop {
                let frame = match read_udp_mux_frame(&mut reader).await {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("udp mux reader exited: {}", e);
                        break;
                    }
                };
                match frame {
                    UdpMuxFrame::Resp {
                        request_id,
                        payload,
                    } => {
                        if let Some(tx) = pending_reader.lock().await.remove(&request_id) {
                            let _ = tx.send(payload);
                        }
                    }
                    UdpMuxFrame::Ping => {
                        let _ = frame_tx_reader.send(UdpMuxFrame::Pong).await;
                    }
                    UdpMuxFrame::Pong => {}
                    UdpMuxFrame::Req { .. } => {}
                }
            }
            pending_reader.lock().await.clear();
        });

        Ok(Arc::new(Self {
            frame_tx,
            pending,
            next_request_id: AtomicU32::new(1),
        }))
    }

    async fn roundtrip(&self, src_addr: String, payload: Vec<u8>) -> Result<Vec<u8>> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(request_id, tx);

        if self
            .frame_tx
            .send(UdpMuxFrame::Req {
                request_id,
                src_addr,
                payload,
            })
            .await
            .is_err()
        {
            self.pending.lock().await.remove(&request_id);
            return Err(Error::Transport("udp mux frame channel closed".to_string()));
        }

        tokio::time::timeout(
            tokio::time::Duration::from_secs(UDP_RESPONSE_TIMEOUT_SECS),
            rx,
        )
        .await
        .map_err(|_| Error::Timeout("udp mux response timeout".to_string()))?
        .map_err(|_| Error::Transport("udp mux response channel closed".to_string()))
    }
}

impl UdpProxy {
    pub async fn new(
        msg: NewProxyMsg,
        resource_controller: Arc<ResourceController>,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<Self> {
        let force_secure = msg.proxy_type == "sudp";
        let remote_port = if msg.remote_port > 0 {
            resource_controller
                .allocate_tcp_port(msg.remote_port)
                .await?
        } else {
            resource_controller.allocate_random_tcp_port().await?
        };

        let bind_addr = format!("0.0.0.0:{}", remote_port);
        let socket = UdpSocket::bind(&bind_addr).await.map_err(Error::Io)?;
        info!("UDP proxy {} listening on {}", msg.proxy_name, bind_addr);

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

        Ok(Self {
            name: msg.proxy_name,
            remote_port,
            socket: Arc::new(socket),
            resource_controller,
            work_conn_req_tx,
            use_compression: msg.use_compression,
            use_encryption: msg.use_encryption || force_secure,
            secret: msg.sk,
            shutdown_tx,
            tunnel: Arc::new(Mutex::new(None)),
        })
    }

    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }
}

#[async_trait]
impl Proxy for UdpProxy {
    async fn run(&self) -> Result<()> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let mut recv_buf = [0u8; UDP_PACKET_MAX];

        loop {
            tokio::select! {
                recv = self.socket.recv_from(&mut recv_buf) => {
                    match recv {
                        Ok((n, src_addr)) => {
                            let packet = recv_buf[..n].to_vec();
                            let socket = Arc::clone(&self.socket);
                            let proxy_name = self.name.clone();
                            let use_compression = self.use_compression;
                            let use_encryption = self.use_encryption;
                            let secret = self.secret.clone();
                            let tunnel = Arc::clone(&self.tunnel);
                            let work_conn_req_tx = self.work_conn_req_tx.clone();
                            let remote_port = self.remote_port;

                            tokio::spawn(async move {
                                if let Err(e) = handle_udp_packet(
                                    socket,
                                    proxy_name,
                                    packet,
                                    src_addr.to_string(),
                                    use_compression,
                                    use_encryption,
                                    secret,
                                    tunnel,
                                    work_conn_req_tx,
                                    remote_port,
                                ).await {
                                    warn!("UDP proxy packet handling failed: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("UDP proxy {} recv error: {}", self.name, e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("UDP proxy {} shutting down", self.name);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        self.tunnel.lock().await.take();
        self.resource_controller
            .release_tcp_port(self.remote_port)
            .await?;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn proxy_type(&self) -> &str {
        "udp"
    }
}

async fn handle_udp_packet(
    socket: Arc<UdpSocket>,
    proxy_name: String,
    packet: Vec<u8>,
    src_addr: String,
    use_compression: bool,
    use_encryption: bool,
    secret: String,
    tunnel_holder: Arc<Mutex<Option<Arc<UdpTunnel>>>>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    remote_port: u16,
) -> Result<()> {
    let mut encoded = packet;
    if use_compression {
        encoded = Compressor::compress(&encoded)?;
    }
    if use_encryption {
        encoded = PacketCipher::encrypt(&encoded, &secret)?;
    }

    let tunnel =
        get_or_create_udp_tunnel(&proxy_name, tunnel_holder, work_conn_req_tx, remote_port).await?;
    let resp = tunnel.roundtrip(src_addr.clone(), encoded).await?;

    let mut decoded = resp;
    if use_encryption {
        decoded = PacketCipher::decrypt(&decoded, &secret)?;
    }
    if use_compression {
        decoded = Compressor::decompress(&decoded)?;
    }

    socket
        .send_to(&decoded, &src_addr)
        .await
        .map_err(Error::Io)?;
    debug!(
        "UDP proxy {} forwarded and returned {} bytes",
        proxy_name,
        decoded.len()
    );
    Ok(())
}

async fn get_or_create_udp_tunnel(
    proxy_name: &str,
    tunnel_holder: Arc<Mutex<Option<Arc<UdpTunnel>>>>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    remote_port: u16,
) -> Result<Arc<UdpTunnel>> {
    let mut guard = tunnel_holder.lock().await;
    if let Some(tunnel) = guard.clone() {
        return Ok(tunnel);
    }

    let (work_conn_tx, work_conn_rx) = oneshot::channel();
    work_conn_req_tx
        .send(WorkConnRequest {
            proxy_name: proxy_name.to_string(),
            reply_tx: work_conn_tx,
        })
        .await
        .map_err(|_| Error::Transport("failed to request udp work connection".to_string()))?;

    let mut work_transport = tokio::time::timeout(
        tokio::time::Duration::from_secs(WORK_CONN_TIMEOUT_SECS),
        work_conn_rx,
    )
    .await
    .map_err(|_| Error::Timeout("udp work connection timeout".to_string()))?
    .map_err(|_| Error::Transport("udp work connection channel closed".to_string()))?;

    work_transport
        .send(Message::StartWorkConn(StartWorkConnMsg {
            proxy_name: proxy_name.to_string(),
            src_addr: String::new(),
            dst_addr: format!("0.0.0.0:{}", remote_port),
            error: String::new(),
        }))
        .await?;

    let tunnel = UdpTunnel::new(work_transport).await?;
    *guard = Some(Arc::clone(&tunnel));
    Ok(tunnel)
}
