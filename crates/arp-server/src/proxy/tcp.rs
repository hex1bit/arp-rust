use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{debug, error, info, warn};

use arp_common::crypto::PacketCipher;
use arp_common::protocol::{Message, NewProxyMsg, StartWorkConnMsg};
use arp_common::transport::copy_bidirectional;
use arp_common::transport::mux::{read_mux_frame, write_mux_frame, MuxFrame};
use arp_common::transport::prefixed::PrefixedStream;
use arp_common::{Error, Result};

use crate::metrics;
use crate::proxy::{Proxy, WorkConnRequest};
use crate::resource::ResourceController;

const BACKEND_EJECT_SECS: u64 = 5;
const BACKEND_FAIL_THRESHOLD: u32 = 1;

pub struct TcpProxy {
    name: String,
    remote_port: u16,
    listener: Mutex<Option<TcpListener>>,
    resource_controller: Arc<ResourceController>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    close_done_tx: watch::Sender<bool>,
    close_done_rx: watch::Receiver<bool>,
    tcp_mux: bool,
    stcp: bool,
    secret: String,
    mux_tunnel: Arc<Mutex<Option<Arc<MuxTunnel>>>>,
}

struct MuxTunnel {
    frame_tx: mpsc::Sender<MuxFrame>,
    streams: Arc<Mutex<HashMap<u32, mpsc::Sender<Vec<u8>>>>>,
    next_stream_id: AtomicU32,
}

impl MuxTunnel {
    async fn new(work_transport: arp_common::transport::MessageTransport) -> Result<Arc<Self>> {
        let (io, pre_read_data) = work_transport.into_inner_with_read_buf();
        let io: arp_common::transport::BoxedStream = if pre_read_data.is_empty() {
            io
        } else {
            Box::new(PrefixedStream::new(pre_read_data, io))
        };

        let (mut reader, mut writer) = tokio::io::split(io);
        let (frame_tx, mut frame_rx) = mpsc::channel::<MuxFrame>(1024);
        let streams: Arc<Mutex<HashMap<u32, mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let writer_frame_tx = frame_tx.clone();
        tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                if let Err(e) = write_mux_frame(&mut writer, &frame).await {
                    warn!("tcp_mux writer loop exited: {}", e);
                    break;
                }
            }
        });

        let streams_reader = Arc::clone(&streams);
        tokio::spawn(async move {
            loop {
                let frame = match read_mux_frame(&mut reader).await {
                    Ok(frame) => frame,
                    Err(e) => {
                        warn!("tcp_mux reader loop exited: {}", e);
                        break;
                    }
                };

                match frame {
                    MuxFrame::Data { stream_id, payload } => {
                        let tx = streams_reader.lock().await.get(&stream_id).cloned();
                        if let Some(tx) = tx {
                            if tx.send(payload).await.is_err() {
                                streams_reader.lock().await.remove(&stream_id);
                            }
                        } else {
                            let _ = writer_frame_tx.send(MuxFrame::Close { stream_id }).await;
                        }
                    }
                    MuxFrame::Close { stream_id } => {
                        streams_reader.lock().await.remove(&stream_id);
                    }
                    MuxFrame::Ping => {
                        let _ = writer_frame_tx.send(MuxFrame::Pong).await;
                    }
                    MuxFrame::Pong => {}
                    MuxFrame::Open { .. } => {}
                }
            }
            streams_reader.lock().await.clear();
        });

        Ok(Arc::new(Self {
            frame_tx,
            streams,
            next_stream_id: AtomicU32::new(1),
        }))
    }

    async fn open_stream(&self) -> Result<(u32, mpsc::Receiver<Vec<u8>>)> {
        let stream_id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
        self.streams.lock().await.insert(stream_id, tx);
        self.frame_tx
            .send(MuxFrame::Open { stream_id })
            .await
            .map_err(|_| Error::Transport("tcp_mux frame channel closed".to_string()))?;
        Ok((stream_id, rx))
    }

    async fn send_data(&self, stream_id: u32, payload: Vec<u8>) -> Result<()> {
        self.frame_tx
            .send(MuxFrame::Data { stream_id, payload })
            .await
            .map_err(|_| Error::Transport("tcp_mux frame channel closed".to_string()))
    }

    async fn close_stream(&self, stream_id: u32) {
        self.streams.lock().await.remove(&stream_id);
        let _ = self.frame_tx.send(MuxFrame::Close { stream_id }).await;
    }
}

impl TcpProxy {
    pub async fn new(
        msg: NewProxyMsg,
        resource_controller: Arc<ResourceController>,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<Self> {
        let stcp = msg.proxy_type == "stcp";
        let remote_port = if msg.remote_port > 0 {
            resource_controller
                .allocate_tcp_port(msg.remote_port)
                .await?
        } else {
            resource_controller.allocate_random_tcp_port().await?
        };

        let bind_addr = format!("0.0.0.0:{}", remote_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(Error::Io)?;

        info!("TCP proxy {} listening on {}", msg.proxy_name, bind_addr);

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let (close_done_tx, close_done_rx) = watch::channel(false);
        let tcp_mux = matches!(
            msg.multiplexer.as_str(),
            "tcp_mux" | "mux" | "smux" | "yamux"
        ) && !stcp;

        Ok(Self {
            name: msg.proxy_name,
            remote_port,
            listener: Mutex::new(Some(listener)),
            resource_controller,
            work_conn_req_tx,
            shutdown_tx,
            close_done_tx,
            close_done_rx,
            tcp_mux,
            stcp,
            secret: msg.sk,
            mux_tunnel: Arc::new(Mutex::new(None)),
        })
    }

    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }
}

#[async_trait]
impl Proxy for TcpProxy {
    async fn run(&self) -> Result<()> {
        let listener = {
            let mut guard = self.listener.lock().await;
            guard
                .take()
                .ok_or_else(|| Error::Transport(format!("TCP proxy {} listener already taken", self.name)))?
        };
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((client_stream, addr)) => {
                            let client_addr = addr.to_string();
                            let proxy_name = self.name.clone();
                            let remote_port = self.remote_port;
                            let work_conn_req_tx = self.work_conn_req_tx.clone();
                            let tcp_mux = self.tcp_mux;
                            let stcp = self.stcp;
                            let secret = self.secret.clone();
                            let mux_tunnel = Arc::clone(&self.mux_tunnel);

                            tokio::spawn(async move {
                                if tcp_mux {
                                    if let Err(e) = handle_mux_client_connection(
                                        proxy_name.clone(),
                                        remote_port,
                                        client_stream,
                                        client_addr.clone(),
                                        work_conn_req_tx,
                                        mux_tunnel,
                                    ).await {
                                        metrics::inc_tcp_proxy_errors();
                                        error!("TCP mux proxy {} client {} error: {}", proxy_name, client_addr, e);
                                    }
                                } else if let Err(e) = handle_plain_client_connection(
                                    proxy_name.clone(),
                                    remote_port,
                                    client_stream,
                                        client_addr.clone(),
                                        work_conn_req_tx,
                                        stcp,
                                        secret,
                                    ).await {
                                    metrics::inc_tcp_proxy_errors();
                                    error!("TCP proxy {} client {} error: {}", proxy_name, client_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("TCP proxy {} accept error: {}", self.name, e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("TCP proxy {} shutting down", self.name);
                    break;
                }
            }
        }

        let _ = self.close_done_tx.send(true);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        self.mux_tunnel.lock().await.take();

        if !*self.close_done_rx.borrow() {
            let mut close_done_rx = self.close_done_rx.clone();
            let _ = tokio::time::timeout(tokio::time::Duration::from_secs(3), async move {
                while !*close_done_rx.borrow_and_update() {
                    if close_done_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }

        self.resource_controller
            .release_tcp_port(self.remote_port)
            .await?;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn proxy_type(&self) -> &str {
        "tcp"
    }
}

async fn request_work_transport(
    proxy_name: &str,
    remote_port: u16,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
) -> Result<arp_common::transport::MessageTransport> {
    let (work_conn_tx, work_conn_rx) = tokio::sync::oneshot::channel();
    work_conn_req_tx
        .send(WorkConnRequest {
            proxy_name: proxy_name.to_string(),
            reply_tx: work_conn_tx,
        })
        .await
        .map_err(|_| Error::Transport("Failed to request work connection".to_string()))?;

    let mut work_transport =
        tokio::time::timeout(tokio::time::Duration::from_secs(10), work_conn_rx)
            .await
            .map_err(|_| Error::Timeout("Work connection timeout".to_string()))?
            .map_err(|_| Error::Transport("Work connection channel closed".to_string()))?;

    work_transport
        .send(Message::StartWorkConn(StartWorkConnMsg {
            proxy_name: proxy_name.to_string(),
            src_addr: String::new(),
            dst_addr: format!("0.0.0.0:{}", remote_port),
            error: String::new(),
        }))
        .await?;

    Ok(work_transport)
}

#[derive(Clone)]
pub struct GroupedBackend {
    pub backend_id: String,
    pub proxy_name: String,
    pub work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    pub stcp: bool,
    pub secret: String,
}

#[derive(Clone)]
struct BackendSnapshot {
    backend_id: String,
    proxy_name: String,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    stcp: bool,
    secret: String,
}

struct GroupedBackendState {
    snapshot: BackendSnapshot,
    failures: u32,
    ejected_until: Option<std::time::Instant>,
}

pub struct GroupedTcpProxy {
    group_id: String,
    remote_port: u16,
    listener: Mutex<Option<TcpListener>>,
    resource_controller: Arc<ResourceController>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    close_done_tx: watch::Sender<bool>,
    close_done_rx: watch::Receiver<bool>,
    started: AtomicBool,
    rr: AtomicU32,
    backends: Arc<Mutex<Vec<GroupedBackendState>>>,
}

impl GroupedTcpProxy {
    pub async fn new(
        group_id: String,
        remote_port: u16,
        resource_controller: Arc<ResourceController>,
    ) -> Result<Self> {
        let remote_port = resource_controller.allocate_tcp_port(remote_port).await?;
        let bind_addr = format!("0.0.0.0:{}", remote_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(Error::Io)?;
        info!("TCP group {} listening on {}", group_id, bind_addr);
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let (close_done_tx, close_done_rx) = watch::channel(false);
        Ok(Self {
            group_id,
            remote_port,
            listener: Mutex::new(Some(listener)),
            resource_controller,
            shutdown_tx,
            close_done_tx,
            close_done_rx,
            started: AtomicBool::new(false),
            rr: AtomicU32::new(0),
            backends: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn ensure_started(self: &Arc<Self>) {
        if self
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let this = self.clone();
            tokio::spawn(async move {
                if let Err(e) = this.clone().run_loop().await {
                    error!("TCP group {} stopped with error: {}", this.group_id, e);
                }
            });
        }
    }

    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    pub async fn add_backend(&self, backend: GroupedBackend) {
        let snapshot = BackendSnapshot {
            backend_id: backend.backend_id,
            proxy_name: backend.proxy_name,
            work_conn_req_tx: backend.work_conn_req_tx,
            stcp: backend.stcp,
            secret: backend.secret,
        };
        let mut backends = self.backends.lock().await;
        if !backends
            .iter()
            .any(|v| v.snapshot.backend_id == snapshot.backend_id)
        {
            backends.push(GroupedBackendState {
                snapshot,
                failures: 0,
                ejected_until: None,
            });
        }
    }

    pub async fn remove_backend(&self, backend_id: &str) -> Result<bool> {
        let mut backends = self.backends.lock().await;
        backends.retain(|v| v.snapshot.backend_id != backend_id);
        let empty = backends.is_empty();
        drop(backends);
        if empty {
            let _ = self.shutdown_tx.send(());
            if !*self.close_done_rx.borrow() {
                let mut close_done_rx = self.close_done_rx.clone();
                let _ = tokio::time::timeout(tokio::time::Duration::from_secs(3), async move {
                    while !*close_done_rx.borrow_and_update() {
                        if close_done_rx.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .await;
            }
            self.resource_controller
                .release_tcp_port(self.remote_port)
                .await?;
        }
        Ok(empty)
    }

    async fn run_loop(self: Arc<Self>) -> Result<()> {
        let listener = {
            let mut guard = self.listener.lock().await;
            guard.take().ok_or_else(|| {
                Error::Transport(format!("TCP group {} listener already taken", self.group_id))
            })?
        };
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                res = listener.accept() => {
                    let (client_stream, addr) = res.map_err(Error::Io)?;
                    let this = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.handle_client(client_stream, addr.to_string()).await {
                            metrics::inc_tcp_proxy_errors();
                            warn!("tcp group {} client {} failed: {}", this.group_id, addr, e);
                        }
                    });
                }
                _ = shutdown_rx.recv() => {
                    info!("TCP group {} shutting down", self.group_id);
                    break;
                }
            }
        }
        let _ = self.close_done_tx.send(true);
        Ok(())
    }

    async fn handle_client(&self, client_stream: TcpStream, client_addr: String) -> Result<()> {
        let Some(backend) = self.pick_backend().await else {
            return Err(Error::Proxy(format!(
                "tcp group {} has no healthy backend available",
                self.group_id
            )));
        };
        let result = handle_plain_client_connection(
            backend.proxy_name.clone(),
            self.remote_port,
            client_stream,
            client_addr,
            backend.work_conn_req_tx.clone(),
            backend.stcp,
            backend.secret.clone(),
        )
        .await;
        match result {
            Ok(()) => {
                self.mark_backend_success(&backend.backend_id).await;
                Ok(())
            }
            Err(e) => {
                self.mark_backend_failure(&backend.backend_id).await;
                Err(e)
            }
        }
    }

    async fn pick_backend(&self) -> Option<BackendSnapshot> {
        let mut backends = self.backends.lock().await;
        if backends.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        let len = backends.len();
        let start = (self.rr.fetch_add(1, Ordering::Relaxed) as usize) % len;
        for i in 0..len {
            let idx = (start + i) % len;
            let state = &mut backends[idx];
            if state.ejected_until.is_some_and(|deadline| deadline > now) {
                continue;
            }
            state.ejected_until = None;
            return Some(state.snapshot.clone());
        }
        None
    }

    async fn mark_backend_success(&self, backend_id: &str) {
        let mut backends = self.backends.lock().await;
        for state in &mut *backends {
            if state.snapshot.backend_id == backend_id {
                state.failures = 0;
                state.ejected_until = None;
                break;
            }
        }
    }

    async fn mark_backend_failure(&self, backend_id: &str) {
        let mut backends = self.backends.lock().await;
        for state in &mut *backends {
            if state.snapshot.backend_id == backend_id {
                state.failures += 1;
                if state.failures >= BACKEND_FAIL_THRESHOLD {
                    state.ejected_until = Some(
                        std::time::Instant::now()
                            + tokio::time::Duration::from_secs(BACKEND_EJECT_SECS),
                    );
                }
                break;
            }
        }
    }
}

pub struct GroupedMemberProxy {
    name: String,
    grouped: Arc<GroupedTcpProxy>,
    backend_id: String,
    groups: Arc<dashmap::DashMap<String, Arc<GroupedTcpProxy>>>,
}

impl GroupedMemberProxy {
    pub fn new(
        name: String,
        grouped: Arc<GroupedTcpProxy>,
        backend_id: String,
        groups: Arc<dashmap::DashMap<String, Arc<GroupedTcpProxy>>>,
    ) -> Self {
        Self {
            name,
            grouped,
            backend_id,
            groups,
        }
    }
}

#[async_trait]
impl Proxy for GroupedMemberProxy {
    async fn run(&self) -> Result<()> {
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let empty = self.grouped.remove_backend(&self.backend_id).await?;
        if empty {
            self.groups.remove(&self.grouped.group_id);
        }
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn proxy_type(&self) -> &str {
        "tcp"
    }
}

async fn handle_plain_client_connection(
    proxy_name: String,
    remote_port: u16,
    mut client_stream: TcpStream,
    client_addr: String,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    stcp: bool,
    secret: String,
) -> Result<()> {
    metrics::inc_tcp_proxy_connections();
    debug!(
        "TCP proxy {} accepting connection from {}",
        proxy_name, client_addr
    );
    let work_transport = request_work_transport(&proxy_name, remote_port, work_conn_req_tx).await?;
    if stcp {
        let work_stream = work_transport.into_inner();
        relay_stcp(client_stream, work_stream, &secret).await?;
        debug!(
            "STCP proxy {} connection from {} closed",
            proxy_name, client_addr
        );
    } else {
        let mut work_stream = work_transport.into_inner();
        let (client_to_work, work_to_client) =
            copy_bidirectional(&mut client_stream, &mut work_stream).await?;
        metrics::add_tcp_bytes_in(client_to_work);
        metrics::add_tcp_bytes_out(work_to_client);
        debug!(
            "TCP proxy {} connection from {} closed: sent {} bytes, received {} bytes",
            proxy_name, client_addr, client_to_work, work_to_client
        );
    }
    Ok(())
}

async fn handle_mux_client_connection(
    proxy_name: String,
    remote_port: u16,
    client_stream: TcpStream,
    client_addr: String,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    mux_tunnel: Arc<Mutex<Option<Arc<MuxTunnel>>>>,
) -> Result<()> {
    let tunnel =
        get_or_create_mux_tunnel(&proxy_name, remote_port, work_conn_req_tx, mux_tunnel).await?;
    let (stream_id, rx) = tunnel.open_stream().await?;
    metrics::inc_tcp_proxy_connections();
    metrics::inc_tcp_mux_streams();
    debug!(
        "TCP mux proxy {} accepted {} as stream_id={}",
        proxy_name, client_addr, stream_id
    );
    relay_mux_stream(client_stream, stream_id, rx, tunnel).await
}

async fn get_or_create_mux_tunnel(
    proxy_name: &str,
    remote_port: u16,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    mux_tunnel: Arc<Mutex<Option<Arc<MuxTunnel>>>>,
) -> Result<Arc<MuxTunnel>> {
    let mut guard = mux_tunnel.lock().await;
    if let Some(tunnel) = guard.clone() {
        return Ok(tunnel);
    }

    let work_transport = request_work_transport(proxy_name, remote_port, work_conn_req_tx).await?;
    let tunnel = MuxTunnel::new(work_transport).await?;
    *guard = Some(Arc::clone(&tunnel));
    Ok(tunnel)
}

async fn relay_mux_stream(
    client_stream: TcpStream,
    stream_id: u32,
    mut rx: mpsc::Receiver<Vec<u8>>,
    tunnel: Arc<MuxTunnel>,
) -> Result<()> {
    let (mut client_reader, mut client_writer) = client_stream.into_split();
    let writer_task = tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            metrics::add_tcp_bytes_out(data.len() as u64);
            if client_writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = client_reader.read(&mut buf).await.map_err(Error::Io)?;
        if n == 0 {
            break;
        }
        metrics::add_tcp_bytes_in(n as u64);
        tunnel.send_data(stream_id, buf[..n].to_vec()).await?;
    }

    tunnel.close_stream(stream_id).await;
    writer_task.abort();
    Ok(())
}

async fn relay_stcp(
    client_stream: TcpStream,
    work_stream: arp_common::transport::BoxedStream,
    secret: &str,
) -> Result<()> {
    let (mut client_r, mut client_w) = client_stream.into_split();
    let (mut work_r, mut work_w) = tokio::io::split(work_stream);
    let secret_a = secret.to_string();
    let secret_b = secret.to_string();

    let a = tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = client_r.read(&mut buf).await.map_err(Error::Io)?;
            if n == 0 {
                work_w.shutdown().await.map_err(Error::Io)?;
                return Ok::<(), Error>(());
            }
            metrics::add_tcp_bytes_in(n as u64);
            let encrypted = PacketCipher::encrypt(&buf[..n], &secret_a)?;
            write_frame(&mut work_w, &encrypted).await?;
        }
    });

    let b = tokio::spawn(async move {
        loop {
            let Some(frame) = read_frame_optional(&mut work_r).await? else {
                client_w.shutdown().await.map_err(Error::Io)?;
                return Ok::<(), Error>(());
            };
            let plain = PacketCipher::decrypt(&frame, &secret_b)?;
            metrics::add_tcp_bytes_out(plain.len() as u64);
            client_w.write_all(&plain).await.map_err(Error::Io)?;
        }
    });

    let (ra, rb) = tokio::join!(a, b);
    match (ra, rb) {
        (Ok(Ok(())), Ok(Ok(()))) => Ok(()),
        (Ok(Err(e)), _) => Err(e),
        (_, Ok(Err(e))) => Err(e),
        (Err(e), _) => Err(Error::Transport(format!("stcp task join failed: {}", e))),
        (_, Err(e)) => Err(Error::Transport(format!("stcp task join failed: {}", e))),
    }
}

async fn write_frame<W>(writer: &mut W, data: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    writer
        .write_u32(data.len() as u32)
        .await
        .map_err(Error::Io)?;
    writer.write_all(data).await.map_err(Error::Io)?;
    writer.flush().await.map_err(Error::Io)?;
    Ok(())
}

async fn read_frame_optional<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return Err(Error::Protocol("stcp frame too large".to_string()));
    }
    let mut data = vec![0u8; len];
    reader.read_exact(&mut data).await.map_err(Error::Io)?;
    Ok(Some(data))
}
