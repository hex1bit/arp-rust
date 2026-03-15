use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;

pub struct Metrics {
    pub incoming_connections_total: AtomicU64,
    pub work_connections_total: AtomicU64,
    pub tcp_proxy_connections_total: AtomicU64,
    pub tcp_proxy_errors_total: AtomicU64,
    pub tcp_mux_streams_opened_total: AtomicU64,
    pub tcp_bytes_in_total: AtomicU64,
    pub tcp_bytes_out_total: AtomicU64,
}

pub static METRICS: Lazy<Metrics> = Lazy::new(|| Metrics {
    incoming_connections_total: AtomicU64::new(0),
    work_connections_total: AtomicU64::new(0),
    tcp_proxy_connections_total: AtomicU64::new(0),
    tcp_proxy_errors_total: AtomicU64::new(0),
    tcp_mux_streams_opened_total: AtomicU64::new(0),
    tcp_bytes_in_total: AtomicU64::new(0),
    tcp_bytes_out_total: AtomicU64::new(0),
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
