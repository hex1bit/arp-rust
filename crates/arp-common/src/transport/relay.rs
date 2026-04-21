use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::crypto::SessionCipher;
use crate::transport::BoxedStream;
use crate::{Error, Result};

/// Write a length-prefixed frame: [u32 big-endian length][payload].
pub async fn write_frame<W>(writer: &mut W, data: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    writer
        .write_u32(data.len() as u32)
        .await
        .map_err(Error::Io)?;
    writer.write_all(data).await.map_err(Error::Io)?;
    writer.flush().await.map_err(Error::Io)?;
    Ok(())
}

/// Read a length-prefixed frame. Returns `None` on clean EOF (peer shutdown).
pub async fn read_frame_optional<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return Err(Error::Protocol("stcp frame too large".to_string()));
    }
    let mut data = vec![0u8; len];
    reader.read_exact(&mut data).await.map_err(Error::Io)?;
    Ok(Some(data))
}

/// Relay an STCP (encrypted) connection between a local TCP stream and a
/// remote work stream. Uses `SessionCipher` to cache the derived AES key.
/// Returns `(bytes_read_from_local, bytes_written_to_local)`.
pub async fn relay_stcp(
    local_stream: TcpStream,
    work_stream: BoxedStream,
    secret: &str,
) -> Result<(u64, u64)> {
    let (mut local_r, mut local_w) = tokio::io::split(local_stream);
    let (mut work_r, mut work_w) = tokio::io::split(work_stream);
    let cipher_a = Arc::new(SessionCipher::new(secret));
    let cipher_b = Arc::new(SessionCipher::new(secret));

    let a = tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = local_r.read(&mut buf).await.map_err(Error::Io)?;
            if n == 0 {
                work_w.shutdown().await.map_err(Error::Io)?;
                return Ok::<u64, Error>(total);
            }
            total += n as u64;
            let encrypted = cipher_a.encrypt(&buf[..n])?;
            write_frame(&mut work_w, &encrypted).await?;
        }
    });

    let b = tokio::spawn(async move {
        let mut total: u64 = 0;
        loop {
            let Some(frame) = read_frame_optional(&mut work_r).await? else {
                local_w.shutdown().await.map_err(Error::Io)?;
                return Ok::<u64, Error>(total);
            };
            let plain = cipher_b.decrypt(&frame)?;
            total += plain.len() as u64;
            local_w.write_all(&plain).await.map_err(Error::Io)?;
        }
    });

    let (ra, rb) = tokio::join!(a, b);
    match (ra, rb) {
        (Ok(Ok(bytes_in)), Ok(Ok(bytes_out))) => Ok((bytes_in, bytes_out)),
        (Ok(Err(e)), _) => Err(e),
        (_, Ok(Err(e))) => Err(e),
        (Err(e), _) => Err(Error::Transport(format!("stcp task join failed: {}", e))),
        (_, Err(e)) => Err(Error::Transport(format!("stcp task join failed: {}", e))),
    }
}
