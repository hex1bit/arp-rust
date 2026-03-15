use std::io::Cursor;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::BoxedStream;

pub struct PrefixedStream {
    prefix: Cursor<Vec<u8>>,
    inner: BoxedStream,
}

impl PrefixedStream {
    pub fn new(prefix: Vec<u8>, inner: BoxedStream) -> Self {
        Self {
            prefix: Cursor::new(prefix),
            inner,
        }
    }
}

impl AsyncRead for PrefixedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let total = self.prefix.get_ref().len() as u64;
        let pos = self.prefix.position();
        if pos < total && buf.remaining() > 0 {
            let start = pos as usize;
            let remaining = (total - pos) as usize;
            let to_copy = remaining.min(buf.remaining());
            let end = start + to_copy;
            buf.put_slice(&self.prefix.get_ref()[start..end]);
            self.prefix.set_position(end as u64);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
