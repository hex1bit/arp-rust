use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use once_cell::sync::Lazy;

pub struct Metrics {
    pub incoming_connections_total: AtomicU64,
    pub work_connections_total: AtomicU64,
    pub tcp_proxy_connections_total: AtomicU64,
    pub tcp_proxy_errors_total: AtomicU64,
    pub tcp_mux_streams_opened_total: AtomicU64,
    pub tcp_bytes_in_total: AtomicU64,
    pub tcp_bytes_out_total: AtomicU64,
    pub auth_failures_total: AtomicU64,
    pub proxy_registrations_total: AtomicU64,
    pub proxy_registration_failures_total: AtomicU64,
    pub req_work_conn_total: AtomicU64,
    pub idle_work_conn_hits_total: AtomicU64,
    pub xtcp_visitor_requests_total: AtomicU64,
    pub xtcp_sk_mismatch_total: AtomicU64,
    pub xtcp_owner_not_found_total: AtomicU64,
    pub xtcp_owner_offline_total: AtomicU64,
    pub xtcp_owner_requests_forwarded_total: AtomicU64,
    pub xtcp_responses_forwarded_total: AtomicU64,
}

pub static METRICS: Lazy<Metrics> = Lazy::new(|| Metrics {
    incoming_connections_total: AtomicU64::new(0),
    work_connections_total: AtomicU64::new(0),
    tcp_proxy_connections_total: AtomicU64::new(0),
    tcp_proxy_errors_total: AtomicU64::new(0),
    tcp_mux_streams_opened_total: AtomicU64::new(0),
    tcp_bytes_in_total: AtomicU64::new(0),
    tcp_bytes_out_total: AtomicU64::new(0),
    auth_failures_total: AtomicU64::new(0),
    proxy_registrations_total: AtomicU64::new(0),
    proxy_registration_failures_total: AtomicU64::new(0),
    req_work_conn_total: AtomicU64::new(0),
    idle_work_conn_hits_total: AtomicU64::new(0),
    xtcp_visitor_requests_total: AtomicU64::new(0),
    xtcp_sk_mismatch_total: AtomicU64::new(0),
    xtcp_owner_not_found_total: AtomicU64::new(0),
    xtcp_owner_offline_total: AtomicU64::new(0),
    xtcp_owner_requests_forwarded_total: AtomicU64::new(0),
    xtcp_responses_forwarded_total: AtomicU64::new(0),
});

pub fn inc_incoming_connections() {
    METRICS
        .incoming_connections_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_work_connections() {
    METRICS
        .work_connections_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_tcp_proxy_connections() {
    METRICS
        .tcp_proxy_connections_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_tcp_proxy_errors() {
    METRICS
        .tcp_proxy_errors_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_tcp_mux_streams() {
    METRICS
        .tcp_mux_streams_opened_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn add_tcp_bytes_in(v: u64) {
    METRICS.tcp_bytes_in_total.fetch_add(v, Ordering::Relaxed);
}

pub fn add_tcp_bytes_out(v: u64) {
    METRICS.tcp_bytes_out_total.fetch_add(v, Ordering::Relaxed);
}

pub fn inc_auth_failures() {
    METRICS.auth_failures_total.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_proxy_registrations() {
    METRICS
        .proxy_registrations_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_proxy_registration_failures() {
    METRICS
        .proxy_registration_failures_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_req_work_conn() {
    METRICS.req_work_conn_total.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_idle_work_conn_hits() {
    METRICS
        .idle_work_conn_hits_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_visitor_requests() {
    METRICS
        .xtcp_visitor_requests_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_sk_mismatch() {
    METRICS
        .xtcp_sk_mismatch_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_owner_not_found() {
    METRICS
        .xtcp_owner_not_found_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_owner_offline() {
    METRICS
        .xtcp_owner_offline_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_owner_requests_forwarded() {
    METRICS
        .xtcp_owner_requests_forwarded_total
        .fetch_add(1, Ordering::Relaxed);
}

pub fn inc_xtcp_responses_forwarded() {
    METRICS
        .xtcp_responses_forwarded_total
        .fetch_add(1, Ordering::Relaxed);
}

// ── Per-proxy metrics ─────────────────────────────────────────────────

pub struct ProxyMetrics {
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub connections_total: AtomicU64,
    pub connections_active: AtomicI64,
    pub errors: AtomicU64,
}

impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            connections_total: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

static PROXY_METRICS: Lazy<DashMap<String, Arc<ProxyMetrics>>> = Lazy::new(DashMap::new);

pub fn get_or_create_proxy_metrics(proxy_name: &str) -> Arc<ProxyMetrics> {
    PROXY_METRICS
        .entry(proxy_name.to_string())
        .or_insert_with(|| Arc::new(ProxyMetrics::new()))
        .clone()
}

pub fn remove_proxy_metrics(proxy_name: &str) {
    PROXY_METRICS.remove(proxy_name);
}

pub fn list_proxy_metrics() -> Vec<(String, Arc<ProxyMetrics>)> {
    PROXY_METRICS
        .iter()
        .map(|e| (e.key().clone(), e.value().clone()))
        .collect()
}
