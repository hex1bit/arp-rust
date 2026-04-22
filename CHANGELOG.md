# Changelog

All notable changes to ARP-Rust are documented in this file.

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
