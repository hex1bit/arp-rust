use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use tokio::io::{duplex, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_tungstenite::{tungstenite::Message as WsMessage, WebSocketStream};

use crate::transport::BoxedStream;

const BRIDGE_BUF: usize = 256 * 1024;

pub fn websocket_to_stream<S>(ws: WebSocketStream<S>) -> BoxedStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (app_stream, bridge_stream) = duplex(BRIDGE_BUF);
    let (bridge_r, bridge_w) = tokio::io::split(bridge_stream);
    let bridge_w = Arc::new(Mutex::new(bridge_w));

    let (ws_sink, mut ws_stream) = ws.split();
    let ws_sink = Arc::new(Mutex::new(ws_sink));

    let ws_sink_send = Arc::clone(&ws_sink);
    let bridge_w_close = Arc::clone(&bridge_w);
    tokio::spawn(async move {
        let mut bridge_r = bridge_r;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match bridge_r.read(&mut buf).await {
                Ok(0) => {
                    let _ = ws_sink_send.lock().await.close().await;
                    break;
                }
                Ok(n) => n,
                Err(_) => {
                    let _ = ws_sink_send.lock().await.close().await;
                    break;
                }
            };

            if ws_sink_send
                .lock()
                .await
                .send(WsMessage::Binary(buf[..n].to_vec().into()))
                .await
                .is_err()
            {
                break;
            }
        }

        let _ = bridge_w_close.lock().await.shutdown().await;
    });

    let bridge_w_recv = Arc::clone(&bridge_w);
    tokio::spawn(async move {
        while let Some(frame) = ws_stream.next().await {
            let Ok(frame) = frame else {
                break;
            };
            match frame {
                WsMessage::Binary(data) => {
                    if bridge_w_recv.lock().await.write_all(&data).await.is_err() {
                        break;
                    }
                }
                WsMessage::Text(_) => {}
                WsMessage::Ping(_) => {}
                WsMessage::Pong(_) => {}
                WsMessage::Close(_) => {
                    break;
                }
                _ => {}
            }
        }

        let _ = bridge_w_recv.lock().await.shutdown().await;
    });

    Box::new(app_stream)
}
