use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub enum MuxFrame {
    Open { stream_id: u32 },
    Data { stream_id: u32, payload: Bytes },
    Close { stream_id: u32 },
    Ping,
    Pong,
}

const FRAME_OPEN: u8 = 1;
const FRAME_DATA: u8 = 2;
const FRAME_CLOSE: u8 = 3;
const FRAME_PING: u8 = 4;
const FRAME_PONG: u8 = 5;
const MAX_FRAME_PAYLOAD: usize = 1024 * 1024;

pub async fn write_mux_frame<W>(writer: &mut W, frame: &MuxFrame) -> Result<()>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    match frame {
        MuxFrame::Open { stream_id } => {
            writer.write_u8(FRAME_OPEN).await.map_err(Error::Io)?;
            writer.write_u32(*stream_id).await.map_err(Error::Io)?;
        }
        MuxFrame::Data { stream_id, payload } => {
            writer.write_u8(FRAME_DATA).await.map_err(Error::Io)?;
            writer.write_u32(*stream_id).await.map_err(Error::Io)?;
            writer
                .write_u32(payload.len() as u32)
                .await
                .map_err(Error::Io)?;
            writer.write_all(payload).await.map_err(Error::Io)?;
        }
        MuxFrame::Close { stream_id } => {
            writer.write_u8(FRAME_CLOSE).await.map_err(Error::Io)?;
            writer.write_u32(*stream_id).await.map_err(Error::Io)?;
        }
        MuxFrame::Ping => {
            writer.write_u8(FRAME_PING).await.map_err(Error::Io)?;
        }
        MuxFrame::Pong => {
            writer.write_u8(FRAME_PONG).await.map_err(Error::Io)?;
        }
    }
    writer.flush().await.map_err(Error::Io)?;
    Ok(())
}

pub async fn read_mux_frame<R>(reader: &mut R) -> Result<MuxFrame>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let frame_type = reader.read_u8().await.map_err(Error::Io)?;
    match frame_type {
        FRAME_OPEN => {
            let stream_id = reader.read_u32().await.map_err(Error::Io)?;
            Ok(MuxFrame::Open { stream_id })
        }
        FRAME_DATA => {
            let stream_id = reader.read_u32().await.map_err(Error::Io)?;
            let len = reader.read_u32().await.map_err(Error::Io)? as usize;
            if len > MAX_FRAME_PAYLOAD {
                return Err(Error::Protocol(format!(
                    "mux payload too large: {} > {}",
                    len, MAX_FRAME_PAYLOAD
                )));
            }
            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload).await.map_err(Error::Io)?;
            Ok(MuxFrame::Data {
                stream_id,
                payload: Bytes::from(payload),
            })
        }
        FRAME_CLOSE => {
            let stream_id = reader.read_u32().await.map_err(Error::Io)?;
            Ok(MuxFrame::Close { stream_id })
        }
        FRAME_PING => Ok(MuxFrame::Ping),
        FRAME_PONG => Ok(MuxFrame::Pong),
        other => Err(Error::Protocol(format!(
            "unknown mux frame type: {}",
            other
        ))),
    }
}
