use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, warn};

use arp_common::config::ProxyConfig;
use arp_common::transport::mux::{read_mux_frame, write_mux_frame, MuxFrame};
use arp_common::transport::prefixed::PrefixedStream;
use arp_common::transport::relay::relay_stcp;
use arp_common::transport::{copy_bidirectional, MessageTransport};
use arp_common::{Error, Result};

use crate::proxy::ClientProxy;

pub struct TcpProxy {
    name: String,
    local_ip: String,
    local_port: u16,
    tcp_mux: bool,
    stcp: bool,
    secret: String,
    health_check_enable: bool,
    health_check_timeout: Duration,
    health_check_interval: Duration,
    health_check_max_failed: u32,
    health_last_check: Arc<Mutex<Instant>>,
    health_failed: Arc<AtomicU32>,
    healthy: Arc<AtomicBool>,
}

impl TcpProxy {
    pub fn new(config: ProxyConfig) -> Self {
        let stcp = config.proxy_type == arp_common::config::ProxyType::Stcp;
        let tcp_mux = matches!(
            config.multiplexer.as_str(),
            "tcp_mux" | "mux" | "smux" | "yamux"
        ) && !stcp;
        Self {
            name: config.name,
            local_ip: config.local_ip,
            local_port: config.local_port,
            tcp_mux,
            stcp,
            secret: config.sk,
            health_check_enable: config.health_check.enable,
            health_check_timeout: Duration::from_secs(config.health_check.timeout_seconds.max(2)),
            health_check_interval: Duration::from_secs(config.health_check.interval_seconds.max(5)),
            health_check_max_failed: config.health_check.max_failed.max(1),
            health_last_check: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(3600))),
            health_failed: Arc::new(AtomicU32::new(0)),
            healthy: Arc::new(AtomicBool::new(true)),
        }
    }

    async fn handle_mux_work_conn(&self, work_transport: MessageTransport) -> Result<()> {
        let local_addr = format!("{}:{}", self.local_ip, self.local_port);
        let (io, pre_read_data) = work_transport.into_inner_with_read_buf();
        let io: arp_common::transport::BoxedStream = if pre_read_data.is_empty() {
            io
        } else {
            Box::new(PrefixedStream::new(pre_read_data, io))
        };

        let (mut reader, mut writer) = tokio::io::split(io);
        let (frame_tx, mut frame_rx) = mpsc::channel::<MuxFrame>(1024);
        let streams: Arc<Mutex<HashMap<u32, mpsc::Sender<bytes::Bytes>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let writer_task = tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                if let Err(e) = write_mux_frame(&mut writer, &frame).await {
                    return Err(e);
                }
            }
            Ok::<(), Error>(())
        });

        loop {
            let frame = match read_mux_frame(&mut reader).await {
                Ok(f) => f,
                Err(e) => {
                    warn!("TCP mux frame read failed for {}: {}", self.name, e);
                    break;
                }
            };

            match frame {
                MuxFrame::Open { stream_id } => {
                    let (tx, rx) = mpsc::channel::<bytes::Bytes>(256);
                    streams.lock().await.insert(stream_id, tx);
                    let frame_tx_clone = frame_tx.clone();
                    let streams_clone = Arc::clone(&streams);
                    let proxy_name = self.name.clone();
                    let local_addr_clone = local_addr.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_local_stream(
                            stream_id,
                            local_addr_clone,
                            rx,
                            frame_tx_clone,
                            streams_clone,
                        )
                        .await
                        {
                            error!(
                                "TCP mux local stream handling failed for {} stream {}: {}",
                                proxy_name, stream_id, e
                            );
                        }
                    });
                }
                MuxFrame::Data { stream_id, payload } => {
                    let tx = streams.lock().await.get(&stream_id).cloned();
                    if let Some(tx) = tx {
                        if tx.send(payload).await.is_err() {
                            streams.lock().await.remove(&stream_id);
                        }
                    } else {
                        let _ = frame_tx.send(MuxFrame::Close { stream_id }).await;
                    }
                }
                MuxFrame::Close { stream_id } => {
                    streams.lock().await.remove(&stream_id);
                }
                MuxFrame::Ping => {
                    let _ = frame_tx.send(MuxFrame::Pong).await;
                }
                MuxFrame::Pong => {}
            }
        }

        streams.lock().await.clear();
        drop(frame_tx);
        let _ = writer_task.await;
        Ok(())
    }
}

#[async_trait]
impl ClientProxy for TcpProxy {
    async fn handle_work_conn(&self, work_transport: MessageTransport) -> Result<()> {
        self.check_health().await?;
        if self.tcp_mux {
            debug!("TCP proxy {} running in tcp_mux mode", self.name);
            return self.handle_mux_work_conn(work_transport).await;
        }
        if self.stcp {
            debug!("TCP proxy {} running in stcp secure mode", self.name);
            return self.handle_stcp_work_conn(work_transport).await;
        }

        debug!(
            "TCP proxy {} handling work connection, connecting to {}:{}",
            self.name, self.local_ip, self.local_port
        );

        let local_addr = format!("{}:{}", self.local_ip, self.local_port);
        let mut local_stream = TcpStream::connect(&local_addr).await.map_err(|e| {
            Error::Proxy(format!(
                "TCP proxy {} failed to connect local service {}: {}",
                self.name, local_addr, e
            ))
        })?;

        debug!(
            "TCP proxy {} connected to local service: {}",
            self.name, local_addr
        );

        let (mut work_stream, pre_read_data) = work_transport.into_inner_with_read_buf();
        if !pre_read_data.is_empty() {
            // Flush bytes already decoded by framed transport before switching to raw copy.
            local_stream
                .write_all(&pre_read_data)
                .await
                .map_err(Error::Io)?;
        }

        match copy_bidirectional(&mut local_stream, &mut work_stream).await {
            Ok((local_to_work, work_to_local)) => {
                debug!(
                    "TCP proxy {} connection closed: sent {} bytes, received {} bytes",
                    self.name, local_to_work, work_to_local
                );
            }
            Err(e) => {
                error!("TCP proxy {} transfer error: {}", self.name, e);
                return Err(e);
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        if !self.health_check_enable {
            return true;
        }
        self.healthy.load(Ordering::Relaxed)
    }
}

impl TcpProxy {
    async fn check_health(&self) -> Result<()> {
        if !self.health_check_enable {
            return Ok(());
        }
        let now = Instant::now();
        {
            let mut last = self.health_last_check.lock().await;
            if now.duration_since(*last) < self.health_check_interval {
                if !self.healthy.load(Ordering::Relaxed) {
                    return Err(Error::Proxy(format!(
                        "proxy {} local endpoint {}:{} is unhealthy",
                        self.name, self.local_ip, self.local_port
                    )));
                }
                return Ok(());
            }
            *last = now;
        }

        let addr = format!("{}:{}", self.local_ip, self.local_port);
        let ok = tokio::time::timeout(self.health_check_timeout, TcpStream::connect(&addr))
            .await
            .is_ok_and(|r| r.is_ok());
        if ok {
            self.health_failed.store(0, Ordering::Relaxed);
            self.healthy.store(true, Ordering::Relaxed);
            return Ok(());
        }

        let failed = self.health_failed.fetch_add(1, Ordering::Relaxed) + 1;
        if failed >= self.health_check_max_failed {
            self.healthy.store(false, Ordering::Relaxed);
        }
        if !self.healthy.load(Ordering::Relaxed) {
            return Err(Error::Proxy(format!(
                "proxy {} local endpoint {}:{} is unhealthy (failed={})",
                self.name, self.local_ip, self.local_port, failed
            )));
        }
        Ok(())
    }

    async fn handle_stcp_work_conn(&self, work_transport: MessageTransport) -> Result<()> {
        let local_addr = format!("{}:{}", self.local_ip, self.local_port);
        let local_stream = TcpStream::connect(&local_addr).await.map_err(|e| {
            Error::Proxy(format!(
                "STCP proxy {} failed to connect local service {}: {}",
                self.name, local_addr, e
            ))
        })?;

        let (work_stream, pre_read_data) = work_transport.into_inner_with_read_buf();
        let work_stream: arp_common::transport::BoxedStream = if pre_read_data.is_empty() {
            work_stream
        } else {
            Box::new(PrefixedStream::new(pre_read_data, work_stream))
        };

        relay_stcp(local_stream, work_stream, &self.secret).await?;
        Ok(())
    }
}

async fn handle_local_stream(
    stream_id: u32,
    local_addr: String,
    mut rx: mpsc::Receiver<bytes::Bytes>,
    frame_tx: mpsc::Sender<MuxFrame>,
    streams: Arc<Mutex<HashMap<u32, mpsc::Sender<bytes::Bytes>>>>,
) -> Result<()> {
    let local_stream = TcpStream::connect(&local_addr).await.map_err(|e| {
        Error::Proxy(format!(
            "tcp_mux failed to connect local service {} for stream {}: {}",
            local_addr, stream_id, e
        ))
    })?;
    let (mut local_reader, mut local_writer) = local_stream.into_split();

    let writer_task = tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            if local_writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = local_reader.read(&mut buf).await.map_err(Error::Io)?;
        if n == 0 {
            break;
        }
        frame_tx
            .send(MuxFrame::Data {
                stream_id,
                payload: bytes::Bytes::copy_from_slice(&buf[..n]),
            })
            .await
            .map_err(|_| Error::Transport("tcp_mux frame channel closed".to_string()))?;
    }

    let _ = frame_tx.send(MuxFrame::Close { stream_id }).await;
    streams.lock().await.remove(&stream_id);
    writer_task.abort();
    Ok(())
}
