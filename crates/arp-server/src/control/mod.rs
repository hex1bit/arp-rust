use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use arp_common::auth::Authenticator;
use arp_common::protocol::{
    CloseProxyMsg, Message, NatHoleRespMsg, NatHoleVisitorMsg, NewProxyMsg, NewProxyRespMsg,
    PingMsg, PongMsg, ReqWorkConnMsg,
};
use arp_common::transport::MessageTransport;
use arp_common::{Error, Result};

use crate::nathole::NatHoleController;
use crate::proxy::{ProxyManager, WorkConnRequest};
use crate::resource::ResourceController;

pub struct Control {
    run_id: String,
    peer_addr: String,
    transport: tokio::sync::Mutex<Option<MessageTransport>>,
    proxy_manager: Arc<ProxyManager>,
    resource_controller: Arc<ResourceController>,
    authenticator: Arc<Box<dyn Authenticator>>,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    work_conn_req_rx: tokio::sync::Mutex<mpsc::Receiver<WorkConnRequest>>,
    pending_work_conns: tokio::sync::Mutex<Vec<WorkConnRequest>>,
    idle_work_conns: tokio::sync::Mutex<Vec<MessageTransport>>,
    outbound_tx: mpsc::Sender<Message>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<Message>>,
    nathole: Arc<NatHoleController>,
}

impl Control {
    pub async fn new(
        run_id: String,
        peer_addr: String,
        transport: MessageTransport,
        proxy_manager: Arc<ProxyManager>,
        resource_controller: Arc<ResourceController>,
        authenticator: Arc<Box<dyn Authenticator>>,
        nathole: Arc<NatHoleController>,
    ) -> Result<Self> {
        let (work_conn_req_tx, work_conn_req_rx) = mpsc::channel(100);
        let (outbound_tx, outbound_rx) = mpsc::channel(100);

        Ok(Self {
            run_id,
            peer_addr,
            transport: tokio::sync::Mutex::new(Some(transport)),
            proxy_manager,
            resource_controller,
            authenticator,
            work_conn_req_tx,
            work_conn_req_rx: tokio::sync::Mutex::new(work_conn_req_rx),
            pending_work_conns: tokio::sync::Mutex::new(Vec::new()),
            idle_work_conns: tokio::sync::Mutex::new(Vec::new()),
            outbound_tx,
            outbound_rx: tokio::sync::Mutex::new(outbound_rx),
            nathole,
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut transport = {
            let mut transport_guard = self.transport.lock().await;
            transport_guard
                .take()
                .ok_or_else(|| Error::Transport("Control transport already taken".to_string()))?
        };

        loop {
            tokio::select! {
                msg = transport.recv() => {
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

        if let Some(idle_conn) = {
            let mut idle = self.idle_work_conns.lock().await;
            idle.pop()
        } {
            work_conn_req
                .reply_tx
                .send(idle_conn)
                .map_err(|_| Error::Transport("failed to send idle work connection".to_string()))?;
            return Ok(());
        }

        let req_proxy_name = work_conn_req.proxy_name.clone();
        {
            let mut pending = self.pending_work_conns.lock().await;
            pending.push(work_conn_req);
        }

        if let Err(e) = transport
            .send(Message::ReqWorkConn(ReqWorkConnMsg {
                proxy_name: req_proxy_name,
            }))
            .await
        {
            let mut pending = self.pending_work_conns.lock().await;
            pending.pop();
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
            Ok(remote_addr) => NewProxyRespMsg {
                proxy_name: msg.proxy_name.clone(),
                remote_addr,
                error: String::new(),
            },
            Err(e) => {
                error!("Failed to register proxy {}: {}", msg.proxy_name, e);
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
            pending.remove(0)
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

pub struct ControlManager {
    controls: DashMap<String, Arc<Control>>,
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

    pub fn count(&self) -> usize {
        self.controls.len()
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
