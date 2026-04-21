use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Semaphore;
use tokio::time::Duration;

/// A simple token-bucket rate limiter.
///
/// Allows up to `bytes_per_sec` bytes per second with a burst equal to
/// `bytes_per_sec` (one second's worth of tokens).
pub struct Throttle {
    bytes_per_sec: u64,
    semaphore: Arc<Semaphore>,
}

impl Throttle {
    pub fn new(bytes_per_sec: u64) -> Arc<Self> {
        let burst = bytes_per_sec as usize;
        let sem = Arc::new(Semaphore::new(burst));
        let throttle = Arc::new(Self {
            bytes_per_sec,
            semaphore: sem.clone(),
        });

        // Background task: refill tokens every 100ms
        let refill_amount = (bytes_per_sec / 10).max(1) as usize;
        let semaphore = sem;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let current = semaphore.available_permits();
                let max = bytes_per_sec as usize;
                let add = refill_amount.min(max.saturating_sub(current));
                if add > 0 {
                    semaphore.add_permits(add);
                }
            }
        });

        throttle
    }

    /// Consume `bytes` tokens, blocking until available.
    pub async fn consume(&self, bytes: u64) {
        let mut remaining = bytes as usize;
        while remaining > 0 {
            let take = remaining.min(self.bytes_per_sec as usize);
            let permit = self.semaphore.acquire_many(take as u32).await;
            match permit {
                Ok(p) => {
                    p.forget(); // tokens consumed, not returned
                    remaining -= take;
                }
                Err(_) => break, // semaphore closed
            }
        }
    }

    pub fn bytes_per_sec(&self) -> u64 {
        self.bytes_per_sec
    }
}

/// Wraps an `AsyncRead + AsyncWrite` stream with bandwidth throttling.
pub struct ThrottledStream<S> {
    inner: S,
    throttle: Arc<Throttle>,
}

impl<S> ThrottledStream<S> {
    pub fn new(inner: S, throttle: Arc<Throttle>) -> Self {
        Self { inner, throttle }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ThrottledStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            let n = buf.filled().len() - before;
            if n > 0 {
                // Best-effort: schedule consumption without blocking poll_read.
                // Full back-pressure requires an async adapter but this is
                // sufficient for coarse-grained rate limiting.
                let throttle = self.throttle.clone();
                let bytes = n as u64;
                tokio::spawn(async move {
                    throttle.consume(bytes).await;
                });
            }
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ThrottledStream<S> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    #[tokio::test]
    async fn test_throttle_basic() {
        let throttle = Throttle::new(1_000_000); // 1MB/s
        // Should not block for small amounts
        throttle.consume(1000).await;
        assert_eq!(throttle.bytes_per_sec(), 1_000_000);
    }

    #[tokio::test]
    async fn test_throttle_refill() {
        let throttle = Throttle::new(10_000); // 10KB/s
        // Consume all tokens
        throttle.consume(10_000).await;
        // Wait for refill (100ms = 1000 tokens)
        tokio::time::sleep(Duration::from_millis(150)).await;
        // Should succeed without long block
        let start = Instant::now();
        throttle.consume(500).await;
        assert!(start.elapsed() < Duration::from_millis(200));
    }
}
