pub mod mux;
pub mod prefixed;
pub mod quic_stream;
pub mod udp_mux;
pub mod ws_stream;

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tracing::{debug, error};

use crate::error::{Error, Result};
use crate::protocol::{Message, MessageCodec};

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type BoxedStream = Box<dyn AsyncStream>;

pub struct MessageTransport {
    framed: Framed<BoxedStream, MessageCodec>,
}

impl MessageTransport {
    pub fn new(stream: TcpStream) -> Self {
        Self::from_stream(Box::new(stream))
    }

    pub fn from_stream(stream: BoxedStream) -> Self {
        let framed = Framed::new(stream, MessageCodec);
        Self { framed }
    }

    pub async fn send(&mut self, msg: Message) -> Result<()> {
        debug!("Sending message: {:?}", msg.type_byte() as char);
        self.framed
            .send(msg)
            .await
            .map_err(|e| Error::Transport(format!("Failed to send message: {}", e)))
    }

    pub async fn recv(&mut self) -> Result<Option<Message>> {
        match self.framed.next().await {
            Some(Ok(msg)) => {
                debug!("Received message: {:?}", msg.type_byte() as char);
                Ok(Some(msg))
            }
            Some(Err(e)) => {
                error!("Failed to receive message: {}", e);
                Err(e)
            }
            None => Ok(None),
        }
    }

    pub fn into_inner(self) -> BoxedStream {
        self.framed.into_inner()
    }

    pub fn into_inner_with_read_buf(self) -> (BoxedStream, Vec<u8>) {
        let parts = self.framed.into_parts();
        (parts.io, parts.read_buf.to_vec())
    }
}

pub async fn copy_bidirectional<A, B>(stream_a: &mut A, stream_b: &mut B) -> Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin + ?Sized,
    B: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    tokio::io::copy_bidirectional(stream_a, stream_b)
        .await
        .map_err(|e| Error::Transport(format!("Bidirectional copy failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PingMsg;
    use tokio::net::TcpListener;

    #[tokio::test]
    #[ignore = "requires opening local TCP listener, which is restricted in some CI/sandbox environments"]
    async fn test_message_transport() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = MessageTransport::new(stream);

            let msg = transport.recv().await.unwrap().unwrap();
            match msg {
                Message::Ping(ping) => {
                    assert_eq!(ping.timestamp, 123456);
                }
                _ => panic!("Expected Ping message"),
            }

            transport
                .send(Message::Pong(crate::protocol::PongMsg {
                    timestamp: 654321,
                }))
                .await
                .unwrap();
        });

        let client_stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = MessageTransport::new(client_stream);

        transport
            .send(Message::Ping(PingMsg { timestamp: 123456 }))
            .await
            .unwrap();

        let msg = transport.recv().await.unwrap().unwrap();
        match msg {
            Message::Pong(pong) => {
                assert_eq!(pong.timestamp, 654321);
            }
            _ => panic!("Expected Pong message"),
        }

        server_handle.await.unwrap();
    }
}
