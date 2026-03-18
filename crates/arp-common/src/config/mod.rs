use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    #[serde(default = "default_bind_port")]
    pub bind_port: u16,

    #[serde(default)]
    pub kcp_bind_port: u16,

    #[serde(default)]
    pub quic_bind_port: u16,

    #[serde(default)]
    pub vhost_http_port: u16,

    #[serde(default)]
    pub vhost_https_port: u16,

    #[serde(default)]
    pub dashboard_addr: String,

    #[serde(default)]
    pub dashboard_port: u16,

    #[serde(default)]
    pub dashboard_user: String,

    #[serde(default)]
    pub dashboard_pwd: String,

    #[serde(default)]
    pub log_level: String,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub transport: TransportConfig,

    #[serde(default)]
    pub allow_ports: Vec<PortRange>,

    #[serde(default)]
    pub max_pool_count: u32,

    #[serde(default)]
    pub max_ports_per_client: u32,

    #[serde(default)]
    pub subdomain_host: String,

    #[serde(default)]
    pub tcp_mux: bool,

    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    #[serde(default = "default_server_port")]
    pub server_port: u16,

    #[serde(default)]
    pub client_id: String,

    #[serde(default)]
    pub log_level: String,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub transport: TransportConfig,

    #[serde(default)]
    pub admin_addr: String,

    #[serde(default)]
    pub admin_port: u16,

    #[serde(default)]
    pub admin_user: String,

    #[serde(default)]
    pub admin_pwd: String,

    #[serde(default)]
    pub proxies: Vec<ProxyConfig>,

    #[serde(default)]
    pub visitors: Vec<VisitorConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default = "default_auth_method")]
    pub method: String,

    #[serde(default)]
    pub token: String,

    #[serde(default)]
    pub oidc: OidcConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OidcConfig {
    #[serde(default)]
    pub client_id: String,

    #[serde(default)]
    pub client_secret: String,

    #[serde(default)]
    pub audience: String,

    #[serde(default)]
    pub issuer: String,

    #[serde(default)]
    pub token_endpoint_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default = "default_protocol")]
    pub protocol: String,

    #[serde(default = "default_tcp_mux")]
    pub tcp_mux: bool,

    #[serde(default)]
    pub tcp_mux_keepalive_interval: u64,

    #[serde(default)]
    pub pool_count: u32,

    #[serde(default)]
    pub heartbeat_interval: u64,

    #[serde(default)]
    pub heartbeat_timeout: u64,

    #[serde(default)]
    pub tls: TlsConfig,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            protocol: default_protocol(),
            tcp_mux: default_tcp_mux(),
            tcp_mux_keepalive_interval: 60,
            pool_count: 1,
            heartbeat_interval: 30,
            heartbeat_timeout: 90,
            tls: TlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub enable: bool,

    #[serde(default)]
    pub cert_file: String,

    #[serde(default)]
    pub key_file: String,

    #[serde(default)]
    pub trusted_ca_file: String,

    #[serde(default)]
    pub server_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub name: String,

    #[serde(rename = "type")]
    pub proxy_type: String,

    #[serde(default = "default_local_ip")]
    pub local_ip: String,

    #[serde(default)]
    pub local_port: u16,

    #[serde(default)]
    pub remote_port: u16,

    #[serde(default)]
    pub use_encryption: bool,

    #[serde(default)]
    pub use_compression: bool,

    #[serde(default)]
    pub custom_domains: Vec<String>,

    #[serde(default)]
    pub subdomain: String,

    #[serde(default)]
    pub locations: Vec<String>,

    #[serde(default)]
    pub host_header_rewrite: String,

    #[serde(default)]
    pub http_user: String,

    #[serde(default)]
    pub http_pwd: String,

    #[serde(default)]
    pub sk: String,
    #[serde(default = "default_true")]
    pub fallback_to_relay: bool,

    #[serde(default)]
    pub multiplexer: String,

    #[serde(default)]
    pub bandwidth_limit: String,

    #[serde(default)]
    pub bandwidth_limit_mode: String,

    #[serde(default)]
    pub health_check: HealthCheckConfig,

    #[serde(default)]
    pub load_balancer: LoadBalancerConfig,

    #[serde(default)]
    pub plugin: PluginConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitorConfig {
    pub name: String,

    #[serde(rename = "type")]
    pub visitor_type: String,

    #[serde(default)]
    pub server_name: String,

    #[serde(default)]
    pub sk: String,

    #[serde(default)]
    pub bind_addr: String,

    #[serde(default)]
    pub bind_port: u16,

    #[serde(default = "default_true")]
    pub fallback_to_relay: bool,

    #[serde(default = "default_xtcp_punch_timeout_secs")]
    pub xtcp_punch_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthCheckConfig {
    #[serde(default)]
    pub enable: bool,

    #[serde(default = "default_health_check_type")]
    pub check_type: String,

    #[serde(default)]
    pub timeout_seconds: u64,

    #[serde(default)]
    pub max_failed: u32,

    #[serde(default)]
    pub interval_seconds: u64,

    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadBalancerConfig {
    #[serde(default)]
    pub group: String,

    #[serde(default)]
    pub group_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    #[serde(default)]
    pub plugin_type: String,

    #[serde(default)]
    pub plugin_params: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRange {
    #[serde(default)]
    pub start: u16,

    #[serde(default)]
    pub end: u16,

    #[serde(default)]
    pub single: u16,
}

impl ServerConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

        toml::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse config: {}", e)))
    }

    pub fn validate(&self) -> Result<()> {
        if self.bind_port == 0 {
            return Err(Error::Config("bind_port cannot be 0".to_string()));
        }
        match self.transport.protocol.as_str() {
            "tcp" | "kcp" | "quic" | "websocket" => {}
            other => {
                return Err(Error::Config(format!(
                    "unsupported transport.protocol: {}",
                    other
                )));
            }
        }
        if self.transport.protocol == "quic"
            && (self.transport.tls.cert_file.is_empty() || self.transport.tls.key_file.is_empty())
        {
            return Err(Error::Config(
                "quic transport requires transport.tls.cert_file and transport.tls.key_file"
                    .to_string(),
            ));
        }
        if self.transport.protocol == "websocket"
            && self.transport.tls.enable
            && (self.transport.tls.cert_file.is_empty() || self.transport.tls.key_file.is_empty())
        {
            return Err(Error::Config(
                "websocket + tls requires transport.tls.cert_file and transport.tls.key_file"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl ClientConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

        toml::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse config: {}", e)))
    }

    pub fn validate(&self) -> Result<()> {
        if self.server_addr.is_empty() {
            return Err(Error::Config("server_addr cannot be empty".to_string()));
        }
        if self.server_port == 0 {
            return Err(Error::Config("server_port cannot be 0".to_string()));
        }
        match self.transport.protocol.as_str() {
            "tcp" | "kcp" | "quic" | "websocket" => {}
            other => {
                return Err(Error::Config(format!(
                    "unsupported transport.protocol: {}",
                    other
                )));
            }
        }
        if (self.transport.protocol == "quic"
            || (self.transport.protocol == "websocket" && self.transport.tls.enable)
            || (self.transport.protocol == "tcp" && self.transport.tls.enable))
            && self.transport.tls.trusted_ca_file.trim().is_empty()
        {
            return Err(Error::Config(format!(
                "{} transport requires transport.tls.trusted_ca_file",
                if self.transport.protocol == "websocket" {
                    "websocket + tls"
                } else {
                    self.transport.protocol.as_str()
                }
            )));
        }
        if (self.transport.protocol == "quic"
            || (self.transport.protocol == "websocket" && self.transport.tls.enable))
            && self.server_addr.parse::<std::net::IpAddr>().is_ok()
            && self.transport.tls.server_name.trim().is_empty()
        {
            return Err(Error::Config(format!(
                "{} transport with IP server_addr requires transport.tls.server_name",
                if self.transport.protocol == "websocket" {
                    "websocket + tls"
                } else {
                    self.transport.protocol.as_str()
                }
            )));
        }
        for proxy in &self.proxies {
            proxy.validate()?;
        }
        Ok(())
    }
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(Error::Config("proxy name cannot be empty".to_string()));
        }
        if self.proxy_type.is_empty() {
            return Err(Error::Config("proxy type cannot be empty".to_string()));
        }

        if self.local_port == 0 {
            return Err(Error::Config(format!(
                "proxy {} local_port cannot be 0",
                self.name
            )));
        }

        if self.proxy_type == "http" || self.proxy_type == "https" {
            if self.custom_domains.is_empty() && self.subdomain.trim().is_empty() {
                return Err(Error::Config(format!(
                    "proxy {} of type {} requires custom_domains or subdomain",
                    self.name, self.proxy_type
                )));
            }
        }

        if self.use_encryption && self.sk.trim().is_empty() {
            return Err(Error::Config(format!(
                "proxy {} enables encryption but sk is empty",
                self.name
            )));
        }

        if (self.proxy_type == "stcp" || self.proxy_type == "sudp" || self.proxy_type == "xtcp")
            && self.sk.trim().is_empty()
        {
            return Err(Error::Config(format!(
                "proxy {} of type {} requires sk",
                self.name, self.proxy_type
            )));
        }

        Ok(())
    }
}

fn default_bind_addr() -> String {
    "0.0.0.0".to_string()
}

fn default_bind_port() -> u16 {
    7000
}

fn default_server_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_server_port() -> u16 {
    7000
}

fn default_local_ip() -> String {
    "127.0.0.1".to_string()
}

fn default_protocol() -> String {
    "tcp".to_string()
}

fn default_tcp_mux() -> bool {
    true
}

fn default_auth_method() -> String {
    "token".to_string()
}

fn default_health_check_type() -> String {
    "tcp".to_string()
}

fn default_true() -> bool {
    true
}

fn default_xtcp_punch_timeout_secs() -> u64 {
    12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_proxy(proxy_type: &str) -> ProxyConfig {
        ProxyConfig {
            name: "p1".to_string(),
            proxy_type: proxy_type.to_string(),
            local_ip: "127.0.0.1".to_string(),
            local_port: 8080,
            remote_port: 0,
            use_encryption: false,
            use_compression: false,
            custom_domains: Vec::new(),
            subdomain: String::new(),
            locations: Vec::new(),
            host_header_rewrite: String::new(),
            http_user: String::new(),
            http_pwd: String::new(),
            sk: String::new(),
            fallback_to_relay: true,
            multiplexer: String::new(),
            bandwidth_limit: String::new(),
            bandwidth_limit_mode: String::new(),
            health_check: HealthCheckConfig::default(),
            load_balancer: LoadBalancerConfig::default(),
            plugin: PluginConfig::default(),
        }
    }

    #[test]
    fn test_http_proxy_requires_domain_or_subdomain() {
        let proxy = base_proxy("http");
        assert!(proxy.validate().is_err());

        let mut with_domain = base_proxy("http");
        with_domain.custom_domains = vec!["app.example.com".to_string()];
        assert!(with_domain.validate().is_ok());
    }

    #[test]
    fn test_https_proxy_requires_local_port() {
        let mut proxy = base_proxy("https");
        proxy.local_port = 0;
        proxy.custom_domains = vec!["secure.example.com".to_string()];
        assert!(proxy.validate().is_err());
    }

    #[test]
    fn test_encryption_requires_sk() {
        let mut proxy = base_proxy("udp");
        proxy.use_encryption = true;
        proxy.sk = String::new();
        assert!(proxy.validate().is_err());

        proxy.sk = "secret".to_string();
        assert!(proxy.validate().is_ok());
    }

    #[test]
    fn test_stcp_sudp_require_sk() {
        let stcp = base_proxy("stcp");
        assert!(stcp.validate().is_err());

        let mut stcp_ok = base_proxy("stcp");
        stcp_ok.sk = "secret".to_string();
        assert!(stcp_ok.validate().is_ok());

        let sudp = base_proxy("sudp");
        assert!(sudp.validate().is_err());

        let mut sudp_ok = base_proxy("sudp");
        sudp_ok.sk = "secret".to_string();
        assert!(sudp_ok.validate().is_ok());

        let xtcp = base_proxy("xtcp");
        assert!(xtcp.validate().is_err());

        let mut xtcp_ok = base_proxy("xtcp");
        xtcp_ok.sk = "secret".to_string();
        assert!(xtcp_ok.validate().is_ok());
    }

    #[test]
    fn test_client_websocket_tls_requires_ca() {
        let cfg = ClientConfig {
            server_addr: "example.com".to_string(),
            server_port: 7000,
            client_id: String::new(),
            log_level: String::new(),
            auth: AuthConfig::default(),
            transport: TransportConfig {
                protocol: "websocket".to_string(),
                tls: TlsConfig {
                    enable: true,
                    ..TlsConfig::default()
                },
                ..TransportConfig::default()
            },
            admin_addr: String::new(),
            admin_port: 0,
            admin_user: String::new(),
            admin_pwd: String::new(),
            proxies: Vec::new(),
            visitors: Vec::new(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_client_websocket_tls_requires_server_name_for_ip() {
        let cfg = ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port: 7000,
            client_id: String::new(),
            log_level: String::new(),
            auth: AuthConfig::default(),
            transport: TransportConfig {
                protocol: "websocket".to_string(),
                tls: TlsConfig {
                    enable: true,
                    trusted_ca_file: "ca.pem".to_string(),
                    ..TlsConfig::default()
                },
                ..TransportConfig::default()
            },
            admin_addr: String::new(),
            admin_port: 0,
            admin_user: String::new(),
            admin_pwd: String::new(),
            proxies: Vec::new(),
            visitors: Vec::new(),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_server_websocket_tls_requires_cert_and_key() {
        let cfg = ServerConfig {
            bind_addr: "0.0.0.0".to_string(),
            bind_port: 7000,
            kcp_bind_port: 0,
            quic_bind_port: 0,
            vhost_http_port: 0,
            vhost_https_port: 0,
            dashboard_addr: String::new(),
            dashboard_port: 0,
            dashboard_user: String::new(),
            dashboard_pwd: String::new(),
            log_level: String::new(),
            auth: AuthConfig::default(),
            transport: TransportConfig {
                protocol: "websocket".to_string(),
                tls: TlsConfig {
                    enable: true,
                    ..TlsConfig::default()
                },
                ..TransportConfig::default()
            },
            allow_ports: Vec::new(),
            max_pool_count: 0,
            max_ports_per_client: 0,
            subdomain_host: String::new(),
            tcp_mux: false,
            custom: HashMap::new(),
        };
        assert!(cfg.validate().is_err());
    }
}
