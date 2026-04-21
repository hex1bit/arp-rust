use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;
use tracing::{error, info};

use arp_common::config::{ClientConfig, ProxyConfig};

use crate::control::Control;

#[derive(Clone)]
pub struct AdminState {
    pub control: Arc<Control>,
    pub config: Arc<ClientConfig>,
}

#[derive(Serialize)]
struct StatusResp {
    status: String,
    server_addr: String,
    server_port: u16,
    run_id: String,
    proxies: Vec<String>,
}

#[derive(Serialize)]
struct ProxiesResp {
    total: usize,
    items: Vec<String>,
}

#[derive(Serialize)]
struct ProxyDetailResp {
    name: String,
    available: bool,
}

#[derive(Serialize)]
struct ActionResp {
    ok: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_addr: Option<String>,
}

pub fn start_admin_server(bind_addr: String, bind_port: u16, state: AdminState) {
    let need_auth = !state.config.admin_user.is_empty() && !state.config.admin_pwd.is_empty();
    let admin_user = state.config.admin_user.clone();
    let admin_pwd = state.config.admin_pwd.clone();

    tokio::spawn(async move {
        let app = Router::new()
            .route("/api/v1/status", get(api_status))
            .route("/api/v1/proxies", get(api_proxies))
            .route("/api/v1/proxies", post(api_add_proxy))
            .route("/api/v1/proxies/:name", get(api_proxy_detail))
            .route("/api/v1/proxies/:name", delete(api_delete_proxy))
            .route("/api/v1/reload", post(api_reload))
            .with_state(state);

        let app = if need_auth {
            app.layer(axum::middleware::from_fn(move |headers: HeaderMap, request: axum::extract::Request, next: axum::middleware::Next| {
                let user = admin_user.clone();
                let pwd = admin_pwd.clone();
                async move {
                    if check_basic_auth(&headers, &user, &pwd) {
                        next.run(request).await
                    } else {
                        (
                            StatusCode::UNAUTHORIZED,
                            [(header::WWW_AUTHENTICATE, "Basic realm=\"arp-admin\"")],
                            "Unauthorized",
                        )
                            .into_response()
                    }
                }
            }))
        } else {
            app
        };

        let addr = format!("{}:{}", bind_addr, bind_port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to bind admin server {}: {}", addr, e);
                return;
            }
        };
        info!("Client admin API listening on {}", addr);

        if let Err(e) = axum::serve(listener, app.into_make_service()).await {
            error!("Admin server failed: {}", e);
        }
    });
}

fn check_basic_auth(headers: &HeaderMap, expected_user: &str, expected_pwd: &str) -> bool {
    let Some(auth) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(auth_str) = auth.to_str() else {
        return false;
    };
    let Some(encoded) = auth_str.strip_prefix("Basic ") else {
        return false;
    };
    let decoded = base64_decode(encoded);
    let Ok(credentials) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((user, pwd)) = credentials.split_once(':') else {
        return false;
    };
    user == expected_user && pwd == expected_pwd
}

fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in input.as_bytes() {
        let val = if b == b'=' {
            break;
        } else if let Some(pos) = TABLE.iter().position(|&c| c == b) {
            pos as u32
        } else {
            continue;
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    output
}

async fn api_status(State(state): State<AdminState>) -> impl IntoResponse {
    let run_id = state.control.run_id_snapshot().await;
    let proxies = state.control.proxy_manager().list_proxy_names();
    Json(StatusResp {
        status: if run_id.is_empty() {
            "connecting".to_string()
        } else {
            "connected".to_string()
        },
        server_addr: state.config.server_addr.clone(),
        server_port: state.config.server_port,
        run_id,
        proxies,
    })
}

async fn api_proxies(State(state): State<AdminState>) -> impl IntoResponse {
    let items = state.control.proxy_manager().list_proxy_names();
    Json(ProxiesResp {
        total: items.len(),
        items,
    })
}

async fn api_proxy_detail(
    Path(name): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    match state.control.proxy_manager().get_proxy(&name) {
        Some(proxy) => (
            StatusCode::OK,
            Json(ProxyDetailResp {
                name: proxy.name().to_string(),
                available: proxy.is_available(),
            }),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ActionResp {
                ok: false,
                message: format!("proxy {} not found", name),
                remote_addr: None,
            }),
        )
            .into_response(),
    }
}

async fn api_add_proxy(
    State(state): State<AdminState>,
    Json(proxy_config): Json<ProxyConfig>,
) -> impl IntoResponse {
    if let Err(e) = proxy_config.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ActionResp {
                ok: false,
                message: format!("invalid proxy config: {}", e),
                remote_addr: None,
            }),
        )
            .into_response();
    }

    match state.control.register_proxy_dynamic(proxy_config).await {
        Ok(remote_addr) => (
            StatusCode::OK,
            Json(ActionResp {
                ok: true,
                message: "proxy registered".to_string(),
                remote_addr: Some(remote_addr),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ActionResp {
                ok: false,
                message: e.to_string(),
                remote_addr: None,
            }),
        )
            .into_response(),
    }
}

async fn api_delete_proxy(
    Path(name): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    match state.control.close_proxy_dynamic(name.clone()).await {
        Ok(()) => Json(ActionResp {
            ok: true,
            message: format!("proxy {} closed", name),
            remote_addr: None,
        }),
        Err(e) => Json(ActionResp {
            ok: false,
            message: e.to_string(),
            remote_addr: None,
        }),
    }
}

async fn api_reload(State(state): State<AdminState>) -> impl IntoResponse {
    let config_path = if let Some(path) = std::env::args().nth(2) {
        path
    } else {
        // Try -c/--config flag pattern
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == "-c" || a == "--config")
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| "client.toml".to_string())
    };

    match state.control.reload_proxies(&config_path).await {
        Ok((added, removed)) => Json(ActionResp {
            ok: true,
            message: format!("reload complete: {} added, {} removed", added, removed),
            remote_addr: None,
        }),
        Err(e) => Json(ActionResp {
            ok: false,
            message: format!("reload failed: {}", e),
            remote_addr: None,
        }),
    }
}
