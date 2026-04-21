use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use arp_common::auth::Authenticator;
use arp_common::protocol::{
    CloseProxyMsg, Message, NatHoleRespMsg, NatHoleVisitorMsg, NewProxyMsg, NewProxyRespMsg,
    PingMsg, PongMsg, ReqWorkConnMsg,
};
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::metrics;
use crate::nathole::NatHoleController;
use crate::proxy::{ProxyManager, WorkConnRequest};
use crate::resource::ResourceController;

pub struct Control {
    run_id: String,
    client_id: String,
    peer_addr: String,
    privilege_key: String,
    pool_count: u32,
    hostname: String,
    os: String,
    arch: String,
    user: String,
    transport: tokio::sync::Mutex<Option<MessageTransport>>,
    proxy_manager: Arc<ProxyManager>,
    resource_controller: Arc<ResourceController>,
    authenticator: Arc<Box<dyn Authenticator>>,
    control_manager: Arc<ControlManager>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    work_conn_req_rx: tokio::sync::Mutex<mpsc::Receiver<WorkConnRequest>>,
    pending_work_conns: tokio::sync::Mutex<VecDeque<WorkConnRequest>>,
    idle_work_conns: tokio::sync::Mutex<Vec<MessageTransport>>,
    outbound_tx: mpsc::Sender<Message>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<Message>>,
    nathole: Arc<NatHoleController>,
    heartbeat_timeout: u64,
    cancel: CancellationToken,
}

impl Control {
    pub async fn new(
        run_id: String,
        client_id: String,
        peer_addr: String,
        privilege_key: String,
        pool_count: u32,
        hostname: String,
        os: String,
        arch: String,
        user: String,
        transport: MessageTransport,
        proxy_manager: Arc<ProxyManager>,
        resource_controller: Arc<ResourceController>,
        authenticator: Arc<Box<dyn Authenticator>>,
        control_manager: Arc<ControlManager>,
        nathole: Arc<NatHoleController>,
        heartbeat_timeout: u64,
    ) -> Result<Self> {
        let (work_conn_req_tx, work_conn_req_rx) = mpsc::channel(100);
        let (outbound_tx, outbound_rx) = mpsc::channel(100);

        Ok(Self {
            run_id,
            client_id,
            peer_addr,
            privilege_key,
            pool_count,
            hostname,
            os,
            arch,
            user,
            transport: tokio::sync::Mutex::new(Some(transport)),
            proxy_manager,
            resource_controller,
            authenticator,
            control_manager,
            work_conn_req_tx,
            work_conn_req_rx: tokio::sync::Mutex::new(work_conn_req_rx),
            pending_work_conns: tokio::sync::Mutex::new(VecDeque::new()),
            idle_work_conns: tokio::sync::Mutex::new(Vec::new()),
            outbound_tx,
            outbound_rx: tokio::sync::Mutex::new(outbound_rx),
            nathole,
            heartbeat_timeout,
            cancel: CancellationToken::new(),
        })
    }

    /// Forcefully close this control connection (called when a newer connection takes over).
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    pub fn os(&self) -> &str {
        &self.os
    }

    pub fn arch(&self) -> &str {
        &self.arch
    }

    pub fn pool_count(&self) -> u32 {
        self.pool_count
    }

    pub async fn idle_work_conn_count(&self) -> usize {
        self.idle_work_conns.lock().await.len()
    }

    pub async fn pending_work_conn_count(&self) -> usize {
        self.pending_work_conns.lock().await.len()
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut transport = {
            let mut transport_guard = self.transport.lock().await;
            transport_guard
                .take()
                .ok_or_else(|| Error::Transport("Control transport already taken".to_string()))?
        };

        let heartbeat_timeout_dur = Duration::from_secs(self.heartbeat_timeout);
        let heartbeat_deadline = tokio::time::sleep(heartbeat_timeout_dur);
        tokio::pin!(heartbeat_deadline);

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    warn!("Control connection for run_id {} cancelled (superseded by new connection)", self.run_id);
                    return Err(Error::Transport("connection superseded".to_string()));
                }
                _ = &mut heartbeat_deadline => {
                    error!(
                        "Server heartbeat timeout: no message from client {} for {}s, closing connection",
                        self.run_id, self.heartbeat_timeout
                    );
                    return Err(Error::Timeout("server heartbeat timeout".to_string()));
                }
                msg = transport.recv() => {
                    // Reset heartbeat deadline on any received message
                    heartbeat_deadline.as_mut().reset(tokio::time::Instant::now() + heartbeat_timeout_dur);

                    let msg = match msg? {
                        Some(msg) => msg,
                        None => {
                            info!("Control connection closed for run_id: {}", self.run_id);
                            return Ok(());
                        }
                    };

                    match msg {
                        Message::NewProxy(new_proxy_msg) => {
                            if let Err(e) = self.handle_new_proxy(&mut transport, new_proxy_msg).await {
                                error!("Failed to handle NewProxy: {}", e);
                            }
                        }
                        Message::CloseProxy(close_proxy_msg) => {
                            if let Err(e) = self.handle_close_proxy(close_proxy_msg).await {
                                error!("Failed to handle CloseProxy: {}", e);
                            }
                        }
                        Message::Ping(ping_msg) => {
                            if let Err(e) = self.handle_ping(&mut transport, ping_msg).await {
                                error!("Failed to handle Ping: {}", e);
                            }
                        }
                        Message::NatHoleVisitor(msg) => {
                            if let Err(e) = self.handle_nathole_visitor(&mut transport, msg).await {
                                error!("Failed to handle NatHoleVisitor: {}", e);
                            }
                        }
                        Message::NatHoleResp(msg) => {
                            if let Err(e) = self.handle_nathole_resp(msg).await {
                                error!("Failed to handle NatHoleResp: {}", e);
                            }
                        }
                        _ => {
                            warn!("Unexpected message type: {:?}", msg.type_byte() as char);
                        }
                    }
                }
                work_conn_req = self.recv_work_conn_request() => {
                    match work_conn_req {
                        Some(work_conn_req) => {
                            if let Err(e) = self.handle_work_conn_request(&mut transport, work_conn_req).await {
                                error!("Failed to send ReqWorkConn: {}", e);
                            }
                        }
                        None => {
                            warn!("Work connection request channel closed for run_id: {}", self.run_id);
                        }
                    }
                }
                outbound_msg = self.recv_outbound_message() => {
                    match outbound_msg {
                        Some(msg) => {
                            if let Err(e) = transport.send(msg).await {
                                error!("Failed to send outbound message for run_id {}: {}", self.run_id, e);
                            }
                        }
                        None => {
                            warn!("Outbound message channel closed for run_id: {}", self.run_id);
                        }
                    }
                }
            }
        }
    }

    async fn recv_work_conn_request(&self) -> Option<WorkConnRequest> {
        let mut rx = self.work_conn_req_rx.lock().await;
        rx.recv().await
    }

    async fn recv_outbound_message(&self) -> Option<Message> {
        let mut rx = self.outbound_rx.lock().await;
        rx.recv().await
    }

    async fn handle_work_conn_request(
        &self,
        transport: &mut MessageTransport,
        work_conn_req: WorkConnRequest,
    ) -> Result<()> {
        debug!(
            "Received work connection request for run_id: {}",
            self.run_id
        );
        metrics::inc_req_work_conn();

        if let Some(idle_conn) = {
            let mut idle = self.idle_work_conns.lock().await;
            idle.pop()
        } {
            metrics::inc_idle_work_conn_hits();
            work_conn_req
                .reply_tx
                .send(idle_conn)
                .map_err(|_| Error::Transport("failed to send idle work connection".to_string()))?;
            return Ok(());
        }

        let req_proxy_name = work_conn_req.proxy_name.clone();
        {
            let mut pending = self.pending_work_conns.lock().await;
            pending.push_back(work_conn_req);
        }

        if let Err(e) = transport
            .send(Message::ReqWorkConn(ReqWorkConnMsg {
                proxy_name: req_proxy_name,
            }))
            .await
        {
            let mut pending = self.pending_work_conns.lock().await;
            pending.pop_back();
            return Err(e);
        }

        Ok(())
    }

    async fn handle_new_proxy(
        &self,
        transport: &mut MessageTransport,
        msg: NewProxyMsg,
    ) -> Result<()> {
        info!(
            "Registering new proxy: name={}, type={}",
            msg.proxy_name, msg.proxy_type
        );

        // Only allow takeover when the owner has the same stable client_id.
        if let Some(owner) = self.control_manager.find_proxy_owner(&msg.proxy_name) {
            if owner.run_id != self.run_id {
                if !owner.client_id.is_empty() && owner.client_id == self.client_id {
                    info!(
                        "Taking over proxy {} from stale run_id {} for client_id {}",
                        msg.proxy_name, owner.run_id, owner.client_id
                    );
                    if self.proxy_manager.evict_proxy_by_name(&msg.proxy_name).await.is_some() {
                        self.control_manager.shutdown_run(&owner.run_id);
                    }
                } else {
                    let resp = NewProxyRespMsg {
                        proxy_name: msg.proxy_name.clone(),
                        remote_addr: String::new(),
                        error: format!(
                            "proxy name {} is already in use by another client",
                            msg.proxy_name
                        ),
                    };
                    return transport.send(Message::NewProxyResp(resp)).await;
                }
            }
        }

        let auth_result = self
            .authenticator
            .authorize_proxy(&self.privilege_key, self.pool_count, &msg);
        if let Err(e) = auth_result {
            metrics::inc_auth_failures();
            let resp = NewProxyRespMsg {
                proxy_name: msg.proxy_name.clone(),
                remote_addr: String::new(),
                error: e.to_string(),
            };
            return transport.send(Message::NewProxyResp(resp)).await;
        }

        let result = self
            .proxy_manager
            .register_proxy(
                &self.run_id,
                msg.clone(),
                self.resource_controller.clone(),
                self.work_conn_req_tx.clone(),
            )
            .await;

        let resp = match result {
            Ok(remote_addr) => {
                metrics::inc_proxy_registrations();
                crate::audit::emit(crate::audit::AuditEvent::ProxyRegistered {
                    client_id: self.client_id.clone(),
                    run_id: self.run_id.clone(),
                    proxy_name: msg.proxy_name.clone(),
                    proxy_type: msg.proxy_type.clone(),
                    remote_addr: remote_addr.clone(),
                });
                NewProxyRespMsg {
                    proxy_name: msg.proxy_name.clone(),
                    remote_addr,
                    error: String::new(),
                }
            },
            Err(e) => {
                metrics::inc_proxy_registration_failures();
                error!("Failed to register proxy {}: {}", msg.proxy_name, e);
                crate::audit::emit(crate::audit::AuditEvent::ProxyRejected {
                    client_id: self.client_id.clone(),
                    run_id: self.run_id.clone(),
                    proxy_name: msg.proxy_name.clone(),
                    reason: e.to_string(),
                });
                NewProxyRespMsg {
                    proxy_name: msg.proxy_name.clone(),
                    remote_addr: String::new(),
                    error: e.to_string(),
                }
            }
        };

        transport.send(Message::NewProxyResp(resp)).await
    }

    async fn handle_close_proxy(&self, msg: CloseProxyMsg) -> Result<()> {
        info!("Closing proxy: {}", msg.proxy_name);
        crate::audit::emit(crate::audit::AuditEvent::ProxyClosed {
            run_id: self.run_id.clone(),
            proxy_name: msg.proxy_name.clone(),
        });
        self.proxy_manager
            .unregister_proxy(&self.run_id, &msg.proxy_name)
            .await
    }

    async fn handle_ping(&self, transport: &mut MessageTransport, msg: PingMsg) -> Result<()> {
        debug!("Ping received from run_id: {}", self.run_id);
        if let Err(e) = self.authenticator.verify_ping(&msg) {
            error!("Ping verification failed: {}", e);
            return Err(e);
        }

        transport
            .send(Message::Pong(PongMsg {
                timestamp: chrono::Utc::now().timestamp(),
            }))
            .await
    }

    async fn handle_nathole_visitor(
        &self,
        transport: &mut MessageTransport,
        msg: NatHoleVisitorMsg,
    ) -> Result<()> {
        if let Some(resp) = self.nathole.handle_visitor(&self.run_id, msg).await? {
            transport.send(Message::NatHoleResp(resp)).await?;
        }
        Ok(())
    }

    async fn handle_nathole_resp(&self, msg: NatHoleRespMsg) -> Result<()> {
        self.nathole.handle_client_resp(&self.run_id, msg).await
    }

    pub async fn handle_work_conn(&self, transport: MessageTransport) -> Result<()> {
        debug!("Handling new work connection for run_id: {}", self.run_id);

        let work_conn_req = {
            let mut pending = self.pending_work_conns.lock().await;
            if pending.is_empty() {
                let mut idle = self.idle_work_conns.lock().await;
                idle.push(transport);
                debug!("Stored idle work connection for run_id: {}", self.run_id);
                return Ok(());
            }
            pending.pop_front().unwrap()
        };

        if let Err(_) = work_conn_req.reply_tx.send(transport) {
            error!("Failed to send work connection to proxy");
            return Err(Error::Transport(
                "Failed to send work connection to proxy".to_string(),
            ));
        }

        debug!("Work connection delivered to proxy");
        Ok(())
    }

    pub async fn send_message(&self, msg: Message) -> Result<()> {
        self.outbound_tx
            .send(msg)
            .await
            .map_err(|_| Error::Transport("control outbound channel closed".to_string()))
    }

    pub fn peer_addr(&self) -> &str {
        &self.peer_addr
    }
}

#[derive(Clone, serde::Serialize)]
pub struct ControlRecord {
    pub run_id: String,
    pub client_id: String,
    pub peer_addr: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub user: String,
    pub pool_count: u32,
}

pub struct ControlManager {
    controls: DashMap<String, Arc<Control>>,
}

#[derive(Clone)]
pub struct ProxyOwner {
    pub run_id: String,
    pub client_id: String,
}

impl ControlManager {
    pub fn new() -> Self {
        Self {
            controls: DashMap::new(),
        }
    }

    pub fn add(&self, run_id: String, control: Arc<Control>) {
        self.controls.insert(run_id, control);
    }

    pub fn get(&self, run_id: &str) -> Option<Arc<Control>> {
        self.controls.get(run_id).map(|entry| entry.clone())
    }

    pub fn remove(&self, run_id: &str) {
        self.controls.remove(run_id);
    }

    /// Shutdown and remove a control connection by run_id.
    /// Used to forcefully close a stale connection whose proxy was evicted.
    pub fn shutdown_run(&self, run_id: &str) {
        if let Some((_, control)) = self.controls.remove(run_id) {
            warn!(
                "Shutting down stale control connection for run_id: {}",
                run_id
            );
            control.shutdown();
        }
    }

    pub fn find_proxy_owner(&self, proxy_name: &str) -> Option<ProxyOwner> {
        self.controls.iter().find_map(|entry| {
            let control = entry.value();
            if control.proxy_manager.list_proxy_keys().iter().any(|key| {
                key == &format!("{}:{}", control.run_id(), proxy_name)
            }) {
                Some(ProxyOwner {
                    run_id: control.run_id().to_string(),
                    client_id: control.client_id().to_string(),
                })
            } else {
                None
            }
        })
    }

    pub fn count(&self) -> usize {
        self.controls.len()
    }

    pub fn list_controls(&self) -> Vec<ControlRecord> {
        let mut items: Vec<ControlRecord> = self
            .controls
            .iter()
            .map(|entry| {
                let control = entry.value();
                ControlRecord {
                    run_id: control.run_id().to_string(),
                    client_id: control.client_id().to_string(),
                    peer_addr: control.peer_addr().to_string(),
                    hostname: control.hostname().to_string(),
                    os: control.os().to_string(),
                    arch: control.arch().to_string(),
                    user: control.user().to_string(),
                    pool_count: control.pool_count(),
                }
            })
            .collect();
        items.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        items
    }

    pub fn get_control_record(&self, run_id: &str) -> Option<ControlRecord> {
        self.controls.get(run_id).map(|entry| {
            let control = entry.value();
            ControlRecord {
                run_id: control.run_id().to_string(),
                client_id: control.client_id().to_string(),
                peer_addr: control.peer_addr().to_string(),
                hostname: control.hostname().to_string(),
                os: control.os().to_string(),
                arch: control.arch().to_string(),
                user: control.user().to_string(),
                pool_count: control.pool_count(),
            }
        })
    }

    pub async fn get_control_queue_stats(&self, run_id: &str) -> Option<(usize, usize)> {
        let control = self.controls.get(run_id).map(|entry| entry.clone())?;
        Some((
            control.pending_work_conn_count().await,
            control.idle_work_conn_count().await,
        ))
    }

    pub async fn get_queue_stats_snapshot(&self) -> (usize, usize) {
        let controls: Vec<Arc<Control>> = self.controls.iter().map(|entry| entry.clone()).collect();
        let mut pending = 0usize;
        let mut idle = 0usize;
        for control in controls {
            pending += control.pending_work_conn_count().await;
            idle += control.idle_work_conn_count().await;
        }
        (pending, idle)
    }

    pub async fn send_message(&self, run_id: &str, msg: Message) -> Result<()> {
        let control = self
            .controls
            .get(run_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::Protocol(format!("Unknown run_id: {}", run_id)))?;
        control.send_message(msg).await
    }

    pub fn peer_addr(&self, run_id: &str) -> Option<String> {
        self.controls
            .get(run_id)
            .map(|entry| entry.peer_addr().to_string())
    }
}
