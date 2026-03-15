use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Codec error: {0}")]
    Codec(String),

    #[error("Proxy error: {0}")]
    Proxy(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Invalid message type: {0}")]
    InvalidMessage(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Channel send error")]
    ChannelSend,

    #[error("Channel receive error")]
    ChannelRecv,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelSend
    }
}

impl From<tokio::sync::oneshot::error::RecvError> for Error {
    fn from(_: tokio::sync::oneshot::error::RecvError) -> Self {
        Error::ChannelRecv
    }
}
