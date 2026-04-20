# ARP-Rust

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

ARP in `ARP-Rust` stands for `Advance Reverse Proxy`.

ARP-Rust is a reverse proxy and tunneling tool written in Rust. It uses a client/server model to expose private TCP, UDP, HTTP, and HTTPS services to the public network, with support for multiple transport backends including TCP, TLS, KCP, QUIC, WebSocket, and WebSocket over TLS (`wss`).

## Status

ARP-Rust is already usable for real client/server forwarding scenarios, including public-network deployments. At the current stage, the most validated transport paths are:

- `tcp`
- `tcp + tls`
- `ws`
- `wss`
- `quic`

For first-time production deployment, `wss` is the recommended default because it balances compatibility, encryption, and ease of deployment.

## Why ARP-Rust

ARP-Rust is aimed at users who want a self-hosted tunneling tool with:

- a compact Rust implementation
- multiple transport backends instead of a single fixed tunnel type
- support for both raw TCP/UDP forwarding and HTTP/HTTPS virtual-host routing
- optional secure and NAT-traversal-oriented proxy modes such as `stcp`, `sudp`, and `xtcp`
- a practical path from simple local testing to public-internet deployment

## Use Cases

Typical use cases include:

- exposing SSH on a private machine through a public server
- publishing an internal web service through HTTP/HTTPS virtual-host routing
- forwarding private database or cache ports for controlled remote access
- exposing custom TCP or UDP services from home labs or office networks
- building a small multi-node TCP service behind one public port with load balancing
- testing NAT traversal flows with `xtcp`

## Production Notes

Before deploying ARP-Rust to the public internet, verify the following:

- use `wss` or `tcp + tls` instead of plain `tcp`
- make sure `auth.token` is changed from the example placeholder
- generate a proper server certificate with matching `subjectAltName`
- keep `transport.tls.server_name` aligned with the certificate hostname
- open only the control port and the required `allow_ports` range in the firewall
- start with a single `tcp` proxy first, then expand to more transports or proxy types

## Features

- Async runtime based on Tokio
- TCP proxy
- UDP proxy and persistent UDP tunnel
- HTTP/HTTPS virtual host routing
- STCP / SUDP support
- XTCP NAT traversal workflow
- TCP load-balancing groups
- Health-check-based backend eject/recover
- TLS transport
- KCP transport
- QUIC transport
- WebSocket transport (`ws` / `wss`)
- Admin endpoints for health, metrics, and proxy status
- TOML-based configuration
- File-based log output with daily rotation and auto-purge (`log_file`, `log_max_days`)

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

Native build:

```bash
cargo build --workspace --release
```

Release binaries:

- `target/release/arps`
- `target/release/arpc`

Docker one-click Linux build from macOS/Linux:

```bash
bash scripts/build-linux.sh
```

Output files:

- `dist/linux-x86_64/arps`
- `dist/linux-x86_64/arpc`
- `dist/arp-rust-linux-x86_64.tar.gz`

The Docker workflow builds inside a Linux container, so it avoids the local cross-toolchain issue on macOS.

If you want prepacked release bundles with example configs, place the binaries together with the files under `examples/` and your chosen certificate files.

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

Then connect through the public side:

```bash
ssh user@server.example.com -p 6001
```

This quick start uses plain TCP for simplicity. For public production usage, prefer the `WSS` deployment path described below.

## Proxy Types

Supported `[[proxies]].type` values include:

- `tcp`
- `udp`
- `http`
- `https`
- `stcp`
- `sudp`
- `xtcp`

In practice:

- use `tcp` for generic TCP services such as SSH, databases, custom TCP applications, and game servers
- use `udp` for UDP-based services such as DNS-like workloads and custom UDP protocols
- use `http` / `https` when you want virtual-host routing based on `Host` or TLS `SNI`
- use `stcp` / `sudp` / `xtcp` when you need shared-secret or NAT traversal flows

## Transport Protocols

Supported `transport.protocol` values include:

- `tcp`
- `kcp`
- `quic`
- `websocket`

TLS can be enabled on top of TCP and WebSocket via `transport.tls.enable = true`.

A practical rule of thumb:

- choose plain `tcp` for the simplest and most stable setup
- choose `tcp + tls` for production deployments on the public internet
- choose `wss` when compatibility with web infrastructure or restricted networks matters most
- choose `quic` when UDP is available and you want better transport performance

In other words:

- choose `wss` as the safe default
- choose `quic` as the performance-oriented option
- choose plain `tcp` mainly for local, lab, or controlled-network setups

## Recommended Deployment Path

If you are deploying ARP-Rust for the first time, the most practical rollout order is:

1. Start with plain `tcp` on a controlled network
2. Move to `tcp + tls` or `wss` before exposing the service to the public internet
3. Use `wss` if you want easier compatibility with reverse proxies, port 443, or restricted networks
4. Use `quic` only after confirming UDP reachability and firewall policy

Recommended defaults:

- local or lab setup: `tcp`
- public production setup: `wss`
- performance-oriented deployment with confirmed UDP support: `quic`

## WSS vs QUIC

- `wss`
  - runs on top of `TCP + TLS + WebSocket`
  - easier to integrate with reverse proxies, CDNs, and HTTP/HTTPS-only environments
  - better when you need compatibility and web-like traffic shape
  - usually has more protocol overhead than QUIC

- `quic`
  - runs on top of `UDP` with built-in TLS
  - usually performs better for handshake latency, multiplexing, and high concurrency
  - better when you control the network and UDP is available
  - depends on firewall and upstream UDP reachability

Practical guidance:

- choose `wss` first for compatibility and restricted networks
- choose `quic` first for performance-oriented deployments where UDP is allowed

## Example Configs

For a scenario-based configuration guide, see `docs/CONFIGURATION_GUIDE.md`.

General examples:

- `examples/server.toml`
- `examples/client.toml`

WebSocket / WSS:

- `examples/server_ws.toml`
- `examples/client_ws.toml`
- `examples/server_prod_wss.toml`
- `examples/client_prod_wss.toml`

KCP / QUIC:

- `examples/server_kcp.toml`
- `examples/client_kcp.toml`
- `examples/server_quic.toml`
- `examples/client_quic.toml`

XTCP:

- `examples/client_xtcp_provider.toml`
- `examples/client_xtcp_visitor.toml`

Vhost HTTP / HTTPS:

- `examples/server_vhost_http.toml`
- `examples/server_vhost_https.toml`
- `examples/client_http_custom_domain.toml`
- `examples/client_http_subdomain.toml`
- `examples/client_https_custom_domain.toml`

## TLS / WSS Certificates

When `transport.tls.enable = true`, the server certificate should:

- be a server certificate, not `CA:TRUE`
- include the real hostname or IP in `subjectAltName`
- match the client-side `transport.tls.server_name`
- be trusted by the client via `trusted_ca_file`

Recommended self-signed certificate command:

```bash
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout server.key \
  -out server.crt \
  -days 365 \
  -subj "/CN=your.server.name" \
  -addext "subjectAltName=DNS:your.server.name,IP:127.0.0.1" \
  -addext "basicConstraints=CA:FALSE" \
  -addext "keyUsage=digitalSignature,keyEncipherment" \
  -addext "extendedKeyUsage=serverAuth"
```

Or use the helper script:

```bash
cd examples
bash gen_self_signed_cert.sh your.server.name 1.2.3.4
```

## WSS Deployment Steps

1. Generate the server certificate and key:

```bash
cd examples
bash gen_self_signed_cert.sh your.server.name 1.2.3.4
```

2. Copy them to the server:

```bash
sudo mkdir -p /etc/arp
sudo cp server.crt /etc/arp/server.crt
sudo cp server.key /etc/arp/server.key
sudo chmod 600 /etc/arp/server.key
```

3. Prepare the server config using `examples/server_prod_wss.toml`

4. Prepare the client config using `examples/client_prod_wss.toml`

5. Start and verify:

```bash
./target/release/arps -c examples/server_prod_wss.toml
./target/release/arpc -c examples/client_prod_wss.toml
nc -vz your.server.name 6001
```

If `nc -vz your.server.name 6001` succeeds but application traffic still fails, check:

- the client-side `local_ip` / `local_port`
- the server firewall or cloud security group
- certificate hostname matching
- whether another process is already using the chosen remote port

## Tests

Full test suite (unit + all E2E):

```bash
bash scripts/test-full.sh
```

Individual tests are also available under `test/` if you want to run a specific scenario.

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
