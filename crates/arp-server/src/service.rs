use std::sync::Arc;
use std::{net::ToSocketAddrs, time::Duration};

use quinn::{Endpoint, ServerConfig as QuicServerConfig, TransportConfig as QuicTransportConfig};
use tokio::net::{TcpListener, TcpStream};
use tokio_kcp::{KcpConfig, KcpListener, KcpNoDelayConfig};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_async;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use arp_common::auth::{create_authenticator, Authenticator};
use arp_common::config::ServerConfig;
use arp_common::protocol::{LoginMsg, LoginRespMsg, Message};
use arp_common::transport::quic_stream::QuicBiStream;
use arp_common::transport::ws_stream::websocket_to_stream;
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::control::{Control, ControlManager};
use crate::metrics;
use crate::nathole::NatHoleController;
use crate::proxy::ProxyManager;
use crate::resource::ResourceController;
use crate::web::{start_admin_server, AdminState};

#[derive(Clone)]
pub struct Service {
    config: Arc<ServerConfig>,
    control_manager: Arc<ControlManager>,
    proxy_manager: Arc<ProxyManager>,
    resource_controller: Arc<ResourceController>,
    authenticator: Arc<Box<dyn Authenticator>>,
    tls_acceptor: Option<Arc<TlsAcceptor>>,
    nathole: Arc<NatHoleController>,
}

impl Service {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let authenticator = create_authenticator(&config.auth);
        let control_manager = Arc::new(ControlManager::new());
        let proxy_manager = Arc::new(ProxyManager::new(
            config.bind_addr.clone(),
            config.subdomain_host.clone(),
            config.vhost_http_port,
            config.vhost_https_port,
        ));
        let resource_controller = Arc::new(ResourceController::new(&config));
        let tls_acceptor = if config.transport.tls.enable {
            Some(Arc::new(build_tls_acceptor(&config)?))
        } else {
            None
        };
        let nathole = Arc::new(NatHoleController::new(
            control_manager.clone(),
            proxy_manager.clone(),
        ));

        Ok(Self {
            config: Arc::new(config),
            control_manager,
            proxy_manager,
            resource_controller,
            authenticator: Arc::new(authenticator),
            tls_acceptor,
            nathole,
        })
    }

    pub async fn run(self) -> Result<()> {
        if self.config.dashboard_port > 0 {
            let dashboard_addr = if self.config.dashboard_addr.is_empty() {
                self.config.bind_addr.clone()
            } else {
                self.config.dashboard_addr.clone()
            };
            start_admin_server(
                dashboard_addr,
                self.config.dashboard_port,
                AdminState {
                    control_manager: self.control_manager.clone(),
                    proxy_manager: self.proxy_manager.clone(),
                    started_at_unix: chrono::Utc::now().timestamp(),
                },
            );
        }

        match self.config.transport.protocol.as_str() {
            "kcp" => self.run_kcp().await,
            "quic" => self.run_quic().await,
            _ => self.run_tcp_like().await,
        }
    }

    async fn run_tcp_like(&self) -> Result<()> {
        let bind_addr = format!("{}:{}", self.config.bind_addr, self.config.bind_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(Error::Io)?;

        info!(
            "Server listening on {} via {}",
            bind_addr, self.config.transport.protocol
        );

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    metrics::inc_incoming_connections();
                    debug!("New connection from {}", peer_addr);
                    let service = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = service.handle_connection(stream).await {
                            error!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn run_kcp(&self) -> Result<()> {
        let bind_port = if self.config.kcp_bind_port > 0 {
            self.config.kcp_bind_port
        } else {
            self.config.bind_port
        };
        let bind_addr = format!("{}:{}", self.config.bind_addr, bind_port);
        let mut listener = KcpListener::bind(build_kcp_config(), &bind_addr)
            .await
            .map_err(|e| Error::Transport(format!("KCP bind failed on {}: {}", bind_addr, e)))?;

        info!("Server listening on {} via kcp", bind_addr);

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    metrics::inc_incoming_connections();
                    let service = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = service
                            .handle_transport(
                                MessageTransport::from_stream(Box::new(stream)),
                                peer_addr.to_string(),
                            )
                            .await
                        {
                            error!("KCP connection error from {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept KCP connection: {}", e);
                }
            }
        }
    }

    async fn run_quic(&self) -> Result<()> {
        let bind_port = if self.config.quic_bind_port > 0 {
            self.config.quic_bind_port
        } else {
            self.config.bind_port
        };
        let bind_addr = resolve_socket_addr(&self.config.bind_addr, bind_port)?;
        let endpoint = Endpoint::server(build_quic_server_config(&self.config)?, bind_addr)
            .map_err(Error::Io)?;

        info!("Server listening on {} via quic", bind_addr);

        loop {
            let Some(incoming) = endpoint.accept().await else {
                return Err(Error::Transport("QUIC endpoint closed".to_string()));
            };
            let service = self.clone();
            let endpoint_clone = endpoint.clone();
            tokio::spawn(async move {
                let conn = match incoming.await {
                    Ok(conn) => conn,
                    Err(e) => {
                        error!("QUIC handshake failed: {}", e);
                        return;
                    }
                };
                let peer_addr = conn.remote_address();
                metrics::inc_incoming_connections();
                let (send, recv) = match conn.accept_bi().await {
                    Ok(streams) => streams,
                    Err(e) => {
                        error!("QUIC stream accept failed from {}: {}", peer_addr, e);
                        return;
                    }
                };
                let transport = MessageTransport::from_stream(Box::new(QuicBiStream::new(
                    recv,
                    send,
                    conn.clone(),
                    Some(endpoint_clone),
                )));
                if let Err(e) = service
                    .handle_transport(transport, peer_addr.to_string())
                    .await
                {
                    error!("QUIC connection error from {}: {}", peer_addr, e);
                }
            });
        }
    }

    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let peer_addr = stream.peer_addr().map_err(|e| Error::Io(e))?;
        let transport = if self.config.transport.protocol == "websocket" {
            if let Some(acceptor) = &self.tls_acceptor {
                let tls_stream = acceptor
                    .accept(stream)
                    .await
                    .map_err(|e| Error::Transport(format!("WSS TLS accept failed: {}", e)))?;
                let ws = accept_async(tls_stream)
                    .await
                    .map_err(|e| Error::Transport(format!("WSS accept failed: {}", e)))?;
                MessageTransport::from_stream(websocket_to_stream(ws))
            } else {
                let ws = accept_async(stream)
                    .await
                    .map_err(|e| Error::Transport(format!("WS accept failed: {}", e)))?;
                MessageTransport::from_stream(websocket_to_stream(ws))
            }
        } else if let Some(acceptor) = &self.tls_acceptor {
            let tls_stream = acceptor
                .accept(stream)
                .await
                .map_err(|e| Error::Transport(format!("TLS accept failed: {}", e)))?;
            MessageTransport::from_stream(Box::new(tls_stream))
        } else {
            MessageTransport::new(stream)
        };

        self.handle_transport(transport, peer_addr.to_string())
            .await
    }

    async fn handle_transport(
        &self,
        mut transport: MessageTransport,
        peer_addr: String,
    ) -> Result<()> {
        let msg = match transport.recv().await? {
            Some(msg) => msg,
            None => {
                warn!("Connection closed without message from {}", peer_addr);
                return Ok(());
            }
        };

        match msg {
            Message::Login(login_msg) => {
                self.handle_login(transport, login_msg, peer_addr.to_string())
                    .await
            }
            Message::NewWorkConn(work_conn_msg) => {
                self.handle_new_work_conn(transport, work_conn_msg).await
            }
            _ => {
                warn!("Unexpected message type from {}: {:?}", peer_addr, msg);
                Err(Error::Protocol(format!(
                    "Expected Login or NewWorkConn message, got {:?}",
                    msg.type_byte() as char
                )))
            }
        }
    }

    async fn handle_login(
        &self,
        mut transport: MessageTransport,
        login_msg: LoginMsg,
        peer_addr: String,
    ) -> Result<()> {
        info!(
            "Client login from {}: version={}, hostname={}, os={}, arch={}",
            peer_addr, login_msg.version, login_msg.hostname, login_msg.os, login_msg.arch
        );

        if let Err(e) = self.authenticator.verify_login(&login_msg) {
            error!("Authentication failed for {}: {}", peer_addr, e);
            transport
                .send(Message::LoginResp(LoginRespMsg {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    run_id: String::new(),
                    error: e.to_string(),
                }))
                .await?;
            return Err(e);
        }

        let run_id = Uuid::new_v4().to_string();
        info!("Generated run_id: {} for {}", run_id, peer_addr);

        transport
            .send(Message::LoginResp(LoginRespMsg {
                version: env!("CARGO_PKG_VERSION").to_string(),
                run_id: run_id.clone(),
                error: String::new(),
            }))
            .await?;

        let control = Arc::new(
            Control::new(
                run_id.clone(),
                peer_addr.clone(),
                transport,
                self.proxy_manager.clone(),
                self.resource_controller.clone(),
                self.authenticator.clone(),
                self.nathole.clone(),
            )
            .await?,
        );

        self.control_manager.add(run_id.clone(), control.clone());

        let result = control.run().await;

        if let Err(e) = self.proxy_manager.unregister_run_proxies(&run_id).await {
            error!("Failed to unregister proxies for run_id {}: {}", run_id, e);
        }
        self.control_manager.remove(&run_id);
        info!("Control connection closed for run_id: {}", run_id);

        result
    }

    async fn handle_new_work_conn(
        &self,
        transport: MessageTransport,
        work_conn_msg: arp_common::protocol::NewWorkConnMsg,
    ) -> Result<()> {
        debug!("NewWorkConn from run_id: {}", work_conn_msg.run_id);
        if work_conn_msg.run_id.trim().is_empty() {
            return Err(Error::Protocol("Empty run_id in NewWorkConn".to_string()));
        }
        metrics::inc_work_connections();

        if let Err(e) = self.authenticator.verify_new_work_conn(&work_conn_msg) {
            error!(
                "Work connection authentication failed for run_id {}: {}",
                work_conn_msg.run_id, e
            );
            return Err(e);
        }

        let control = self
            .control_manager
            .get(&work_conn_msg.run_id)
            .ok_or_else(|| Error::Protocol(format!("Unknown run_id: {}", work_conn_msg.run_id)))?;

        control.handle_work_conn(transport).await
    }
}

fn build_tls_acceptor(config: &ServerConfig) -> Result<TlsAcceptor> {
    if config.transport.tls.cert_file.is_empty() || config.transport.tls.key_file.is_empty() {
        return Err(Error::Config(
            "tls.enable is true but cert_file/key_file is empty".to_string(),
        ));
    }

    let cert_file = std::fs::File::open(&config.transport.tls.cert_file)
        .map_err(|e| Error::Config(format!("Failed to open cert file: {}", e)))?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Config(format!("Failed to parse certs: {}", e)))?;
    if certs.is_empty() {
        return Err(Error::Config("No certificates found".to_string()));
    }

    let key_file = std::fs::File::open(&config.transport.tls.key_file)
        .map_err(|e| Error::Config(format!("Failed to open key file: {}", e)))?;
    let mut key_reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| Error::Config(format!("Failed to parse private key: {}", e)))?
        .ok_or_else(|| Error::Config("No private key found".to_string()))?;

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| Error::Config(format!("Invalid TLS cert/key: {}", e)))?;

    Ok(TlsAcceptor::from(Arc::new(server_config)))
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

fn build_quic_server_config(config: &ServerConfig) -> Result<QuicServerConfig> {
    if config.transport.tls.cert_file.is_empty() || config.transport.tls.key_file.is_empty() {
        return Err(Error::Config(
            "quic transport requires tls.cert_file and tls.key_file".to_string(),
        ));
    }

    let cert_file = std::fs::File::open(&config.transport.tls.cert_file)
        .map_err(|e| Error::Config(format!("Failed to open cert file: {}", e)))?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Config(format!("Failed to parse certs: {}", e)))?;

    let key_file = std::fs::File::open(&config.transport.tls.key_file)
        .map_err(|e| Error::Config(format!("Failed to open key file: {}", e)))?;
    let mut key_reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| Error::Config(format!("Failed to parse private key: {}", e)))?
        .ok_or_else(|| Error::Config("No private key found".to_string()))?;

    let mut server_config = QuicServerConfig::with_single_cert(certs, key)
        .map_err(|e| Error::Config(format!("Invalid QUIC cert/key: {}", e)))?;
    let mut transport = QuicTransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(60)
            .try_into()
            .map_err(|e| Error::Config(format!("Invalid QUIC idle timeout: {}", e)))?,
    ));
    server_config.transport = Arc::new(transport);
    Ok(server_config)
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<std::net::SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| Error::Config(format!("Failed to resolve {}:{}: {}", host, port, e)))?
        .next()
        .ok_or_else(|| Error::Config(format!("No socket address resolved for {}:{}", host, port)))
}
