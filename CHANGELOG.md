# Changelog

All notable changes to ARP-Rust are documented in this file.

---

## [0.5.5] — 2026-05-13

### Fixed

- **ThrottledStream rate limiting completely ineffective** — the previous implementation used `tokio::spawn` (fire-and-forget) to consume tokens after data was already returned to the caller, providing zero actual backpressure. The stream is now rewritten with two internal fields:
  - `pending_data: Option<Bytes>` — bytes read from the inner stream but waiting for token approval
  - `pending_consume: Option<Pin<Box<dyn Future>>>` — the in-flight `consume()` future
  - `poll_read` now stores data in `pending_data` first (never touching the caller's `ReadBuf` until tokens are granted), satisfying the `AsyncRead` contract
  - New tests: `test_throttled_stream_enforces_rate` (3 KB at 1 KB/s ≥ 1.5 s) and `test_throttle_drop_does_not_leak`

- **`Throttle` background refill task memory leak** — the background task held a strong `Arc<Semaphore>` reference, preventing cleanup even after all `Arc<Throttle>` clones were dropped. Fixed by using `Arc::downgrade`; the task now exits automatically when the last `Throttle` is dropped.

- **`MuxTunnel` sub-stream close not notifying peer** — when the local sub-stream receiver was dropped (downstream TCP client disconnected), the MuxTunnel reader loop only removed the entry from the streams map but never sent a `MuxFrame::Close` back to the client. The client would continue sending data into the dead stream. Now sends `MuxFrame::Close { stream_id }` in both the "stream not found" and "send error" branches.

- **Admin `shutdown` API did not stop client from reconnecting** — `shutdown_run` previously called `cancel()` directly, which closed the TCP connection. The client treated the resulting `ConnectionClosed` error as retriable and re-connected. Now:
  1. Server sends a new `ServerShutdown` protocol message before closing
  2. Client `run_message_loop` handles `Message::ServerShutdown` → returns `Ok(())` (no reconnect)
  3. `run_message_loop` also monitors `CancellationToken` directly via a `select!` branch

### Added

- **`Message::ServerShutdown(ServerShutdownMsg)`** — new protocol message (type byte `'x'`) that the server sends to ask a client to exit without reconnecting. Carries an optional `reason` string for logging.

### Changed

- **`Control::shutdown_graceful()`** (server) — new method used by `ControlManager::shutdown_run`. Sends `ServerShutdown` via the outbound channel then cancels the connection after a 300 ms grace period, giving the message time to flush.

- **`test_e2e_stcp_sudp.sh` rewritten** — previous script used bare `nc` to connect to an STCP port (STCP requires an HMAC-signed visitor handshake, so `nc` always fails). New script:
  - Starts a provider `arpc` with `stcp` + `udp` proxies
  - Starts a visitor `arpc` with an `[[visitors]]` stcp entry (binds local port 22250)
  - Tests STCP through the visitor local port (end-to-end echo)
  - Tests plain UDP proxy via direct UDP send to server port

### Tested

- All 35 unit tests pass (1 ignored — requires open TCP listener)
- All 14 E2E test scripts pass

---

## [0.5.1] — 2026-04-23

### Changed

- **STCP (Secret TCP) completely redesigned** — previous implementation was architecturally wrong (opened a public port like regular TCP). New implementation follows the correct frp STCP design:
  - Provider registers STCP proxy → server stores in internal registry, **no public port opened**
  - Visitor client listens on a local port, holds `sk`, connects to provider through server relay
  - Server verifies `sk` via HMAC signature before allowing relay
  - Only visitors with the correct `sk` can access the service
  - New protocol message: `StcpVisitorConn` for visitor-to-server handshake

### Added

- New example configs: `client_stcp_provider.toml`, `client_stcp_visitor.toml`
- `StcpVisitorConn` protocol message (type byte `v`) for STCP visitor data connections
- Server `secret_registry` for STCP proxy entries (separate from TCP/XTCP registries)
- Server `handle_stcp_visitor_conn` for sk verification + work connection relay
- Client STCP visitor: local TCP listener + per-connection server relay with HMAC handshake

### Fixed

- STCP provider no longer opens a public port (was incorrectly sharing TCP's port-binding logic)
- STCP provider work connections now use plain TCP mode (not encrypted relay mode, since access control is server-side)

### Tested

- STCP provider registers → server confirms no public port (`remote_addr = "stcp"`)
- STCP visitor with correct sk → HTTP 200 through server relay
- STCP visitor with wrong sk → rejected (`stcp visitor sk mismatch`)
- All 33 unit tests passing

---

## [0.4.2] — 2026-04-22

### Fixed

- **Heartbeat timeout detection blocked by stuck network I/O** — when a WSS/TCP connection enters a half-open state (network degraded but TCP not closed), the client message loop could get stuck in `send_control_message` or `recv_control_message`, preventing the heartbeat timeout check from ever firing. Observed timeout values of 491s and 1050s instead of the configured 90s.
- **Mux auto-enable incorrectly applied to HTTP/HTTPS proxies** — v0.4.0 extended mux to HTTP/HTTPS, but server-side vhost routing sends raw HTTP bytes over work connections, not mux frames. This caused `unknown mux frame type: 71` errors ('G' from "GET /"). Mux auto-enable is now restricted to TCP proxies only.

### Improved

- **Three-layer heartbeat defense against stuck connections:**
  1. Separate `timeout_checker` (5s interval) — pure timestamp comparison with zero I/O, guaranteed to fire even when send/recv are blocked
  2. Send timeout (10s) — Ping write wrapped in `tokio::time::timeout`, returns error immediately if WSS write is stuck
  3. Recv timeout (60s) — `transport.recv()` wrapped in timeout, releases Mutex lock on timeout to let the select loop continue
- Worst-case heartbeat timeout detection: 90s (configured) + 5s (checker interval) = **95s**, instead of the previous unbounded duration

### Tested

- Vhost HTTP routing: subdomain and custom domain both verified end-to-end
- All 6 transport protocols re-verified: TCP, TCP+TLS, WS, WSS, KCP, QUIC

---

## [0.4.1] — 2026-04-22

### Added

- Chinese documentation: `README-zh.md`, `docs/配置指南.md`, `docs/开发说明.md`

### Fixed

- Removed debug log (`TcpProxy::new` trace) inadvertently left in client proxy initialization

---

## [0.4.0] — 2026-04-21

Major release: transforms ARP-Rust from a basic tunnel tool into a production-grade, dynamically manageable tunnel platform.

### Added

**Dynamic Proxy Management (v0.2)**
- Client REST API for runtime proxy management (`admin_addr`, `admin_port` config)
  - `GET /api/v1/status` — client connection status
  - `GET /api/v1/proxies` — list registered proxies
  - `GET /api/v1/proxies/:name` — proxy detail
  - `POST /api/v1/proxies` — add proxy at runtime (JSON body)
  - `DELETE /api/v1/proxies/:name` — remove proxy at runtime
  - `POST /api/v1/reload` — reload config file, diff and apply
- Basic Auth support for client admin API (`admin_user`, `admin_pwd`)
- `arpc status` subcommand — show connection and proxy status
- `arpc check [name]` subcommand — diagnose proxy and local service connectivity
- Config hot-reload via `SIGHUP` signal (Unix) — add/remove proxies without restart

**Security (v0.3)**
- HMAC-SHA256 signed authentication — tokens are never transmitted in plaintext
- Anti-replay protection with 5-minute timestamp verification window
- Structured JSON audit logging on server:
  - `client_login` / `client_login_failed`
  - `client_disconnect`
  - `proxy_registered` / `proxy_rejected` / `proxy_closed`
  - `work_conn_auth_failed`

**Operations (v0.3)**
- Graceful shutdown for both server and client
  - Server: stops accepting, drains connections (30s timeout)
  - Client: clean disconnect on Ctrl+C / SIGTERM
- Per-proxy metrics: `bytes_in`, `bytes_out`, `connections_total`, `connections_active`, `errors`
- Prometheus-format per-proxy metrics in `/metrics` endpoint

**Performance (v0.4)**
- `SessionCipher`: cached AES-256-GCM derived key, avoids per-packet SHA-256
- `Bytes` zero-copy in MuxFrame data path (replaces `Vec<u8>` heap allocations)
- Mux-first architecture: `tcp_mux` auto-enabled for TCP, HTTP, and HTTPS proxies

**Advanced Features (v0.4)**
- Multi-tenant runtime limits in `auth.rules`:
  - `max_connections` — per-client concurrent connection cap
  - `bandwidth_limit_bytes` — per-client bandwidth cap (bytes/sec)
- Token bucket rate limiter (`ThrottledStream`) for bandwidth enforcement
- SSE real-time event stream: `GET /api/v1/events/stream` on server dashboard

**Type Safety**
- `ProxyType` enum (`tcp`, `http`, `https`, `udp`, `stcp`, `sudp`, `xtcp`) replacing string matching
- `TransportProtocol` enum (`tcp`, `kcp`, `quic`, `websocket`) replacing string matching

**Code Quality**
- Extracted shared code to `arp-common`: `relay_stcp`, `write_frame`, `read_frame_optional`, `build_kcp_config`, `resolve_socket_addr`
- `pending_work_conns` changed from `Vec` to `VecDeque` (O(n) → O(1) dequeue)

### Fixed

- Client exits permanently when server closes connection — now reconnects with exponential backoff (2s → 4s → 8s → 16s → 30s max)
- Per-proxy metrics not recorded in `tcp_mux` mode
- Dynamic proxy registration deadlock — message loop blocked while waiting for server response
- `NatHoleVisitor.signed_msg` misleading name (now documented as simple `sk|addr` format, not cryptographic signature)

### Server Admin Endpoints (new)

- `GET /api/v1/events/stream` — SSE real-time audit event stream
- Per-proxy metrics in `/metrics`: `arp_proxy_bytes_in{proxy="..."}`, `arp_proxy_bytes_out{proxy="..."}`, etc.

### Client Admin Endpoints (new)

All endpoints listed under "Dynamic Proxy Management" above.

### Configuration Changes

**New client fields:**
- `admin_addr` — bind address for client admin API (default: not enabled)
- `admin_port` — port for client admin API (0 = disabled)
- `admin_user` — Basic Auth username (optional)
- `admin_pwd` — Basic Auth password (optional)

**New server fields:**
- `dashboard_addr` — bind address for dashboard (was implicit)

**New `auth.rules` fields:**
- `max_connections` — max concurrent connections per token (0 = unlimited)
- `bandwidth_limit_bytes` — bandwidth cap in bytes/sec per token (0 = unlimited)

### Testing

- 33 unit tests (up from 23), all passing
- Remote end-to-end tests on all 6 transport protocols: TCP, TCP+TLS, WS, WSS, KCP, QUIC
- 5-minute stability test passed (10/10 checks)
- Server restart auto-reconnection verified with exponential backoff

---

## [0.1.2] — 2026-04-20

### Added
- Configurable log file output with daily rotation and auto-purge (`log_file`, `log_max_days`)
- Log transport protocol label on client connect

### Fixed
- Minor logging improvements

---

## [0.1.1] — 2026-03-27

### Added
- Multi-platform release build script (mac-arm64, linux-x86_64, linux-arm64)
- HTTP/HTTPS virtual host routing with multi-backend round-robin
- XTCP NAT traversal with relay fallback
- TCP load-balancing groups with health-check-based backend eject/recover
- Server dashboard with HTML UI, REST API, and Prometheus metrics
- KCP and QUIC transport support
- WebSocket and WSS transport support

---

## [0.1.0] — 2026-03-17

### Added
- Initial release
- TCP/UDP proxy with client/server model
- STCP/SUDP encrypted proxy support
- TLS transport
- Token-based authentication with per-token rules
- TOML configuration
- Async runtime based on Tokio
