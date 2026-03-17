use futures::{SinkExt, StreamExt};
use tokio::io::{duplex, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{tungstenite::Message as WsMessage, WebSocketStream};

use crate::transport::BoxedStream;

const BRIDGE_BUF: usize = 256 * 1024;

pub fn websocket_to_stream<S>(ws: WebSocketStream<S>) -> BoxedStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (app_stream, bridge_stream) = duplex(BRIDGE_BUF);
    let (mut bridge_r, mut bridge_w) = tokio::io::split(bridge_stream);

    let (mut ws_sink, mut ws_stream) = ws.split();

    // bridge → WebSocket: read from duplex, send as binary WS frames.
    // No Arc<Mutex> needed: each direction is exclusively owned by one task.
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match bridge_r.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let _ = ws_sink.close().await;
                    break;
                }
                Ok(n) => n,
            };
            if ws_sink
                .send(WsMessage::Binary(buf[..n].to_vec()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // WebSocket → bridge: receive WS frames, write payload to duplex.
    tokio::spawn(async move {
        while let Some(frame) = ws_stream.next().await {
            let Ok(frame) = frame else {
                break;
            };
            match frame {
                WsMessage::Binary(data) => {
                    if bridge_w.write_all(&data).await.is_err() {
                        break;
                    }
                }
                WsMessage::Text(text) => {
                    // Treat text frames as raw bytes (protocol fallback).
                    if bridge_w.write_all(text.as_bytes()).await.is_err() {
                        break;
                    }
                }
                WsMessage::Ping(_) => {
                    // tokio-tungstenite automatically queues a Pong reply for us.
                }
                WsMessage::Pong(_) => {}
                WsMessage::Close(_) => {
                    break;
                }
                WsMessage::Frame(_) => {}
            }
        }

        let _ = bridge_w.shutdown().await;
    });

    Box::new(app_stream)
}
