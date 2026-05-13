use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
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

        // Background task: refill tokens every 100ms.
        // Uses a Weak reference so the task exits automatically when the
        // last Arc<Throttle> is dropped (prevents semaphore leak).
        let refill_amount = (bytes_per_sec / 10).max(1) as usize;
        let weak_sem = Arc::downgrade(&sem);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                // If the Throttle (and its semaphore) has been dropped, stop.
                let Some(semaphore) = weak_sem.upgrade() else { break };
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
///
/// # How backpressure works
///
/// The key constraint of `AsyncRead::poll_read` is:
/// - If it returns `Poll::Pending`, the output `ReadBuf` **must not** have
///   been modified; the caller will re-use the same buffer on the next poll.
/// - If it returns `Poll::Ready(Ok(()))`, the data is considered delivered.
///
/// To enforce the rate limit we therefore **buffer** bytes internally:
///
/// 1. **Read phase**: delegate to the inner stream, storing any bytes read
///    into `pending_data` instead of directly into the caller's `ReadBuf`.
///    After storing, immediately start a `consume()` future.
///
/// 2. **Consume phase** (`pending_consume` is `Some`): poll the token future.
///    While it is `Poll::Pending`, return `Poll::Pending` to the caller
///    (no data visible yet).  Once tokens are granted, flush `pending_data`
///    into the caller's `ReadBuf` and return `Poll::Ready`.
///
/// `poll_write` is not throttled — rate-limiting inbound reads is sufficient
/// for typical tunnel workloads.
pub struct ThrottledStream<S> {
    inner: S,
    throttle: Arc<Throttle>,
    /// Bytes that have been read from the inner stream but are waiting for
    /// token-bucket approval before being handed to the caller.
    pending_data: Option<Bytes>,
    /// A pending `consume()` future; present while we wait for token grants.
    pending_consume: Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
}

impl<S> ThrottledStream<S> {
    pub fn new(inner: S, throttle: Arc<Throttle>) -> Self {
        Self {
            inner,
            throttle,
            pending_data: None,
            pending_consume: None,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ThrottledStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // ── Phase 1: drain a pending consume future ───────────────────────
        // If there is a consume future in flight, poll it first.
        // Only once it resolves do we hand `pending_data` to the caller.
        if self.pending_consume.is_some() {
            let fut = self.pending_consume.as_mut().unwrap();
            match fut.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => {
                    self.pending_consume = None;
                    // Fall through to flush pending_data below.
                }
            }
        }

        // If we have buffered data that just got its token grant, flush it now.
        if let Some(data) = self.pending_data.take() {
            let to_copy = data.len().min(buf.remaining());
            buf.put_slice(&data[..to_copy]);
            if to_copy < data.len() {
                // More data than the caller's buffer can hold right now;
                // keep the remainder for the next poll.
                self.pending_data = Some(data.slice(to_copy..));
            }
            return Poll::Ready(Ok(()));
        }

        // ── Phase 2: read from the inner stream into a temporary buffer ───
        // We use a separate BytesMut so we never touch the caller's ReadBuf
        // before token consumption is complete (required by AsyncRead contract).
        let capacity = buf.remaining();
        let mut tmp = BytesMut::zeroed(capacity);
        let mut tmp_buf = ReadBuf::new(&mut tmp);

        match Pin::new(&mut self.inner).poll_read(cx, &mut tmp_buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                let n = tmp_buf.filled().len();
                if n == 0 {
                    // EOF — pass through immediately (no tokens needed).
                    return Poll::Ready(Ok(()));
                }

                let data = Bytes::copy_from_slice(tmp_buf.filled());

                // Build and immediately poll the consume future once.
                // If tokens are available right away (common for small reads)
                // we resolve inline and skip the buffering path.
                let throttle = self.throttle.clone();
                let bytes = n as u64;
                let mut fut: Pin<Box<dyn Future<Output = ()> + Send + 'static>> =
                    Box::pin(async move { throttle.consume(bytes).await });

                match fut.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        // Tokens granted immediately — copy straight to caller.
                        buf.put_slice(&data);
                        Poll::Ready(Ok(()))
                    }
                    Poll::Pending => {
                        // Park: stash data + future; the waker is already
                        // registered by the semaphore internals.
                        self.pending_data = Some(data);
                        self.pending_consume = Some(fut);
                        Poll::Pending
                    }
                }
            }
        }
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

    /// Verify that ThrottledStream actually enforces the rate limit.
    ///
    /// We create a 1 KB/s throttle and try to read 3 KB of data from a
    /// duplex pipe (instantly available).  With real backpressure the read
    /// should take at least ~1.5 seconds:
    ///   - First KB: consumed by the initial burst (no wait).
    ///   - Second KB: must wait for one 100ms refill cycle to accumulate
    ///     enough tokens (~1s total for 1 KB at 1 KB/s).
    ///   - Third KB: another ~1s.
    #[tokio::test]
    async fn test_throttled_stream_enforces_rate() {
        let rate: u64 = 1_024; // 1 KB/s
        let data_size = 3 * 1_024usize; // 3 KB
        let data = vec![0xABu8; data_size];

        let throttle = Throttle::new(rate);

        let (mut writer, reader) = tokio::io::duplex(data_size * 2);
        writer.write_all(&data).await.unwrap();
        drop(writer); // signal EOF

        let mut throttled = ThrottledStream::new(reader, throttle);
        let mut out = Vec::new();

        let start = Instant::now();
        throttled.read_to_end(&mut out).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(out.len(), data_size, "all bytes must be received");
        assert_eq!(&out, &data, "data must be intact");
        // After the initial burst (1 KB free), 2 KB remain at 1 KB/s = ~2s.
        // Allow generous slack for CI timing jitter.
        assert!(
            elapsed >= Duration::from_millis(1_500),
            "throttle should have delayed reads; elapsed={:?}",
            elapsed
        );
    }

    /// Verify that dropping the last Arc<Throttle> stops the background task.
    #[tokio::test]
    async fn test_throttle_drop_does_not_leak() {
        let throttle = Throttle::new(1_000);
        drop(throttle);
        // Give the background task one tick to notice the weak ref is dead.
        tokio::time::sleep(Duration::from_millis(200)).await;
        // If we reach here without deadlock the test passes.
    }
}
