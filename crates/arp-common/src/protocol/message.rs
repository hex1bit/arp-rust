use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    Login(LoginMsg),
    LoginResp(LoginRespMsg),
    NewProxy(NewProxyMsg),
    NewProxyResp(NewProxyRespMsg),
    CloseProxy(CloseProxyMsg),
    ReqWorkConn(ReqWorkConnMsg),
    NewWorkConn(NewWorkConnMsg),
    StartWorkConn(StartWorkConnMsg),
    Ping(PingMsg),
    Pong(PongMsg),
    UdpPacket(UdpPacketMsg),
    NatHoleVisitor(NatHoleVisitorMsg),
    NatHoleClient(NatHoleClientMsg),
    NatHoleResp(NatHoleRespMsg),
    StcpVisitorConn(StcpVisitorConnMsg),
}

impl Message {
    pub fn type_byte(&self) -> u8 {
        match self {
            Message::Login(_) => b'o',
            Message::LoginResp(_) => b'1',
            Message::NewProxy(_) => b'p',
            Message::NewProxyResp(_) => b'2',
            Message::CloseProxy(_) => b'c',
            Message::ReqWorkConn(_) => b'r',
            Message::NewWorkConn(_) => b'w',
            Message::StartWorkConn(_) => b's',
            Message::Ping(_) => b'h',
            Message::Pong(_) => b'4',
            Message::UdpPacket(_) => b'u',
            Message::NatHoleVisitor(_) => b'i',
            Message::NatHoleClient(_) => b'n',
            Message::NatHoleResp(_) => b'm',
            Message::StcpVisitorConn(_) => b'v',
        }
    }

    pub fn from_type_byte(byte: u8) -> Option<&'static str> {
        match byte {
            b'o' => Some("Login"),
            b'1' => Some("LoginResp"),
            b'p' => Some("NewProxy"),
            b'2' => Some("NewProxyResp"),
            b'c' => Some("CloseProxy"),
            b'r' => Some("ReqWorkConn"),
            b'w' => Some("NewWorkConn"),
            b's' => Some("StartWorkConn"),
            b'h' => Some("Ping"),
            b'4' => Some("Pong"),
            b'u' => Some("UdpPacket"),
            b'i' => Some("NatHoleVisitor"),
            b'n' => Some("NatHoleClient"),
            b'm' => Some("NatHoleResp"),
            b'v' => Some("StcpVisitorConn"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginMsg {
    pub version: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub client_id: String,
    pub timestamp: i64,
    #[serde(default)]
    pub privilege_key: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub pool_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRespMsg {
    pub version: String,
    pub run_id: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProxyMsg {
    pub proxy_name: String,
    pub proxy_type: String,
    #[serde(default)]
    pub use_encryption: bool,
    #[serde(default)]
    pub use_compression: bool,
    #[serde(default)]
    pub local_ip: String,
    #[serde(default)]
    pub local_port: u16,
    #[serde(default)]
    pub remote_port: u16,
    #[serde(default)]
    pub custom_domains: Vec<String>,
    #[serde(default)]
    pub subdomain: String,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub host_header_rewrite: String,
    #[serde(default)]
    pub sk: String,
    #[serde(default)]
    pub multiplexer: String,
    #[serde(default = "default_true")]
    pub fallback_to_relay: bool,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProxyRespMsg {
    pub proxy_name: String,
    #[serde(default)]
    pub remote_addr: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseProxyMsg {
    pub proxy_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqWorkConnMsg {
    #[serde(default)]
    pub proxy_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewWorkConnMsg {
    pub run_id: String,
    #[serde(default)]
    pub privilege_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartWorkConnMsg {
    pub proxy_name: String,
    #[serde(default)]
    pub src_addr: String,
    #[serde(default)]
    pub dst_addr: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PingMsg {
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PongMsg {
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpPacketMsg {
    pub content: Vec<u8>,
    #[serde(default)]
    pub local_addr: String,
    #[serde(default)]
    pub remote_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatHoleVisitorMsg {
    pub proxy_name: String,
    pub signed_msg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatHoleClientMsg {
    pub proxy_name: String,
    pub visitor_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatHoleRespMsg {
    pub visitor_addr: String,
    pub client_addr: String,
    #[serde(default)]
    pub relay_addr: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StcpVisitorConnMsg {
    pub proxy_name: String,
    pub sk_signature: String,
    pub timestamp: i64,
}

fn default_true() -> bool {
    true
}
