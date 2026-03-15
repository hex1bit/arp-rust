use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tracing::{error, info};

use crate::control::ControlManager;
use crate::metrics::METRICS;
use crate::proxy::ProxyManager;

#[derive(Clone)]
pub struct AdminState {
    pub control_manager: Arc<ControlManager>,
    pub proxy_manager: Arc<ProxyManager>,
    pub started_at_unix: i64,
}

#[derive(Serialize)]
struct StatusResp {
    status: &'static str,
    started_at_unix: i64,
    active_controls: usize,
    active_proxies: usize,
}

#[derive(Serialize)]
struct ProxiesResp {
    total: usize,
    items: Vec<String>,
}

pub fn start_admin_server(bind_addr: String, bind_port: u16, state: AdminState) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(dashboard))
            .route("/healthz", get(healthz))
            .route("/metrics", get(metrics))
            .route("/api/v1/status", get(api_status))
            .route("/api/v1/proxies", get(api_proxies))
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

async fn api_status(State(state): State<AdminState>) -> impl IntoResponse {
    Json(StatusResp {
        status: "ok",
        started_at_unix: state.started_at_unix,
        active_controls: state.control_manager.count(),
        active_proxies: state.proxy_manager.count(),
    })
}

async fn api_proxies(State(state): State<AdminState>) -> impl IntoResponse {
    let items = state.proxy_manager.list_proxy_keys();
    Json(ProxiesResp {
        total: items.len(),
        items,
    })
}

async fn dashboard(State(state): State<AdminState>) -> impl IntoResponse {
    let controls = state.control_manager.count();
    let proxies = state.proxy_manager.count();
    let html = format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>ARP Dashboard</title>
  <style>
    body {{ font-family: Menlo, monospace; margin: 32px; background: #f6f7fb; color: #222; }}
    .card {{ background: #fff; border: 1px solid #d9dce5; border-radius: 10px; padding: 16px; max-width: 740px; }}
    h1 {{ margin-top: 0; font-size: 22px; }}
    .num {{ font-size: 28px; font-weight: 700; }}
    a {{ color: #1f4acc; text-decoration: none; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>ARP Dashboard</h1>
    <p>Active Controls: <span class="num">{}</span></p>
    <p>Active Proxies: <span class="num">{}</span></p>
    <p>Metrics endpoint: <code>/metrics</code></p>
    <p>Admin API: <a href="/api/v1/status">/api/v1/status</a>, <a href="/api/v1/proxies">/api/v1/proxies</a></p>
  </div>
</body>
</html>"#,
        controls, proxies
    );
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

async fn metrics(State(state): State<AdminState>) -> impl IntoResponse {
    let body = format!(
        concat!(
            "arp_active_controls {}\n",
            "arp_active_proxies {}\n",
            "arp_incoming_connections_total {}\n",
            "arp_work_connections_total {}\n",
            "arp_tcp_proxy_connections_total {}\n",
            "arp_tcp_proxy_errors_total {}\n",
            "arp_tcp_mux_streams_opened_total {}\n",
            "arp_tcp_bytes_in_total {}\n",
            "arp_tcp_bytes_out_total {}\n",
        ),
        state.control_manager.count(),
        state.proxy_manager.count(),
        METRICS
            .incoming_connections_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .work_connections_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .tcp_proxy_connections_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .tcp_proxy_errors_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .tcp_mux_streams_opened_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .tcp_bytes_in_total
            .load(std::sync::atomic::Ordering::Relaxed),
        METRICS
            .tcp_bytes_out_total
            .load(std::sync::atomic::Ordering::Relaxed),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}
