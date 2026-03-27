use std::collections::VecDeque;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use arp_common::protocol::{Message, NatHoleClientMsg, NatHoleRespMsg, NatHoleVisitorMsg};
use arp_common::{Error, Result};

use crate::control::ControlManager;
use crate::metrics;
use crate::proxy::ProxyManager;

struct PendingNatVisitor {
    visitor_run_id: String,
    proxy_name: String,
    relay_addr: String,
}

#[derive(Clone, serde::Serialize)]
pub struct XtcpEvent {
    pub stage: String,
    pub proxy_name: String,
    pub visitor_run_id: String,
    pub owner_run_id: String,
    pub visitor_addr: String,
    pub client_addr: String,
    pub relay_addr: String,
    pub error: String,
}

pub struct NatHoleController {
    control_manager: Arc<ControlManager>,
    proxy_manager: Arc<ProxyManager>,
    pending: DashMap<String, PendingNatVisitor>,
    recent_events: Mutex<VecDeque<XtcpEvent>>,
}

impl NatHoleController {
    pub fn new(control_manager: Arc<ControlManager>, proxy_manager: Arc<ProxyManager>) -> Self {
        Self {
            control_manager,
            proxy_manager,
            pending: DashMap::new(),
            recent_events: Mutex::new(VecDeque::with_capacity(32)),
        }
    }

    pub async fn handle_visitor(
        &self,
        visitor_run_id: &str,
        msg: NatHoleVisitorMsg,
    ) -> Result<Option<NatHoleRespMsg>> {
        metrics::inc_xtcp_visitor_requests();
        let (signed_sk, mut visitor_addr) = parse_signed_msg(&msg.signed_msg)?;
        if let Some(visitor_peer) = self.control_manager.peer_addr(visitor_run_id) {
            visitor_addr = normalize_endpoint_addr(&visitor_addr, &visitor_peer);
        }
        let Some(owner) = self.proxy_manager.get_xtcp_owner(&msg.proxy_name) else {
            metrics::inc_xtcp_owner_not_found();
            self.push_event(XtcpEvent {
                stage: "owner_not_found".to_string(),
                proxy_name: msg.proxy_name.clone(),
                visitor_run_id: visitor_run_id.to_string(),
                owner_run_id: String::new(),
                visitor_addr: visitor_addr.clone(),
                client_addr: String::new(),
                relay_addr: String::new(),
                error: format!("xtcp proxy {} not found", msg.proxy_name),
            })
            .await;
            return Ok(Some(NatHoleRespMsg {
                visitor_addr,
                client_addr: String::new(),
                relay_addr: String::new(),
                error: format!("xtcp proxy {} not found", msg.proxy_name),
            }));
        };

        if owner.sk != signed_sk {
            metrics::inc_xtcp_sk_mismatch();
            self.push_event(XtcpEvent {
                stage: "sk_mismatch".to_string(),
                proxy_name: msg.proxy_name.clone(),
                visitor_run_id: visitor_run_id.to_string(),
                owner_run_id: owner.run_id.clone(),
                visitor_addr: visitor_addr.clone(),
                client_addr: String::new(),
                relay_addr: owner.relay_addr.clone(),
                error: "xtcp sk mismatch".to_string(),
            })
            .await;
            return Ok(Some(NatHoleRespMsg {
                visitor_addr,
                client_addr: String::new(),
                relay_addr: String::new(),
                error: "xtcp sk mismatch".to_string(),
            }));
        }

        self.pending.insert(
            visitor_addr.clone(),
            PendingNatVisitor {
                visitor_run_id: visitor_run_id.to_string(),
                proxy_name: msg.proxy_name.clone(),
                relay_addr: owner.relay_addr.clone(),
            },
        );

        let req = NatHoleClientMsg {
            proxy_name: msg.proxy_name.clone(),
            visitor_addr: visitor_addr.clone(),
        };

        if let Err(e) = self
            .control_manager
            .send_message(&owner.run_id, Message::NatHoleClient(req))
            .await
        {
            metrics::inc_xtcp_owner_offline();
            self.push_event(XtcpEvent {
                stage: "owner_offline".to_string(),
                proxy_name: msg.proxy_name.clone(),
                visitor_run_id: visitor_run_id.to_string(),
                owner_run_id: owner.run_id.clone(),
                visitor_addr: visitor_addr.clone(),
                client_addr: String::new(),
                relay_addr: owner.relay_addr.clone(),
                error: format!("owner client offline: {}", e),
            })
            .await;
            warn!("failed to forward NatHoleClient to owner: {}", e);
            return Ok(Some(NatHoleRespMsg {
                visitor_addr: String::new(),
                client_addr: String::new(),
                relay_addr: String::new(),
                error: "owner client offline".to_string(),
            }));
        }

        metrics::inc_xtcp_owner_requests_forwarded();
        self.push_event(XtcpEvent {
            stage: "visitor_forwarded".to_string(),
            proxy_name: msg.proxy_name,
            visitor_run_id: visitor_run_id.to_string(),
            owner_run_id: owner.run_id,
            visitor_addr,
            client_addr: String::new(),
            relay_addr: owner.relay_addr,
            error: String::new(),
        })
        .await;

        Ok(None)
    }

    pub async fn handle_client_resp(
        &self,
        owner_run_id: &str,
        mut msg: NatHoleRespMsg,
    ) -> Result<()> {
        let raw_client_addr = msg.client_addr.clone();
        if let Some(public_peer) = self.control_manager.peer_addr(owner_run_id) {
            msg.client_addr = normalize_endpoint_addr(&msg.client_addr, &public_peer);
            debug!(
                "normalize NatHoleResp client_addr owner_run_id={} peer_addr={} raw={} normalized={}",
                owner_run_id, public_peer, raw_client_addr, msg.client_addr
            );
        }

        let Some((_, pending)) = self.pending.remove(&msg.visitor_addr) else {
            debug!(
                "no pending nat visitor found for visitor_addr={}, drop response",
                msg.visitor_addr
            );
            return Ok(());
        };

        debug!(
            "forward NatHoleResp for proxy {} to visitor run_id {}",
            pending.proxy_name, pending.visitor_run_id
        );
        if msg.relay_addr.is_empty() {
            msg.relay_addr = pending.relay_addr.clone();
        }

        metrics::inc_xtcp_responses_forwarded();
        self.push_event(XtcpEvent {
            stage: if msg.error.is_empty() {
                "response_forwarded".to_string()
            } else {
                "response_error".to_string()
            },
            proxy_name: pending.proxy_name.clone(),
            visitor_run_id: pending.visitor_run_id.clone(),
            owner_run_id: owner_run_id.to_string(),
            visitor_addr: msg.visitor_addr.clone(),
            client_addr: msg.client_addr.clone(),
            relay_addr: msg.relay_addr.clone(),
            error: msg.error.clone(),
        })
        .await;

        self.control_manager
            .send_message(&pending.visitor_run_id, Message::NatHoleResp(msg))
            .await
    }

    pub async fn recent_events(&self) -> Vec<XtcpEvent> {
        self.recent_events.lock().await.iter().cloned().collect()
    }

    async fn push_event(&self, event: XtcpEvent) {
        let mut events = self.recent_events.lock().await;
        if events.len() >= 32 {
            events.pop_front();
        }
        events.push_back(event);
    }
}

fn parse_signed_msg(signed_msg: &str) -> Result<(String, String)> {
    let (sk, visitor_addr) = signed_msg.split_once('|').ok_or_else(|| {
        Error::Protocol(
            "invalid NatHoleVisitor.signed_msg, expected '<sk>|<visitor_addr>'".to_string(),
        )
    })?;

    let sk = sk.trim().to_string();
    let visitor_addr = visitor_addr.trim().to_string();

    if sk.is_empty() {
        return Err(Error::Protocol("xtcp visitor sk is empty".to_string()));
    }
    if visitor_addr.is_empty() {
        return Err(Error::Protocol("xtcp visitor_addr is empty".to_string()));
    }

    Ok((sk, visitor_addr))
}

fn normalize_endpoint_addr(endpoint_addr: &str, peer_addr: &str) -> String {
    let Ok(endpoint_sa) = endpoint_addr.parse::<std::net::SocketAddr>() else {
        return endpoint_addr.to_string();
    };
    let owner_ip = peer_addr.parse::<std::net::SocketAddr>().map(|sa| sa.ip());
    let Ok(owner_ip) = owner_ip else {
        return endpoint_addr.to_string();
    };

    let endpoint_ip = endpoint_sa.ip();
    let use_owner_ip = endpoint_ip.is_unspecified() || endpoint_ip.is_loopback();
    if !use_owner_ip {
        return endpoint_addr.to_string();
    }

    std::net::SocketAddr::new(owner_ip, endpoint_sa.port()).to_string()
}
