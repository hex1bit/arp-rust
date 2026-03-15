use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use arp_common::protocol::{Message, NewProxyMsg, StartWorkConnMsg};
use arp_common::transport::{copy_bidirectional, MessageTransport};
use arp_common::{Error, Result};

use crate::proxy::{Proxy, WorkConnRequest};

const HTTP_HEADER_MAX: usize = 16 * 1024;
const TLS_HELLO_MAX: usize = 16 * 1024;
const WORK_CONN_TIMEOUT_SECS: u64 = 10;
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
struct VhostRoute {
    route_key: String,
    proxy_name: String,
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
}

pub struct VhostManager {
    bind_addr: String,
    subdomain_host: String,
    vhost_http_port: u16,
    vhost_https_port: u16,
    http_routes: DashMap<String, Vec<VhostRoute>>,
    https_routes: DashMap<String, Vec<VhostRoute>>,
    http_rr: DashMap<String, Arc<AtomicUsize>>,
    https_rr: DashMap<String, Arc<AtomicUsize>>,
    http_listener_started: AtomicBool,
    https_listener_started: AtomicBool,
}

impl VhostManager {
    pub fn new(
        bind_addr: String,
        subdomain_host: String,
        vhost_http_port: u16,
        vhost_https_port: u16,
    ) -> Self {
        Self {
            bind_addr,
            subdomain_host: subdomain_host.trim().trim_end_matches('.').to_lowercase(),
            vhost_http_port,
            vhost_https_port,
            http_routes: DashMap::new(),
            https_routes: DashMap::new(),
            http_rr: DashMap::new(),
            https_rr: DashMap::new(),
            http_listener_started: AtomicBool::new(false),
            https_listener_started: AtomicBool::new(false),
        }
    }

    pub fn register_http(
        self: &Arc<Self>,
        run_id: &str,
        msg: &NewProxyMsg,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<String> {
        if self.vhost_http_port == 0 {
            return Err(Error::Proxy(
                "vhost_http_port is 0, HTTP vhost proxy is disabled".to_string(),
            ));
        }

        let domains = self.collect_domains(msg)?;
        let route_key = format!("{}:{}", run_id, msg.proxy_name);
        self.add_routes(
            &self.http_routes,
            &self.http_rr,
            &domains,
            &route_key,
            &msg.proxy_name,
            work_conn_req_tx,
        )?;
        self.ensure_http_listener_started();
        Ok(format!("http://{}:{}", domains[0], self.vhost_http_port))
    }

    pub fn register_https(
        self: &Arc<Self>,
        run_id: &str,
        msg: &NewProxyMsg,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<String> {
        if self.vhost_https_port == 0 {
            return Err(Error::Proxy(
                "vhost_https_port is 0, HTTPS vhost proxy is disabled".to_string(),
            ));
        }

        let domains = self.collect_domains(msg)?;
        let route_key = format!("{}:{}", run_id, msg.proxy_name);
        self.add_routes(
            &self.https_routes,
            &self.https_rr,
            &domains,
            &route_key,
            &msg.proxy_name,
            work_conn_req_tx,
        )?;
        self.ensure_https_listener_started();
        Ok(format!("https://{}:{}", domains[0], self.vhost_https_port))
    }

    pub fn unregister_route(&self, route_key: &str, proxy_type: &str) {
        let routes = match proxy_type {
            "http" => &self.http_routes,
            "https" => &self.https_routes,
            _ => return,
        };
        let rr = match proxy_type {
            "http" => &self.http_rr,
            "https" => &self.https_rr,
            _ => return,
        };

        let keys: Vec<String> = routes
            .iter()
            .filter_map(|entry| {
                if entry.value().iter().any(|r| r.route_key == route_key) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();
        for key in keys {
            if let Some(mut entry) = routes.get_mut(&key) {
                entry.retain(|r| r.route_key != route_key);
                if !entry.is_empty() {
                    continue;
                }
            }
            routes.remove(&key);
            rr.remove(&key);
        }
    }

    fn collect_domains(&self, msg: &NewProxyMsg) -> Result<Vec<String>> {
        let mut domains: Vec<String> = msg
            .custom_domains
            .iter()
            .map(|d| normalize_host(d))
            .filter(|d| !d.is_empty())
            .collect();

        if !msg.subdomain.trim().is_empty() {
            if self.subdomain_host.is_empty() {
                return Err(Error::Proxy(
                    "subdomain is set but server subdomain_host is empty".to_string(),
                ));
            }
            domains.push(format!(
                "{}.{}",
                msg.subdomain.trim().to_lowercase(),
                self.subdomain_host
            ));
        }

        domains.sort();
        domains.dedup();

        if domains.is_empty() {
            return Err(Error::Proxy(
                "http/https proxy requires custom_domains or subdomain".to_string(),
            ));
        }

        Ok(domains)
    }

    fn add_routes(
        &self,
        routes: &DashMap<String, Vec<VhostRoute>>,
        rr: &DashMap<String, Arc<AtomicUsize>>,
        domains: &[String],
        route_key: &str,
        proxy_name: &str,
        work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    ) -> Result<()> {
        for domain in domains {
            {
                let mut entry = routes.entry(domain.clone()).or_default();
                if !entry.iter().any(|r| r.route_key == route_key) {
                    entry.push(VhostRoute {
                        route_key: route_key.to_string(),
                        proxy_name: proxy_name.to_string(),
                        work_conn_req_tx: work_conn_req_tx.clone(),
                    });
                }
            }
            rr.entry(domain.clone())
                .or_insert_with(|| Arc::new(AtomicUsize::new(0)));
        }
        Ok(())
    }

    fn ensure_http_listener_started(self: &Arc<Self>) {
        if self
            .http_listener_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = Arc::clone(&mgr).run_http_listener().await {
                    error!("HTTP vhost listener stopped: {}", e);
                    mgr.http_listener_started.store(false, Ordering::SeqCst);
                }
            });
        }
    }

    fn ensure_https_listener_started(self: &Arc<Self>) {
        if self
            .https_listener_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = Arc::clone(&mgr).run_https_listener().await {
                    error!("HTTPS vhost listener stopped: {}", e);
                    mgr.https_listener_started.store(false, Ordering::SeqCst);
                }
            });
        }
    }

    async fn run_http_listener(self: Arc<Self>) -> Result<()> {
        let bind_addr = format!("{}:{}", self.bind_addr, self.vhost_http_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(Error::Io)?;
        info!("HTTP vhost listener started on {}", bind_addr);

        loop {
            let (stream, addr) = listener.accept().await.map_err(Error::Io)?;
            let mgr = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = mgr.handle_http_conn(stream).await {
                    warn!("HTTP vhost connection {} failed: {}", addr, e);
                }
            });
        }
    }

    async fn run_https_listener(self: Arc<Self>) -> Result<()> {
        let bind_addr = format!("{}:{}", self.bind_addr, self.vhost_https_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(Error::Io)?;
        info!("HTTPS vhost listener started on {}", bind_addr);

        loop {
            let (stream, addr) = listener.accept().await.map_err(Error::Io)?;
            let mgr = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = mgr.handle_https_conn(stream).await {
                    warn!("HTTPS vhost connection {} failed: {}", addr, e);
                }
            });
        }
    }

    async fn handle_http_conn(&self, mut client_stream: TcpStream) -> Result<()> {
        let src_addr = client_stream
            .peer_addr()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::new());
        let dst_addr = client_stream
            .local_addr()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::new());
        let request_head = read_http_request_head(&mut client_stream).await?;
        let host = extract_http_host(&request_head)
            .ok_or_else(|| Error::Protocol("missing Host header".to_string()))?;
        let domain = normalize_host(&host);
        let route = self
            .http_routes
            .get(&domain)
            .and_then(|v| {
                let routes = v.value();
                if routes.is_empty() {
                    return None;
                }
                let rr = self
                    .http_rr
                    .entry(domain.clone())
                    .or_insert_with(|| Arc::new(AtomicUsize::new(0)));
                let idx = rr.fetch_add(1, Ordering::Relaxed) % routes.len();
                Some(routes[idx].clone())
            })
            .ok_or_else(|| Error::Proxy(format!("no http route for host {}", domain)))?;

        let work_transport = request_work_conn(
            route.work_conn_req_tx.clone(),
            &route.proxy_name,
            &src_addr,
            &dst_addr,
        )
        .await?;
        let mut work_stream = work_transport.into_inner();
        work_stream
            .write_all(&request_head)
            .await
            .map_err(Error::Io)?;
        copy_bidirectional(&mut client_stream, &mut work_stream).await?;
        Ok(())
    }

    async fn handle_https_conn(&self, mut client_stream: TcpStream) -> Result<()> {
        let src_addr = client_stream
            .peer_addr()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::new());
        let dst_addr = client_stream
            .local_addr()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::new());
        let client_hello = read_tls_client_hello(&mut client_stream).await?;
        let sni = parse_tls_sni(&client_hello)
            .ok_or_else(|| Error::Protocol("missing TLS SNI".to_string()))?;
        let domain = normalize_host(&sni);
        let route = self
            .https_routes
            .get(&domain)
            .and_then(|v| {
                let routes = v.value();
                if routes.is_empty() {
                    return None;
                }
                let rr = self
                    .https_rr
                    .entry(domain.clone())
                    .or_insert_with(|| Arc::new(AtomicUsize::new(0)));
                let idx = rr.fetch_add(1, Ordering::Relaxed) % routes.len();
                Some(routes[idx].clone())
            })
            .ok_or_else(|| Error::Proxy(format!("no https route for sni {}", domain)))?;

        let work_transport = request_work_conn(
            route.work_conn_req_tx.clone(),
            &route.proxy_name,
            &src_addr,
            &dst_addr,
        )
        .await?;
        let mut work_stream = work_transport.into_inner();
        work_stream
            .write_all(&client_hello)
            .await
            .map_err(Error::Io)?;
        copy_bidirectional(&mut client_stream, &mut work_stream).await?;
        Ok(())
    }
}

async fn request_work_conn(
    work_conn_req_tx: mpsc::Sender<WorkConnRequest>,
    proxy_name: &str,
    src_addr: &str,
    dst_addr: &str,
) -> Result<MessageTransport> {
    let (work_conn_tx, work_conn_rx) = tokio::sync::oneshot::channel();
    work_conn_req_tx
        .send(WorkConnRequest {
            proxy_name: proxy_name.to_string(),
            reply_tx: work_conn_tx,
        })
        .await
        .map_err(|_| {
            Error::Transport("failed to request work connection from control".to_string())
        })?;

    let mut work_transport = tokio::time::timeout(
        tokio::time::Duration::from_secs(WORK_CONN_TIMEOUT_SECS),
        work_conn_rx,
    )
    .await
    .map_err(|_| Error::Timeout("work connection timeout".to_string()))?
    .map_err(|_| Error::Transport("work connection channel closed".to_string()))?;

    work_transport
        .send(Message::StartWorkConn(StartWorkConnMsg {
            proxy_name: proxy_name.to_string(),
            src_addr: src_addr.to_string(),
            dst_addr: dst_addr.to_string(),
            error: String::new(),
        }))
        .await?;

    Ok(work_transport)
}

async fn read_http_request_head(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut data = Vec::with_capacity(1024);
    let deadline = tokio::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);
    // Read until HTTP header terminator so we can route by Host and still forward exact bytes.
    let result = tokio::time::timeout(deadline, async {
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).await.map_err(Error::Io)?;
            if n == 0 {
                return Err(Error::ConnectionClosed);
            }
            data.extend_from_slice(&buf[..n]);
            if data.windows(4).any(|w| w == b"\r\n\r\n") {
                return Ok(());
            }
            if data.len() >= HTTP_HEADER_MAX {
                return Err(Error::Protocol("http header too large".to_string()));
            }
        }
    })
    .await;

    match result {
        Ok(r) => r.map(|_| data),
        Err(_) => Err(Error::Timeout("http header read timeout".to_string())),
    }
}

async fn read_tls_client_hello(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let deadline = tokio::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);
    // Read exactly one TLS record (the initial ClientHello record in our vhost scenario).
    tokio::time::timeout(deadline, async {
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).await.map_err(Error::Io)?;
        if header[0] != 0x16 {
            return Err(Error::Protocol("not a TLS handshake record".to_string()));
        }
        let record_len = u16::from_be_bytes([header[3], header[4]]) as usize;
        if record_len == 0 || record_len > TLS_HELLO_MAX {
            return Err(Error::Protocol("invalid TLS record length".to_string()));
        }
        let mut payload = vec![0u8; record_len];
        stream.read_exact(&mut payload).await.map_err(Error::Io)?;
        let mut out = Vec::with_capacity(5 + record_len);
        out.extend_from_slice(&header);
        out.extend_from_slice(&payload);
        Ok(out)
    })
    .await
    .map_err(|_| Error::Timeout("tls client hello read timeout".to_string()))?
}

pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.').to_lowercase();
    if let Some(stripped) = trimmed.strip_prefix('[') {
        return stripped.trim_end_matches(']').to_string();
    }
    if let Some((name, _port)) = trimmed.rsplit_once(':') {
        if !name.contains(':') {
            return name.to_string();
        }
    }
    trimmed
}

pub fn extract_http_host(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    for line in text.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("host") {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

pub fn parse_tls_sni(record: &[u8]) -> Option<String> {
    // Parse enough of TLS ClientHello to extract extension 0x0000 (server_name/SNI).
    if record.len() < 5 || record[0] != 0x16 {
        return None;
    }
    let rec_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    if rec_len + 5 > record.len() {
        return None;
    }
    let mut pos = 5;
    if pos + 4 > record.len() || record[pos] != 0x01 {
        return None;
    }
    let hs_len = ((record[pos + 1] as usize) << 16)
        | ((record[pos + 2] as usize) << 8)
        | record[pos + 3] as usize;
    pos += 4;
    if pos + hs_len > record.len() {
        return None;
    }

    pos += 2 + 32;
    if pos >= record.len() {
        return None;
    }
    let session_id_len = record[pos] as usize;
    pos += 1 + session_id_len;
    if pos + 2 > record.len() {
        return None;
    }
    let cipher_len = u16::from_be_bytes([record[pos], record[pos + 1]]) as usize;
    pos += 2 + cipher_len;
    if pos >= record.len() {
        return None;
    }
    let compression_len = record[pos] as usize;
    pos += 1 + compression_len;
    if pos + 2 > record.len() {
        return None;
    }
    let extensions_len = u16::from_be_bytes([record[pos], record[pos + 1]]) as usize;
    pos += 2;
    let extensions_end = pos + extensions_len;
    if extensions_end > record.len() {
        return None;
    }

    while pos + 4 <= extensions_end {
        let ext_type = u16::from_be_bytes([record[pos], record[pos + 1]]);
        let ext_len = u16::from_be_bytes([record[pos + 2], record[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > extensions_end {
            return None;
        }
        if ext_type == 0x0000 {
            if ext_len < 2 {
                return None;
            }
            let mut sn_pos = pos + 2;
            let sn_end = pos + ext_len;
            while sn_pos + 3 <= sn_end {
                let name_type = record[sn_pos];
                let name_len =
                    u16::from_be_bytes([record[sn_pos + 1], record[sn_pos + 2]]) as usize;
                sn_pos += 3;
                if sn_pos + name_len > sn_end {
                    return None;
                }
                if name_type == 0 {
                    return std::str::from_utf8(&record[sn_pos..sn_pos + name_len])
                        .ok()
                        .map(|v| v.to_string());
                }
                sn_pos += name_len;
            }
            return None;
        }
        pos += ext_len;
    }

    None
}

pub struct HttpProxy {
    route_key: String,
    vhost_manager: Arc<VhostManager>,
}

impl HttpProxy {
    pub fn new(route_key: String, vhost_manager: Arc<VhostManager>) -> Self {
        Self {
            route_key,
            vhost_manager,
        }
    }
}

#[async_trait]
impl Proxy for HttpProxy {
    async fn run(&self) -> Result<()> {
        debug!("HTTP proxy route active: {}", self.route_key);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        self.vhost_manager.unregister_route(&self.route_key, "http");
        Ok(())
    }

    fn name(&self) -> &str {
        &self.route_key
    }

    fn proxy_type(&self) -> &str {
        "http"
    }
}

pub struct HttpsProxy {
    route_key: String,
    vhost_manager: Arc<VhostManager>,
}

impl HttpsProxy {
    pub fn new(route_key: String, vhost_manager: Arc<VhostManager>) -> Self {
        Self {
            route_key,
            vhost_manager,
        }
    }
}

#[async_trait]
impl Proxy for HttpsProxy {
    async fn run(&self) -> Result<()> {
        debug!("HTTPS proxy route active: {}", self.route_key);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        self.vhost_manager
            .unregister_route(&self.route_key, "https");
        Ok(())
    }

    fn name(&self) -> &str {
        &self.route_key
    }

    fn proxy_type(&self) -> &str {
        "https"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_http_host() {
        let req = b"GET / HTTP/1.1\r\nHost: Example.com:8080\r\nUser-Agent: test\r\n\r\n";
        let host = extract_http_host(req).unwrap();
        assert_eq!(normalize_host(&host), "example.com");
    }

    #[test]
    fn test_parse_tls_sni() {
        let host = "api.example.com";
        let record = build_test_client_hello(host);
        let parsed = parse_tls_sni(&record).unwrap();
        assert_eq!(parsed, host);
    }

    #[test]
    fn test_http_routes_allow_multi_backend() {
        let mgr = VhostManager::new("127.0.0.1".to_string(), "".to_string(), 18080, 18443);
        let (tx1, _rx1) = mpsc::channel(8);
        let (tx2, _rx2) = mpsc::channel(8);
        mgr.add_routes(
            &mgr.http_routes,
            &mgr.http_rr,
            &["app.example.com".to_string()],
            "run1:p1",
            "p1",
            tx1,
        )
        .unwrap();
        mgr.add_routes(
            &mgr.http_routes,
            &mgr.http_rr,
            &["app.example.com".to_string()],
            "run2:p2",
            "p2",
            tx2,
        )
        .unwrap();

        let routes = mgr.http_routes.get("app.example.com").unwrap();
        assert_eq!(routes.len(), 2);
    }

    fn build_test_client_hello(host: &str) -> Vec<u8> {
        let host_bytes = host.as_bytes();
        let mut ext = Vec::new();
        let sn_list_len = 1 + 2 + host_bytes.len();
        ext.extend_from_slice(&(sn_list_len as u16).to_be_bytes());
        ext.push(0);
        ext.extend_from_slice(&(host_bytes.len() as u16).to_be_bytes());
        ext.extend_from_slice(host_bytes);

        let mut extensions = Vec::new();
        extensions.extend_from_slice(&0u16.to_be_bytes());
        extensions.extend_from_slice(&(ext.len() as u16).to_be_bytes());
        extensions.extend_from_slice(&ext);

        let mut body = Vec::new();
        body.extend_from_slice(&[0x03, 0x03]);
        body.extend_from_slice(&[0u8; 32]);
        body.push(0);
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(&[0x00, 0x2f]);
        body.push(1);
        body.push(0);
        body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
        body.extend_from_slice(&extensions);

        let mut handshake = Vec::new();
        handshake.push(0x01);
        let len = body.len();
        handshake.extend_from_slice(&[
            ((len >> 16) & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            (len & 0xff) as u8,
        ]);
        handshake.extend_from_slice(&body);

        let mut record = Vec::new();
        record.push(0x16);
        record.extend_from_slice(&[0x03, 0x01]);
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }
}
