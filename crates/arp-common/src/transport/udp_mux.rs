use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub enum UdpMuxFrame {
    Req {
        request_id: u32,
        src_addr: String,
        payload: Vec<u8>,
    },
    Resp {
        request_id: u32,
        payload: Vec<u8>,
    },
    Ping,
    Pong,
}

const FRAME_REQ: u8 = 1;
const FRAME_RESP: u8 = 2;
const FRAME_PING: u8 = 3;
const FRAME_PONG: u8 = 4;
const MAX_ADDR_LEN: usize = 1024;
const MAX_PAYLOAD_LEN: usize = 1024 * 1024;

pub async fn write_udp_mux_frame<W>(writer: &mut W, frame: &UdpMuxFrame) -> Result<()>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    match frame {
        UdpMuxFrame::Req {
            request_id,
            src_addr,
            payload,
        } => {
            if src_addr.len() > MAX_ADDR_LEN {
                return Err(Error::Protocol("udp mux src_addr too long".to_string()));
            }
            writer.write_u8(FRAME_REQ).await.map_err(Error::Io)?;
            writer.write_u32(*request_id).await.map_err(Error::Io)?;
            writer
                .write_u16(src_addr.len() as u16)
                .await
                .map_err(Error::Io)?;
            writer
                .write_all(src_addr.as_bytes())
                .await
                .map_err(Error::Io)?;
            writer
                .write_u32(payload.len() as u32)
                .await
                .map_err(Error::Io)?;
            writer.write_all(payload).await.map_err(Error::Io)?;
        }
        UdpMuxFrame::Resp {
            request_id,
            payload,
        } => {
            writer.write_u8(FRAME_RESP).await.map_err(Error::Io)?;
            writer.write_u32(*request_id).await.map_err(Error::Io)?;
            writer
                .write_u32(payload.len() as u32)
                .await
                .map_err(Error::Io)?;
            writer.write_all(payload).await.map_err(Error::Io)?;
        }
        UdpMuxFrame::Ping => {
            writer.write_u8(FRAME_PING).await.map_err(Error::Io)?;
        }
        UdpMuxFrame::Pong => {
            writer.write_u8(FRAME_PONG).await.map_err(Error::Io)?;
        }
    }
    writer.flush().await.map_err(Error::Io)?;
    Ok(())
}

pub async fn read_udp_mux_frame<R>(reader: &mut R) -> Result<UdpMuxFrame>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let frame_type = reader.read_u8().await.map_err(Error::Io)?;
    match frame_type {
        FRAME_REQ => {
            let request_id = reader.read_u32().await.map_err(Error::Io)?;
            let addr_len = reader.read_u16().await.map_err(Error::Io)? as usize;
            if addr_len > MAX_ADDR_LEN {
                return Err(Error::Protocol("udp mux src_addr too long".to_string()));
            }
            let mut addr = vec![0u8; addr_len];
            reader.read_exact(&mut addr).await.map_err(Error::Io)?;
            let payload_len = reader.read_u32().await.map_err(Error::Io)? as usize;
            if payload_len > MAX_PAYLOAD_LEN {
                return Err(Error::Protocol("udp mux payload too large".to_string()));
            }
            let mut payload = vec![0u8; payload_len];
            reader.read_exact(&mut payload).await.map_err(Error::Io)?;
            Ok(UdpMuxFrame::Req {
                request_id,
                src_addr: String::from_utf8(addr)
                    .map_err(|e| Error::Protocol(format!("invalid udp mux src_addr: {}", e)))?,
                payload,
            })
        }
        FRAME_RESP => {
            let request_id = reader.read_u32().await.map_err(Error::Io)?;
            let payload_len = reader.read_u32().await.map_err(Error::Io)? as usize;
            if payload_len > MAX_PAYLOAD_LEN {
                return Err(Error::Protocol("udp mux payload too large".to_string()));
            }
            let mut payload = vec![0u8; payload_len];
            reader.read_exact(&mut payload).await.map_err(Error::Io)?;
            Ok(UdpMuxFrame::Resp {
                request_id,
                payload,
            })
        }
        FRAME_PING => Ok(UdpMuxFrame::Ping),
        FRAME_PONG => Ok(UdpMuxFrame::Pong),
        other => Err(Error::Protocol(format!(
            "unknown udp mux frame type: {}",
            other
        ))),
    }
}
