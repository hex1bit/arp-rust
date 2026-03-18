use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::{net::ToSocketAddrs, time::Duration};

use quinn::{ClientConfig as QuicClientConfig, Endpoint, TransportConfig as QuicTransportConfig};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_kcp::{KcpConfig, KcpNoDelayConfig, KcpStream};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::client_async;
use tracing::{debug, error, info, warn};

use arp_common::config::{ClientConfig, VisitorConfig};
use arp_common::protocol::{
    LoginMsg, LoginRespMsg, Message, NatHoleRespMsg, NatHoleVisitorMsg, NewProxyMsg,
    NewProxyRespMsg, PingMsg, PongMsg,
};
use arp_common::transport::quic_stream::QuicBiStream;
use arp_common::transport::ws_stream::websocket_to_stream;
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::proxy::ProxyManager;

/// Pre-built transport configuration to avoid rebuilding TLS context on every work connection.
#[derive(Clone)]
enum CachedTransportConfig {
    /// Plain TCP (no TLS, no WS)
    PlainTcp,
    /// TCP + TLS
    Tls {
        connector: TlsConnector,
        server_name: rustls::pki_types::ServerName<'static>,
    },
    /// WebSocket (plain)
    Ws,
    /// WebSocket + TLS (WSS)
    Wss {
        connector: TlsConnector,
        server_name: rustls::pki_types::ServerName<'static>,
    },
    /// KCP (no TLS context needed)
    Kcp,
    /// QUIC
    Quic(QuicClientConfig),
}

impl CachedTransportConfig {
    fn build(config: &ClientConfig) -> Result<Self> {
        match config.transport.protocol.as_str() {
            "kcp" => Ok(Self::Kcp),
            "quic" => Ok(Self::Quic(build_quic_client_config(config)?)),
            "websocket" => {
                if config.transport.tls.enable {
                    let connector = build_tls_connector(config)?;
                    let server_name = tls_server_name(config)?;
                    Ok(Self::Wss {
                        connector,
                        server_name,
                    })
                } else {
                    Ok(Self::Ws)
                }
            }
            _ => {
                if config.transport.tls.enable {
                    let connector = build_tls_connector(config)?;
                    let server_name = tls_server_name(config)?;
                    Ok(Self::Tls {
                        connector,
                        server_name,
                    })
                } else {
                    Ok(Self::PlainTcp)
                }
            }
        }
    }
}

pub struct Control {
    config: Arc<ClientConfig>,
    transport: tokio::sync::Mutex<MessageTransport>,
    run_id: Arc<tokio::sync::RwLock<String>>,
    proxy_manager: Arc<ProxyManager>,
    cmd_tx: mpsc::Sender<ControlCommand>,
    cmd_rx: tokio::sync::Mutex<mpsc::Receiver<ControlCommand>>,
    pending_xtcp: tokio::sync::Mutex<Option<oneshot::Sender<Result<NatHoleRespMsg>>>>,
    /// Tracks the last time a Pong was received (or when we connected), as Unix timestamp seconds.
    last_pong: Arc<AtomicI64>,
    /// Pre-built transport config (TLS context, etc.) to avoid rebuilding per work connection.
    cached_transport_config: Arc<CachedTransportConfig>,
}

enum ControlCommand {
    OpenXtcp {
        server_name: String,
        sk: String,
        visitor_addr: String,
        reply_tx: oneshot::Sender<Result<NatHoleRespMsg>>,
    },
}

impl Control {
    pub async fn new(config: Arc<ClientConfig>, proxy_manager: Arc<ProxyManager>) -> Result<Self> {
        let server_addr = format!("{}:{}", config.server_addr, config.server_port);
        info!("Connecting to server: {}", server_addr);
        let cached_transport_config = Arc::new(CachedTransportConfig::build(&config)?);
        let transport = connect_server_transport_cached(&config, &cached_transport_config).await?;
        let (cmd_tx, cmd_rx) = mpsc::channel(100);

        Ok(Self {
            config,
            transport: tokio::sync::Mutex::new(transport),
            run_id: Arc::new(tokio::sync::RwLock::new(String::new())),
            proxy_manager,
            cmd_tx,
            cmd_rx: tokio::sync::Mutex::new(cmd_rx),
            pending_xtcp: tokio::sync::Mutex::new(None),
            last_pong: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp())),
            cached_transport_config,
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        self.login().await?;

        self.register_proxies().await?;
        self.start_visitors().await;
        self.prewarm_work_conn_pool().await;

        self.run_message_loop().await?;

        Ok(())
    }

    fn effective_client_id(&self) -> String {
        if !self.config.client_id.trim().is_empty() {
            return self.config.client_id.trim().to_string();
        }

        format!(
            "{}@{}:{}",
            whoami::username(),
            hostname::get()
                .unwrap_or_default()
                .to_string_lossy(),
            self.config.server_addr
        )
    }

    async fn prewarm_work_conn_pool(&self) {
        let pool_count = self.config.transport.pool_count;
        if pool_count == 0 {
            return;
        }

        for _ in 0..pool_count {
            let config = self.config.clone();
            let proxy_manager = self.proxy_manager.clone();
            let run_id = self.run_id.clone();
            let cached = self.cached_transport_config.clone();
            tokio::spawn(async move {
                loop {
                    let current_run_id = run_id.read().await.clone();
                    if current_run_id.trim().is_empty() {
                        warn!("Skip pooled work connection because run_id is empty");
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        continue;
                    }

                    if let Err(e) = Self::create_work_conn_static(
                        config.clone(),
                        cached.clone(),
                        proxy_manager.clone(),
                        current_run_id,
                    )
                    .await
                    {
                        match e {
                            Error::ConnectionClosed => {
                                debug!("Pre-warmed work connection closed, reconnecting...");
                            }
                            _ => {
                                error!("Pooled work connection closed with error: {}", e);
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            });
        }
    }

    async fn login(&self) -> Result<()> {
        info!("Logging in to server...");

        let login_msg = LoginMsg {
            version: env!("CARGO_PKG_VERSION").to_string(),
            hostname: hostname::get()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            user: whoami::username(),
            client_id: self.effective_client_id(),
            timestamp: chrono::Utc::now().timestamp(),
            privilege_key: self.config.auth.token.clone(),
            run_id: String::new(),
            pool_count: self.config.transport.pool_count,
        };

        {
            let mut transport = self.transport.lock().await;
            transport.send(Message::Login(login_msg)).await?;

            match transport.recv().await? {
                Some(Message::LoginResp(LoginRespMsg {
                    version,
                    run_id,
                    error,
                })) => {
                    if !error.is_empty() {
                        error!("Login failed: {}", error);
                        return Err(Error::Auth(error));
                    }
                    info!(
                        "Login successful, run_id: {}, server version: {}",
                        run_id, version
                    );
                    *self.run_id.write().await = run_id;
                }
                Some(msg) => {
                    return Err(Error::Protocol(format!(
                        "Expected LoginResp, got {:?}",
                        msg.type_byte() as char
                    )));
                }
                None => {
                    return Err(Error::ConnectionClosed);
                }
            }
        }

        Ok(())
    }

    async fn register_proxies(&self) -> Result<()> {
        info!("Registering {} proxies...", self.config.proxies.len());

        for proxy_config in &self.config.proxies {
            let has_lb_group = !proxy_config.load_balancer.group.trim().is_empty();
            let multiplexer = if proxy_config.multiplexer.is_empty()
                && self.config.transport.tcp_mux
                && proxy_config.proxy_type == "tcp"
                && !has_lb_group
            {
                "tcp_mux".to_string()
            } else {
                proxy_config.multiplexer.clone()
            };
            let extra = serde_json::json!({
                "load_balancer": {
                    "group": proxy_config.load_balancer.group,
                    "group_key": proxy_config.load_balancer.group_key,
                }
            });
            let msg = NewProxyMsg {
                proxy_name: proxy_config.name.clone(),
                proxy_type: proxy_config.proxy_type.clone(),
                use_encryption: proxy_config.use_encryption,
                use_compression: proxy_config.use_compression,
                local_ip: proxy_config.local_ip.clone(),
                local_port: proxy_config.local_port,
                remote_port: proxy_config.remote_port,
                custom_domains: proxy_config.custom_domains.clone(),
                subdomain: proxy_config.subdomain.clone(),
                locations: proxy_config.locations.clone(),
                host_header_rewrite: proxy_config.host_header_rewrite.clone(),
                sk: proxy_config.sk.clone(),
                multiplexer,
                fallback_to_relay: proxy_config.fallback_to_relay,
                extra,
            };
            let mut effective_proxy_config = proxy_config.clone();
            effective_proxy_config.multiplexer = msg.multiplexer.clone();

            {
                let mut transport = self.transport.lock().await;
                transport.send(Message::NewProxy(msg.clone())).await?;

                match transport.recv().await? {
                    Some(Message::NewProxyResp(NewProxyRespMsg {
                        proxy_name,
                        remote_addr,
                        error,
                    })) => {
                        if !error.is_empty() {
                            error!("Proxy {} registration failed: {}", proxy_name, error);
                            return Err(Error::Proxy(error));
                        }
                        info!(
                            "Proxy {} registered successfully, remote address: {}",
                            proxy_name, remote_addr
                        );
                    }
                    Some(msg) => {
                        return Err(Error::Protocol(format!(
                            "Expected NewProxyResp, got {:?}",
                            msg.type_byte() as char
                        )));
                    }
                    None => {
                        return Err(Error::ConnectionClosed);
                    }
                }
            }

            self.proxy_manager
                .register_proxy(effective_proxy_config)
                .await;
        }

        Ok(())
    }

    async fn start_visitors(self: &Arc<Self>) {
        for visitor in self.config.visitors.clone() {
            if visitor.visitor_type != "xtcp" {
                warn!(
                    "Unsupported visitor type {}, only xtcp is implemented",
                    visitor.visitor_type
                );
                continue;
            }
            let ctrl = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = ctrl.run_xtcp_visitor(visitor).await {
                    error!("xtcp visitor task exited: {}", e);
                }
            });
        }
    }

    async fn run_xtcp_visitor(self: Arc<Self>, visitor: VisitorConfig) -> Result<()> {
        let bind_addr = if visitor.bind_addr.trim().is_empty() {
            "127.0.0.1".to_string()
        } else {
            visitor.bind_addr.clone()
        };
        let bind_port = if visitor.bind_port == 0 {
            return Err(Error::Config(format!(
                "xtcp visitor {} bind_port cannot be 0",
                visitor.name
            )));
        } else {
            visitor.bind_port
        };
        let listener = TcpListener::bind(format!("{}:{}", bind_addr, bind_port))
            .await
            .map_err(Error::Io)?;
        info!(
            "XTCP visitor {} listening on {}:{} (server_name={})",
            visitor.name, bind_addr, bind_port, visitor.server_name
        );

        loop {
            let (mut inbound, _) = listener.accept().await.map_err(Error::Io)?;
            let ctrl = Arc::clone(&self);
            let visitor_cfg = visitor.clone();
            tokio::spawn(async move {
                let result = async {
                    let udp = tokio::net::UdpSocket::bind("0.0.0.0:0")
                        .await
                        .map_err(Error::Io)?;
                    let punch_listener = TcpListener::bind("0.0.0.0:0").await.map_err(Error::Io)?;
                    let visitor_addr = punch_listener.local_addr().map_err(Error::Io)?.to_string();
                    let peer = ctrl
                        .request_xtcp_peer(
                            visitor_cfg.server_name.clone(),
                            visitor_cfg.sk.clone(),
                            visitor_addr.clone(),
                        )
                        .await?;
                    let relay_addr =
                        normalize_relay_addr(&peer.relay_addr, &ctrl.config.server_addr);

                    for _ in 0..5 {
                        let _ = udp.send_to(b"arp-xtcp-punch", &peer.client_addr).await;
                    }

                    let punch_timeout_secs = visitor_cfg.xtcp_punch_timeout_secs.max(3);
                    let mut connect_task = tokio::spawn({
                        let peer_addr = peer.client_addr.clone();
                        async move {
                            tokio::time::timeout(
                                tokio::time::Duration::from_secs(punch_timeout_secs),
                                TcpStream::connect(&peer_addr),
                            )
                            .await
                        }
                    });
                    let mut accept_task = tokio::spawn(async move {
                        tokio::time::timeout(
                            tokio::time::Duration::from_secs(punch_timeout_secs),
                            punch_listener.accept(),
                        )
                        .await
                    });

                    let direct_result: Result<TcpStream> = tokio::select! {
                        conn = &mut connect_task => {
                            accept_task.abort();
                            match conn {
                                Ok(Ok(Ok(stream))) => Ok(stream),
                                Ok(Ok(Err(e))) => Err(Error::Io(e)),
                                Ok(Err(_)) => Err(Error::Timeout("xtcp peer connect timeout".to_string())),
                                Err(e) => Err(Error::Transport(format!("xtcp connect task failed: {}", e))),
                            }
                        }
                        acc = &mut accept_task => {
                            connect_task.abort();
                            match acc {
                                Ok(Ok(Ok((stream, _)))) => Ok(stream),
                                Ok(Ok(Err(e))) => Err(Error::Io(e)),
                                Ok(Err(_)) => Err(Error::Timeout("xtcp peer accept timeout".to_string())),
                                Err(e) => Err(Error::Transport(format!("xtcp accept task failed: {}", e))),
                            }
                        }
                    };

                    let mut peer = match direct_result {
                        Ok(stream) => stream,
                        Err(e) => {
                            if visitor_cfg.fallback_to_relay && !relay_addr.is_empty() {
                                warn!(
                                    "xtcp direct path failed for {} ({}), fallback relay {}",
                                    visitor_cfg.name, e, relay_addr
                                );
                                TcpStream::connect(&relay_addr).await.map_err(|ioe| {
                                    Error::Proxy(format!(
                                        "xtcp direct failed ({}), relay connect {} failed: {}",
                                        e, relay_addr, ioe
                                    ))
                                })?
                            } else if visitor_cfg.fallback_to_relay {
                                return Err(Error::Proxy(format!(
                                    "xtcp direct failed: {}, relay unavailable",
                                    e
                                )));
                            } else {
                                return Err(Error::Proxy(format!(
                                    "xtcp direct failed: {} (fallback disabled)",
                                    e
                                )));
                            }
                        }
                    };

                    arp_common::transport::copy_bidirectional(&mut inbound, &mut peer).await?;
                    Ok::<(), Error>(())
                }
                .await;

                if let Err(e) = result {
                    let _ = inbound
                        .write_all(format!("xtcp error: {}\n", e).as_bytes())
                        .await;
                }
            });
        }
    }

    async fn request_xtcp_peer(
        &self,
        server_name: String,
        sk: String,
        visitor_addr: String,
    ) -> Result<NatHoleRespMsg> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ControlCommand::OpenXtcp {
                server_name,
                sk,
                visitor_addr,
                reply_tx: tx,
            })
            .await
            .map_err(|_| Error::Transport("control command channel closed".to_string()))?;

        match tokio::time::timeout(tokio::time::Duration::from_secs(10), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending_xtcp.lock().await.take();
                Err(Error::Transport("xtcp response channel closed".to_string()))
            }
            Err(_) => {
                self.pending_xtcp.lock().await.take();
                Err(Error::Timeout("xtcp nat hole response timeout".to_string()))
            }
        }
    }

    async fn run_message_loop(&self) -> Result<()> {
        info!("Entering message loop...");

        let heartbeat_interval = if self.config.transport.heartbeat_interval > 0 {
            self.config.transport.heartbeat_interval
        } else {
            30
        };
        let heartbeat_timeout = if self.config.transport.heartbeat_timeout > 0 {
            self.config.transport.heartbeat_timeout
        } else {
            90
        };

        let mut heartbeat_ticker =
            tokio::time::interval(Duration::from_secs(heartbeat_interval));
        heartbeat_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = heartbeat_ticker.tick() => {
                    let now = chrono::Utc::now().timestamp();
                    let last = self.last_pong.load(Ordering::Relaxed);
                    if now - last > heartbeat_timeout as i64 {
                        error!(
                            "Heartbeat timeout: no Pong received for {} seconds, closing connection",
                            now - last
                        );
                        return Err(Error::Timeout("heartbeat timeout".to_string()));
                    }
                    debug!("Sending Ping to server");
                    self.send_control_message(Message::Ping(PingMsg {
                        timestamp: now,
                    }))
                    .await?;
                }
                cmd = self.recv_control_command() => {
                    if let Some(cmd) = cmd {
                        self.handle_control_command(cmd).await?;
                    } else {
                        warn!("control command channel closed");
                    }
                }
                msg = self.recv_control_message() => {
                    let msg = match msg? {
                        Some(msg) => msg,
                        None => {
                            info!("Connection closed by server");
                            return Ok(());
                        }
                    };
                    match msg {
                        Message::ReqWorkConn(req) => {
                            debug!("Received ReqWorkConn for proxy: {}", req.proxy_name);
                            if !req.proxy_name.is_empty()
                                && !self.proxy_manager.is_proxy_available(&req.proxy_name)
                            {
                                warn!("Skip ReqWorkConn for unhealthy proxy {}", req.proxy_name);
                                continue;
                            }
                            let config = self.config.clone();
                            let cached = self.cached_transport_config.clone();
                            let proxy_manager = self.proxy_manager.clone();
                            let run_id = self.run_id.read().await.clone();
                            if run_id.trim().is_empty() {
                                error!("Cannot create work connection: empty run_id");
                                continue;
                            }
                            tokio::spawn(async move {
                                if let Err(e) =
                                    Self::create_work_conn_static(config, cached, proxy_manager, run_id).await
                                {
                                    match e {
                                        Error::ConnectionClosed => {
                                            debug!("Work connection closed before StartWorkConn (pre-warmed connection may have been used instead)");
                                        }
                                        _ => {
                                            error!("Failed to create work connection: {}", e);
                                        }
                                    }
                                }
                            });
                        }
                        Message::Ping(_) => {
                            debug!("Received Ping");
                            self.send_control_message(Message::Pong(PongMsg {
                                timestamp: chrono::Utc::now().timestamp(),
                            }))
                            .await?;
                        }
                        Message::Pong(pong) => {
                            debug!("Received Pong (server_ts={})", pong.timestamp);
                            self.last_pong
                                .store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
                        }
                        Message::NatHoleClient(req) => {
                            debug!(
                                "Received NatHoleClient: proxy={}, visitor_addr={}",
                                req.proxy_name, req.visitor_addr
                            );
                            let resp = match self
                                .proxy_manager
                                .handle_nat_hole_client(&req.proxy_name, &req.visitor_addr)
                                .await
                            {
                                Ok(client_addr) => NatHoleRespMsg {
                                    visitor_addr: req.visitor_addr,
                                    client_addr,
                                    relay_addr: String::new(),
                                    error: String::new(),
                                },
                                Err(e) => NatHoleRespMsg {
                                    visitor_addr: req.visitor_addr,
                                    client_addr: String::new(),
                                    relay_addr: String::new(),
                                    error: e.to_string(),
                                },
                            };
                            self.send_control_message(Message::NatHoleResp(resp)).await?;
                        }
                        Message::NatHoleResp(resp) => {
                            self.handle_nathole_response(resp).await;
                        }
                        _ => {
                            warn!("Unexpected message type: {:?}", msg.type_byte() as char);
                        }
                    }
                }
            }
        }
    }

    async fn recv_control_message(&self) -> Result<Option<Message>> {
        let mut transport = self.transport.lock().await;
        transport.recv().await
    }

    async fn send_control_message(&self, msg: Message) -> Result<()> {
        let mut transport = self.transport.lock().await;
        transport.send(msg).await
    }

    async fn recv_control_command(&self) -> Option<ControlCommand> {
        let mut rx = self.cmd_rx.lock().await;
        rx.recv().await
    }

    async fn handle_control_command(&self, cmd: ControlCommand) -> Result<()> {
        match cmd {
            ControlCommand::OpenXtcp {
                server_name,
                sk,
                visitor_addr,
                reply_tx,
            } => {
                {
                    let mut pending = self.pending_xtcp.lock().await;
                    if pending.is_some() {
                        let _ = reply_tx.send(Err(Error::Proxy(
                            "xtcp visitor request already in progress".to_string(),
                        )));
                        return Ok(());
                    }
                    *pending = Some(reply_tx);
                }

                let signed_msg = format!("{}|{}", sk, visitor_addr);
                if let Err(e) = self
                    .send_control_message(Message::NatHoleVisitor(NatHoleVisitorMsg {
                        proxy_name: server_name,
                        signed_msg,
                    }))
                    .await
                {
                    self.pending_xtcp.lock().await.take();
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    async fn handle_nathole_response(&self, resp: NatHoleRespMsg) {
        let tx = { self.pending_xtcp.lock().await.take() };
        if let Some(tx) = tx {
            if resp.error.is_empty() {
                let _ = tx.send(Ok(resp));
            } else {
                let _ = tx.send(Err(Error::Proxy(resp.error)));
            }
        } else {
            warn!("received unexpected NatHoleResp without pending request");
        }
    }

    async fn create_work_conn_static(
        config: Arc<ClientConfig>,
        cached: Arc<CachedTransportConfig>,
        proxy_manager: Arc<ProxyManager>,
        run_id: String,
    ) -> Result<()> {
        if run_id.trim().is_empty() {
            return Err(Error::Protocol(
                "refuse to create work connection with empty run_id".to_string(),
            ));
        }

        debug!("Creating work connection...");

        let server_addr = format!("{}:{}", config.server_addr, config.server_port);
        let mut transport = connect_server_transport_cached(&config, &cached).await.map_err(|e| {
            error!("Failed to connect to server for work conn: {}", e);
            e
        })?;

        debug!("Work connection established to {}", server_addr);

        transport
            .send(Message::NewWorkConn(arp_common::protocol::NewWorkConnMsg {
                run_id,
                privilege_key: config.auth.token.clone(),
            }))
            .await?;

        debug!("Sent NewWorkConn message");

        match transport.recv().await? {
            Some(Message::StartWorkConn(start_msg)) => {
                if !start_msg.error.is_empty() {
                    error!("StartWorkConn error: {}", start_msg.error);
                    return Err(Error::Proxy(start_msg.error));
                }

                debug!(
                    "Work connection started for proxy: {}",
                    start_msg.proxy_name
                );

                let proxy = proxy_manager
                    .get_proxy(&start_msg.proxy_name)
                    .ok_or_else(|| {
                        error!("Unknown proxy: {}", start_msg.proxy_name);
                        Error::Proxy(format!("Unknown proxy: {}", start_msg.proxy_name))
                    })?;

                debug!("Got proxy handler, starting data transfer...");
                proxy.handle_work_conn(transport).await?;
                debug!("Work connection completed");
            }
            Some(msg) => {
                error!("Expected StartWorkConn, got {:?}", msg.type_byte() as char);
                return Err(Error::Protocol(format!(
                    "Expected StartWorkConn, got {:?}",
                    msg.type_byte() as char
                )));
            }
            None => {
                debug!("Connection closed while waiting for StartWorkConn");
                return Err(Error::ConnectionClosed);
            }
        }

        Ok(())
    }
}

async fn connect_server_transport_cached(
    config: &ClientConfig,
    cached: &CachedTransportConfig,
) -> Result<MessageTransport> {
    match cached {
        CachedTransportConfig::Kcp => {
            let server_addr = resolve_socket_addr(&config.server_addr, config.server_port)?;
            let stream = KcpStream::connect(&build_kcp_config(), server_addr)
                .await
                .map_err(|e| Error::Transport(format!("KCP connect failed: {}", e)))?;
            Ok(MessageTransport::from_stream(Box::new(stream)))
        }
        CachedTransportConfig::Quic(quic_config) => {
            let server_addr = resolve_socket_addr(&config.server_addr, config.server_port)?;
            let bind_addr = match server_addr {
                std::net::SocketAddr::V4(_) => "0.0.0.0:0",
                std::net::SocketAddr::V6(_) => "[::]:0",
            };
            let mut endpoint = Endpoint::client(bind_addr.parse().map_err(|e| {
                Error::Config(format!(
                    "Invalid QUIC client bind address {}: {}",
                    bind_addr, e
                ))
            })?)
            .map_err(Error::Io)?;
            endpoint.set_default_client_config(quic_config.clone());
            let server_name = if !config.transport.tls.server_name.is_empty() {
                config.transport.tls.server_name.clone()
            } else {
                config.server_addr.clone()
            };
            let conn = endpoint
                .connect(server_addr, &server_name)
                .map_err(|e| Error::Transport(format!("QUIC connect init failed: {}", e)))?
                .await
                .map_err(|e| Error::Transport(format!("QUIC connect failed: {}", e)))?;
            let (send, recv) = conn
                .open_bi()
                .await
                .map_err(|e| Error::Transport(format!("QUIC stream open failed: {}", e)))?;
            Ok(MessageTransport::from_stream(Box::new(QuicBiStream::new(
                recv,
                send,
                conn.clone(),
                Some(endpoint),
            ))))
        }
        CachedTransportConfig::Ws => {
            let ws_url = format!(
                "ws://{}:{}/ws",
                config.server_addr, config.server_port
            );
            let stream =
                TcpStream::connect(format!("{}:{}", config.server_addr, config.server_port))
                    .await
                    .map_err(Error::Io)?;
            let (ws, _) = client_async(&ws_url, stream)
                .await
                .map_err(|e| Error::Transport(format!("WS handshake failed: {}", e)))?;
            Ok(MessageTransport::from_stream(websocket_to_stream(ws)))
        }
        CachedTransportConfig::Wss {
            connector,
            server_name,
        } => {
            let ws_url = format!(
                "wss://{}:{}/ws",
                config.server_addr, config.server_port
            );
            let stream =
                TcpStream::connect(format!("{}:{}", config.server_addr, config.server_port))
                    .await
                    .map_err(Error::Io)?;
            let tls_stream = connector
                .connect(server_name.clone(), stream)
                .await
                .map_err(|e| Error::Transport(format!("WSS TLS connect failed: {}", e)))?;
            let (ws, _) = client_async(&ws_url, tls_stream)
                .await
                .map_err(|e| Error::Transport(format!("WSS handshake failed: {}", e)))?;
            Ok(MessageTransport::from_stream(websocket_to_stream(ws)))
        }
        CachedTransportConfig::Tls {
            connector,
            server_name,
        } => {
            let server_addr = format!("{}:{}", config.server_addr, config.server_port);
            let stream = TcpStream::connect(&server_addr).await.map_err(Error::Io)?;
            let tls_stream = connector
                .connect(server_name.clone(), stream)
                .await
                .map_err(|e| Error::Transport(format!("TLS connect failed: {}", e)))?;
            Ok(MessageTransport::from_stream(Box::new(tls_stream)))
        }
        CachedTransportConfig::PlainTcp => {
            let server_addr = format!("{}:{}", config.server_addr, config.server_port);
            let stream = TcpStream::connect(&server_addr).await.map_err(Error::Io)?;
            Ok(MessageTransport::new(stream))
        }
    }
}

fn normalize_relay_addr(relay_addr: &str, server_addr: &str) -> String {
    let Ok(sa) = relay_addr.parse::<std::net::SocketAddr>() else {
        return relay_addr.to_string();
    };
    if !sa.ip().is_unspecified() && !sa.ip().is_loopback() {
        return relay_addr.to_string();
    }
    format!("{}:{}", server_addr, sa.port())
}

fn build_kcp_config() -> KcpConfig {
    let mut config = KcpConfig::default();
    config.stream = true;
    config.nodelay = KcpNoDelayConfig::fastest();
    config.flush_write = true;
    config.flush_acks_input = true;
    config.session_expire = Duration::from_secs(120);
    config
}

fn build_quic_client_config(config: &ClientConfig) -> Result<QuicClientConfig> {
    if config.transport.tls.trusted_ca_file.is_empty() {
        return Err(Error::Config(
            "quic transport requires transport.tls.trusted_ca_file".to_string(),
        ));
    }

    let ca_file = std::fs::File::open(&config.transport.tls.trusted_ca_file)
        .map_err(|e| Error::Config(format!("Failed to open trusted CA file: {}", e)))?;
    let mut ca_reader = std::io::BufReader::new(ca_file);

    let mut roots = quinn::rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut ca_reader) {
        let cert = cert.map_err(|e| Error::Config(format!("Failed to parse CA cert: {}", e)))?;
        roots
            .add(cert)
            .map_err(|e| Error::Config(format!("Failed to add CA cert: {}", e)))?;
    }

    let mut client_config = QuicClientConfig::with_root_certificates(Arc::new(roots))
        .map_err(|e| Error::Config(format!("Failed to build QUIC client config: {}", e)))?;
    let mut transport = QuicTransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(60)
            .try_into()
            .map_err(|e| Error::Config(format!("Invalid QUIC idle timeout: {}", e)))?,
    ));
    client_config.transport_config(Arc::new(transport));
    Ok(client_config)
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<std::net::SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| Error::Config(format!("Failed to resolve {}:{}: {}", host, port, e)))?
        .next()
        .ok_or_else(|| Error::Config(format!("No socket address resolved for {}:{}", host, port)))
}

fn build_tls_connector(config: &ClientConfig) -> Result<TlsConnector> {
    if config.transport.tls.trusted_ca_file.is_empty() {
        return Err(Error::Config(
            "tls.enable is true but trusted_ca_file is empty".to_string(),
        ));
    }

    let ca_file = std::fs::File::open(&config.transport.tls.trusted_ca_file)
        .map_err(|e| Error::Config(format!("Failed to open trusted CA file: {}", e)))?;
    let mut ca_reader = std::io::BufReader::new(ca_file);

    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut ca_reader) {
        let cert = cert.map_err(|e| Error::Config(format!("Failed to parse CA cert: {}", e)))?;
        roots
            .add(cert)
            .map_err(|e| Error::Config(format!("Failed to add CA cert: {}", e)))?;
    }

    let client_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(client_config)))
}

fn tls_server_name(config: &ClientConfig) -> Result<rustls::pki_types::ServerName<'static>> {
    let server_name = if !config.transport.tls.server_name.is_empty() {
        config.transport.tls.server_name.clone()
    } else {
        config.server_addr.clone()
    };
    rustls::pki_types::ServerName::try_from(server_name.clone())
        .map(|name| name.to_owned())
        .map_err(|e| Error::Config(format!("Invalid TLS server_name {}: {}", server_name, e)))
}
