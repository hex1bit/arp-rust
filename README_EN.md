# ARP-Rust

[中文 README](README.md)

ARP in `ARP-Rust` stands for `Advance Reverse Proxy`.

ARP-Rust is a fast, secure, and high-performance reverse proxy tool written in Rust. It is designed to expose local services to the public network through a compact client/server architecture.

## Features

- Async runtime based on Tokio
- TCP proxy
- UDP proxy and persistent UDP tunnel
- HTTP/HTTPS virtual host routing
- STCP / SUDP support
- XTCP NAT traversal workflow
- TCP load-balancing groups
- TLS transport
- KCP transport
- QUIC transport
- WebSocket transport
- Admin endpoints for health, metrics, and proxy status
- TOML-based configuration

## Workspace Layout

```text
arp-rust/
├── crates/
│   ├── arp-common/
│   ├── arp-server/
│   └── arp-client/
├── examples/
├── docs/
└── test/
```

## Build

```bash
cargo build --workspace --release
```

Release binaries:

- `target/release/arps`
- `target/release/arpc`

## Quick Start

Server example:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"
tcp_mux = true

[[allow_ports]]
start = 6001
end = 7000
```

Client example:

```toml
server_addr = "server.example.com"
server_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"
tcp_mux = true
pool_count = 1

[[proxies]]
name = "ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Run:

```bash
./target/release/arps -c server.toml
./target/release/arpc -c client.toml
```

## Tests

Unit tests:

```bash
cargo test --workspace
```

End-to-end tests:

```bash
bash test/test_e2e.sh
bash test/test_e2e_tcp_mux.sh
bash test/test_e2e_vhost.sh
bash test/test_e2e_udp.sh
bash test/test_e2e_stcp_sudp.sh
bash test/test_e2e_tcp_lb_health.sh
bash test/test_e2e_xtcp.sh
bash test/test_e2e_ws.sh
bash test/test_e2e_tls.sh
bash test/test_e2e_kcp.sh
bash test/test_e2e_quic.sh
```

## Admin Endpoints

When `dashboard_addr` and `dashboard_port` are configured on the server:

- `GET /healthz`
- `GET /metrics`
- `GET /api/v1/status`
- `GET /api/v1/proxies`

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

MIT
