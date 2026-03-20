use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tracing::{error, info};

use crate::control::{ControlManager, ControlRecord};
use crate::metrics::METRICS;
use crate::nathole::NatHoleController;
use crate::proxy::{ProxyManager, ProxyRecord};

#[derive(Clone)]
pub struct AdminState {
    pub control_manager: Arc<ControlManager>,
    pub proxy_manager: Arc<ProxyManager>,
    pub nathole: Arc<NatHoleController>,
    pub started_at_unix: i64,
    pub transport_protocol: String,
    pub bind_addr: String,
    pub bind_port: u16,
    pub dashboard_enabled: bool,
    pub vhost_http_port: u16,
    pub vhost_https_port: u16,
}

#[derive(Serialize)]
struct StatusResp {
    status: &'static str,
    started_at_unix: i64,
    transport_protocol: String,
    bind_addr: String,
    bind_port: u16,
    dashboard_enabled: bool,
    vhost_http_port: u16,
    vhost_https_port: u16,
    active_controls: usize,
    active_proxies: usize,
    pending_work_connections: usize,
    idle_work_connections: usize,
    proxy_type_counts: Vec<TypeCount>,
}

#[derive(Serialize)]
struct TypeCount {
    proxy_type: String,
    total: usize,
}

#[derive(Serialize)]
struct ProxiesResp {
    total: usize,
    items: Vec<ProxyRecord>,
}

#[derive(Serialize)]
struct ClientsResp {
    total: usize,
    items: Vec<ControlRecord>,
}

#[derive(Serialize)]
struct ClientDetailResp {
    client: ControlRecord,
    pending_work_connections: usize,
    idle_work_connections: usize,
}

#[derive(Serialize)]
struct ActionResp {
    ok: bool,
    message: String,
}

#[derive(Serialize)]
struct ReadyResp {
    ready: bool,
    active_controls: usize,
    active_proxies: usize,
}

#[derive(Serialize)]
struct XtcpEventsResp {
    total: usize,
    items: Vec<crate::nathole::XtcpEvent>,
}

pub fn start_admin_server(bind_addr: String, bind_port: u16, state: AdminState) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(dashboard))
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics))
            .route("/api/v1/status", get(api_status))
            .route("/api/v1/proxies", get(api_proxies))
            .route("/api/v1/proxies/:name", get(api_proxy_detail))
            .route("/api/v1/clients", get(api_clients))
            .route("/api/v1/clients/:run_id", get(api_client_detail))
            .route("/api/v1/clients/:run_id/shutdown", post(api_client_shutdown))
            .route("/api/v1/xtcp/events", get(api_xtcp_events))
            .with_state(state);

        let addr = format!("{}:{}", bind_addr, bind_port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to bind admin server {}: {}", addr, e);
                return;
            }
        };
        info!("Admin server listening on {}", addr);

        if let Err(e) = axum::serve(listener, app).await {
            error!("Admin server failed: {}", e);
        }
    });
}

async fn healthz() -> impl IntoResponse {
    "ok"
}

async fn readyz(State(state): State<AdminState>) -> impl IntoResponse {
    let ready = state.active_ready().await;
    Json(ReadyResp {
        ready,
        active_controls: state.control_manager.count(),
        active_proxies: state.proxy_manager.count(),
    })
}

async fn api_status(State(state): State<AdminState>) -> impl IntoResponse {
    Json(state.status_snapshot().await)
}

async fn api_proxies(State(state): State<AdminState>) -> impl IntoResponse {
    let items = state.proxy_manager.list_records();
    Json(ProxiesResp {
        total: items.len(),
        items,
    })
}

async fn api_proxy_detail(
    Path(name): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    match state.proxy_manager.get_record(&name) {
        Some(record) => (StatusCode::OK, Json(record)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ActionResp {
                ok: false,
                message: format!("proxy {} not found", name),
            }),
        )
            .into_response(),
    }
}

async fn api_clients(State(state): State<AdminState>) -> impl IntoResponse {
    let items = state.control_manager.list_controls();
    Json(ClientsResp {
        total: items.len(),
        items,
    })
}

async fn api_client_detail(
    Path(run_id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(client) = state.control_manager.get_control_record(&run_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ActionResp {
                ok: false,
                message: format!("client {} not found", run_id),
            }),
        )
            .into_response();
    };
    let (pending_work_connections, idle_work_connections) = state
        .control_manager
        .get_control_queue_stats(&run_id)
        .await
        .unwrap_or((0, 0));
    (
        StatusCode::OK,
        Json(ClientDetailResp {
            client,
            pending_work_connections,
            idle_work_connections,
        }),
    )
        .into_response()
}

async fn api_client_shutdown(
    Path(run_id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if state.control_manager.get(&run_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ActionResp {
                ok: false,
                message: format!("client {} not found", run_id),
            }),
        )
            .into_response();
    }
    state.control_manager.shutdown_run(&run_id);
    (
        StatusCode::OK,
        Json(ActionResp {
            ok: true,
            message: format!("shutdown requested for {}", run_id),
        }),
    )
        .into_response()
}

async fn api_xtcp_events(State(state): State<AdminState>) -> impl IntoResponse {
    let items = state.nathole.recent_events().await;
    Json(XtcpEventsResp {
        total: items.len(),
        items,
    })
}

async fn dashboard(State(state): State<AdminState>) -> impl IntoResponse {
    let status = state.status_snapshot().await;
    let xtcp_events = state.nathole.recent_events().await;
    let html = format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>ARP Dashboard</title>
  <style>
    body {{ font-family: Menlo, monospace; margin: 32px; background: #f6f7fb; color: #222; }}
    .grid {{ display: grid; gap: 16px; max-width: 1100px; }}
    .card {{ background: #fff; border: 1px solid #d9dce5; border-radius: 10px; padding: 16px; }}
    h1, h2 {{ margin-top: 0; }}
    .num {{ font-size: 28px; font-weight: 700; }}
    code {{ background: #f3f4f8; padding: 1px 4px; border-radius: 4px; }}
    ul {{ padding-left: 18px; }}
  </style>
</head>
<body>
  <div class="grid">
    <div class="card">
      <h1>ARP Dashboard</h1>
      <p>Protocol: <code>{}</code></p>
      <p>Server Bind: <code>{}:{}</code></p>
      <p>Active Controls: <span class="num">{}</span></p>
      <p>Active Proxies: <span class="num">{}</span></p>
      <p>Pending Work Connections: <span class="num">{}</span></p>
      <p>Idle Work Connections: <span class="num">{}</span></p>
      <p>Admin API: <a href="/api/v1/status">/api/v1/status</a>, <a href="/api/v1/proxies">/api/v1/proxies</a>, <a href="/api/v1/clients">/api/v1/clients</a></p>
    </div>
    <div class="card">
      <h2>Proxy Types</h2>
      <ul>{}</ul>
    </div>
    <div class="card">
      <h2>Recent XTCP Events</h2>
      <ul>{}</ul>
    </div>
  </div>
</body>
</html>"#,
        status.transport_protocol,
        status.bind_addr,
        status.bind_port,
        status.active_controls,
        status.active_proxies,
        status.pending_work_connections,
        status.idle_work_connections,
        render_type_counts(&status.proxy_type_counts),
        render_xtcp_events(&xtcp_events),
    );
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

async fn metrics(State(state): State<AdminState>) -> impl IntoResponse {
    let status = state.status_snapshot().await;
    let body = format!(
        concat!(
            "arp_active_controls {}\n",
            "arp_active_proxies {}\n",
            "arp_pending_work_connections {}\n",
            "arp_idle_work_connections {}\n",
            "arp_incoming_connections_total {}\n",
            "arp_work_connections_total {}\n",
            "arp_tcp_proxy_connections_total {}\n",
            "arp_tcp_proxy_errors_total {}\n",
            "arp_tcp_mux_streams_opened_total {}\n",
            "arp_tcp_bytes_in_total {}\n",
            "arp_tcp_bytes_out_total {}\n",
            "arp_auth_failures_total {}\n",
            "arp_proxy_registrations_total {}\n",
            "arp_proxy_registration_failures_total {}\n",
            "arp_req_work_conn_total {}\n",
            "arp_idle_work_conn_hits_total {}\n",
            "arp_xtcp_visitor_requests_total {}\n",
            "arp_xtcp_sk_mismatch_total {}\n",
            "arp_xtcp_owner_not_found_total {}\n",
            "arp_xtcp_owner_offline_total {}\n",
            "arp_xtcp_owner_requests_forwarded_total {}\n",
            "arp_xtcp_responses_forwarded_total {}\n",
        ),
        status.active_controls,
        status.active_proxies,
        status.pending_work_connections,
        status.idle_work_connections,
        METRICS.incoming_connections_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.work_connections_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.tcp_proxy_connections_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.tcp_proxy_errors_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.tcp_mux_streams_opened_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.tcp_bytes_in_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.tcp_bytes_out_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.auth_failures_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.proxy_registrations_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.proxy_registration_failures_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.req_work_conn_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.idle_work_conn_hits_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_visitor_requests_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_sk_mismatch_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_owner_not_found_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_owner_offline_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_owner_requests_forwarded_total.load(std::sync::atomic::Ordering::Relaxed),
        METRICS.xtcp_responses_forwarded_total.load(std::sync::atomic::Ordering::Relaxed),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

impl AdminState {
    async fn status_snapshot(&self) -> StatusResp {
        let (pending_work_connections, idle_work_connections) =
            self.control_manager.get_queue_stats_snapshot().await;
        let proxy_type_counts = self
            .proxy_manager
            .count_by_type()
            .into_iter()
            .map(|(proxy_type, total)| TypeCount { proxy_type, total })
            .collect();
        StatusResp {
            status: "ok",
            started_at_unix: self.started_at_unix,
            transport_protocol: self.transport_protocol.clone(),
            bind_addr: self.bind_addr.clone(),
            bind_port: self.bind_port,
            dashboard_enabled: self.dashboard_enabled,
            vhost_http_port: self.vhost_http_port,
            vhost_https_port: self.vhost_https_port,
            active_controls: self.control_manager.count(),
            active_proxies: self.proxy_manager.count(),
            pending_work_connections,
            idle_work_connections,
            proxy_type_counts,
        }
    }

    async fn active_ready(&self) -> bool {
        let status = self.status_snapshot().await;
        status.active_controls > 0 || status.active_proxies == 0
    }
}

fn render_type_counts(items: &[TypeCount]) -> String {
    if items.is_empty() {
        return "<li>none</li>".to_string();
    }
    items
        .iter()
        .map(|item| format!("<li><code>{}</code>: {}</li>", item.proxy_type, item.total))
        .collect::<Vec<_>>()
        .join("")
}

fn render_xtcp_events(items: &[crate::nathole::XtcpEvent]) -> String {
    if items.is_empty() {
        return "<li>none</li>".to_string();
    }
    items
        .iter()
        .rev()
        .take(8)
        .map(|item| {
            format!(
                "<li><code>{}</code> proxy=<code>{}</code> visitor=<code>{}</code> relay=<code>{}</code> err=<code>{}</code></li>",
                item.stage,
                item.proxy_name,
                item.visitor_addr,
                item.relay_addr,
                item.error,
            )
        })
        .collect::<Vec<_>>()
        .join("")
}
