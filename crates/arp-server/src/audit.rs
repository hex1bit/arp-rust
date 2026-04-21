use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::info;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum AuditEvent {
    #[serde(rename = "client_login")]
    ClientLogin {
        client_id: String,
        peer_addr: String,
        run_id: String,
    },
    #[serde(rename = "client_login_failed")]
    ClientLoginFailed {
        peer_addr: String,
        reason: String,
    },
    #[serde(rename = "client_disconnect")]
    ClientDisconnect {
        client_id: String,
        run_id: String,
    },
    #[serde(rename = "proxy_registered")]
    ProxyRegistered {
        client_id: String,
        run_id: String,
        proxy_name: String,
        proxy_type: String,
        remote_addr: String,
    },
    #[serde(rename = "proxy_rejected")]
    ProxyRejected {
        client_id: String,
        run_id: String,
        proxy_name: String,
        reason: String,
    },
    #[serde(rename = "proxy_closed")]
    ProxyClosed {
        run_id: String,
        proxy_name: String,
    },
    #[serde(rename = "work_conn_auth_failed")]
    WorkConnAuthFailed {
        run_id: String,
        reason: String,
    },
}

static EVENT_BUS: Lazy<broadcast::Sender<AuditEvent>> = Lazy::new(|| {
    let (tx, _) = broadcast::channel(256);
    tx
});

pub fn emit(event: AuditEvent) {
    let json = serde_json::to_string(&event).unwrap_or_else(|_| format!("{:?}", event));
    info!(target: "audit", "{}", json);
    let _ = EVENT_BUS.send(event);
}

pub fn subscribe() -> broadcast::Receiver<AuditEvent> {
    EVENT_BUS.subscribe()
}
