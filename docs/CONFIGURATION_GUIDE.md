# Configuration Guide

This guide explains how to prepare `server.toml` and `client.toml` for different transport protocols and deployment scenarios in ARP-Rust.

## 1. Configuration model

ARP-Rust uses two sides:

- **server**: public entry point, receives client control/work connections, exposes remote ports or vhost routes
- **client**: runs near the private service, connects to the server, and forwards local traffic outward

Typical startup:

```bash
./target/release/arps -c server.toml
./target/release/arpc -c client.toml
```

## 2. Common server fields

Minimal server fields:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000
log_level = "info"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"

[[allow_ports]]
start = 6000
end = 7000
```

Important fields:

- `bind_addr`: server listen address
- `bind_port`: control port for `tcp` / `tcp+tls` / `websocket`
- `kcp_bind_port`: optional KCP UDP port, defaults to `bind_port`
- `quic_bind_port`: optional QUIC UDP port, defaults to `bind_port`
- `vhost_http_port`: public HTTP vhost entry port
- `vhost_https_port`: public HTTPS vhost entry port
- `dashboard_addr` / `dashboard_port`: optional admin endpoints
- `allow_ports`: allowed remote port ranges for `tcp` / `udp` style proxies
- `subdomain_host`: base domain for HTTP/HTTPS subdomain routing

## 3. Common client fields

Minimal client fields:

```toml
server_addr = "server.example.com"
server_port = 17000
client_id = "node-a"
log_level = "info"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"
pool_count = 1
```

Important fields:

- `server_addr`: public server address
- `server_port`: control port on the server
- `client_id`: stable identity of the client instance
- `transport.pool_count`: number of pre-warmed work connections
- `transport.heartbeat_interval`: client heartbeat period
- `transport.heartbeat_timeout`: local timeout for detecting dead server connections
- `[[proxies]]`: provider-side proxy definitions
- `[[visitors]]`: visitor-side config for `xtcp`

### Why `client_id` matters

If the same client reconnects and re-registers the same `proxy.name`, the server can safely take over the old stale connection only when the `client_id` matches.

If two different clients use the same `proxy.name` but different `client_id` values, the server rejects the second one instead of kicking out the first one.

## 4. Proxy fields

Common `[[proxies]]` fields:

```toml
[[proxies]]
name = "ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Common meanings:

- `name`: proxy identifier, should be unique within your deployment intent
- `type`: `tcp`, `udp`, `http`, `https`, `stcp`, `sudp`, `xtcp`
- `local_ip` / `local_port`: private service address on the client machine
- `remote_port`: public port allocated on the server for TCP/UDP style proxies
- `custom_domains`: domain list for `http` / `https`
- `subdomain`: subdomain routing for `http` / `https`
- `sk`: shared secret for `stcp`, `sudp`, `xtcp`
- `fallback_to_relay`: whether `xtcp` falls back to relay TCP when direct punch fails
- `multiplexer`: can be left empty in most cases; `tcp_mux` is auto-enabled for standard TCP when allowed

## 5. Transport protocol scenarios

### 5.1 Plain TCP: easiest for local, lab, controlled networks

Use when:

- you want the simplest setup
- both sides are under your control
- you are exposing SSH, databases, internal services in a controlled network

Server:

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
start = 6000
end = 7000
```

Client:

```toml
server_addr = "server.example.com"
server_port = 17000
client_id = "ssh-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"
tcp_mux = true
pool_count = 1

[[proxies]]
name = "ssh_tcp_22"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Reference examples:

- `examples/server.toml`
- `examples/client.toml`
- `examples/server_remote.toml`
- `examples/client_remote_tcp22.toml`

### 5.2 TCP + TLS: production-oriented encrypted TCP tunnel

Use when:

- you want plain TCP semantics with TLS encryption
- you do not need WebSocket wrapping
- your network allows direct TLS TCP traffic

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"

[transport.tls]
enable = true
cert_file = "/etc/arp/server.crt"
key_file = "/etc/arp/server.key"

[[allow_ports]]
start = 6001
end = 7000
```

Client:

```toml
server_addr = "your.server.name"
server_port = 17000
client_id = "tls-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"
pool_count = 1

[transport.tls]
enable = true
trusted_ca_file = "/etc/arp/server.crt"
server_name = "your.server.name"

[[proxies]]
name = "ssh_tls"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Notes:

- if `server_addr` is an IP, also set `transport.tls.server_name`
- `server_name` must match the server certificate SAN/CN

### 5.3 WebSocket / WSS: recommended default for public deployment

Use when:

- you want the best compatibility with reverse proxies and standard web ports
- the network is restrictive
- you plan to run on port `443`

#### WS for testing

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"

[[allow_ports]]
start = 6001
end = 7000
```

Client:

```toml
server_addr = "127.0.0.1"
server_port = 17000
client_id = "ws-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"
pool_count = 1

[[proxies]]
name = "ssh_ws"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Reference examples:

- `examples/server_ws.toml`
- `examples/client_ws.toml`

#### WSS for production

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 443

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"
tcp_mux = true

[transport.tls]
enable = true
cert_file = "/etc/arp/server.crt"
key_file = "/etc/arp/server.key"

[[allow_ports]]
start = 6001
end = 7000
```

Client:

```toml
server_addr = "your.server.name"
server_port = 443
client_id = "prod-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"
tcp_mux = true
pool_count = 1

[transport.tls]
enable = true
trusted_ca_file = "/etc/arp/server.crt"
server_name = "your.server.name"

[[proxies]]
name = "ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Reference examples:

- `examples/server_prod_wss.toml`
- `examples/client_prod_wss.toml`

### 5.4 KCP: UDP-based transport for controlled networks

Use when:

- both sides allow UDP
- you want a lighter UDP-based transport
- you are testing or running in a controlled network

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000
kcp_bind_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "kcp"

[[allow_ports]]
start = 6001
end = 7000
```

Client:

```toml
server_addr = "your.server.name"
server_port = 17000
client_id = "kcp-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "kcp"
pool_count = 1

[[proxies]]
name = "kcp_ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Reference examples:

- `examples/server_kcp.toml`
- `examples/client_kcp.toml`

### 5.5 QUIC: performance-oriented transport when UDP is available

Use when:

- you want UDP + TLS + multiplexing
- you control firewall policy and UDP reachability
- you prefer a performance-oriented deployment over maximum compatibility

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000
quic_bind_port = 17000

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "quic"

[transport.tls]
cert_file = "/etc/arp/server.crt"
key_file = "/etc/arp/server.key"

[[allow_ports]]
start = 6001
end = 7000
```

Client:

```toml
server_addr = "your.server.name"
server_port = 17000
client_id = "quic-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "quic"
pool_count = 1

[transport.tls]
trusted_ca_file = "/etc/arp/server.crt"
server_name = "your.server.name"

[[proxies]]
name = "quic_ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Reference examples:

- `examples/server_quic.toml`
- `examples/client_quic.toml`

## 6. Application scenarios

### 6.1 Expose SSH

Best choices:

- lab / controlled network: `tcp`
- public internet default: `wss`
- UDP allowed and performance-focused: `quic`

Typical proxy section:

```toml
[[proxies]]
name = "ssh"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6001
```

Use:

```bash
ssh user@server.example.com -p 6001
```

### 6.2 Expose a local web app as a raw TCP service

Use when the application already speaks HTTP locally but you only need public port forwarding.

Example:

```toml
[[proxies]]
name = "web_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 8080
remote_port = 6002
```

Then access it through:

- `http://server.example.com:6002`

### 6.3 HTTP virtual-host routing

Use when:

- you want domain-based routing instead of exposing a raw port
- several sites share one public HTTP/HTTPS entry

Server:

```toml
bind_addr = "0.0.0.0"
bind_port = 17000
vhost_http_port = 80
vhost_https_port = 443
subdomain_host = "example.com"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"

[transport.tls]
enable = true
cert_file = "/etc/arp/server.crt"
key_file = "/etc/arp/server.key"
```

Client with custom domain:

```toml
server_addr = "your.server.name"
server_port = 443
client_id = "web-node-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "websocket"

[transport.tls]
enable = true
trusted_ca_file = "/etc/arp/server.crt"
server_name = "your.server.name"

[[proxies]]
name = "app_http"
type = "http"
local_ip = "127.0.0.1"
local_port = 8080
custom_domains = ["app.example.com"]
```

Or use subdomain routing:

```toml
[[proxies]]
name = "app_http"
type = "http"
local_ip = "127.0.0.1"
local_port = 8080
subdomain = "app"
```

Reference examples:

- `examples/server_vhost_http.toml`
- `examples/server_vhost_https.toml`
- `examples/client_http_custom_domain.toml`
- `examples/client_http_subdomain.toml`
- `examples/client_https_custom_domain.toml`

In that case the public address becomes:

- `http://app.example.com`
- or `https://app.example.com` when using HTTPS vhost entry

### 6.4 UDP service forwarding

Use when the local service is UDP-based.

Client proxy example:

```toml
[[proxies]]
name = "udp_demo"
type = "udp"
local_ip = "127.0.0.1"
local_port = 5353
remote_port = 6003
```

Notes:

- use a UDP-capable client application to verify
- server still enforces `allow_ports`

### 6.5 STCP / SUDP shared-secret exposure

Use when:

- the tunnel should be protected by a shared secret
- you want the provider side not to expose an open public raw proxy in the normal way

Provider example:

```toml
[[proxies]]
name = "secure_ssh"
type = "stcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = 6004
sk = "replace_with_shared_secret"
```

Same idea applies to `sudp`:

```toml
[[proxies]]
name = "secure_udp"
type = "sudp"
local_ip = "127.0.0.1"
local_port = 5353
remote_port = 6005
sk = "replace_with_shared_secret"
```

## 7. XTCP NAT traversal scenario

Use when:

- both sides are behind NAT
- you want direct peer punching when possible
- relay fallback is acceptable when direct punching fails

Provider-side client:

```toml
server_addr = "your.server.name"
server_port = 17000
client_id = "xtcp-provider-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"

[[proxies]]
name = "xtcp_demo"
type = "xtcp"
local_ip = "127.0.0.1"
local_port = 22
sk = "xtcp_shared_secret"
fallback_to_relay = true
```

Visitor-side client:

```toml
server_addr = "your.server.name"
server_port = 17000
client_id = "xtcp-visitor-1"

[auth]
method = "token"
token = "replace_with_token"

[transport]
protocol = "tcp"

[[visitors]]
name = "xtcp_visitor"
type = "xtcp"
server_name = "xtcp_demo"
sk = "xtcp_shared_secret"
bind_addr = "127.0.0.1"
bind_port = 6001
fallback_to_relay = true
xtcp_punch_timeout_secs = 12
```

Reference examples:

- `examples/client_xtcp_provider.toml`
- `examples/client_xtcp_visitor.toml`

Behavior summary:

- visitor connects to its local `bind_addr:bind_port`
- client and visitor try direct NAT punching
- if direct path fails and `fallback_to_relay = true`, traffic falls back to relay TCP

## 8. TCP load-balancing group scenario

Use when multiple clients should serve one public TCP port.

Requirements:

- each backend uses `type = "tcp"` or `"stcp"`
- each backend must use the **same** fixed `remote_port`
- all backends in the same group use the same `load_balancer.group`

Example on multiple clients:

```toml
[[proxies]]
name = "api-node-a"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 8080
remote_port = 6010

[proxies.load_balancer]
group = "api-cluster"
group_key = "v1"
```

Another client:

```toml
[[proxies]]
name = "api-node-b"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 8080
remote_port = 6010

[proxies.load_balancer]
group = "api-cluster"
group_key = "v1"
```

Notes:

- `remote_port` must be fixed, not `0`
- backend health failures are temporarily ejected and later retried

## 9. TLS file rules

When using `transport.tls.enable = true` or `protocol = "quic"`:

Server side:

- `cert_file` and `key_file` must be set
- certificate SAN must match the hostname clients use

Client side:

- set `trusted_ca_file`
- set `server_name` when the server is accessed by IP or when explicit hostname matching is needed

## 10. Recommended protocol choices by scenario

- **local lab / simplest setup**: `tcp`
- **public internet default**: `wss`
- **performance-first and UDP is allowed**: `quic`
- **UDP-based control transport in controlled networks**: `kcp`
- **domain-based web publishing**: `http` / `https` proxy + vhost ports
- **NAT traversal**: `xtcp`

## 11. Common mistakes

- forgetting to replace `auth.token`
- choosing a `remote_port` outside `allow_ports`
- using `websocket + tls` or `quic` without valid cert/key/CA settings
- using an IP `server_addr` for TLS/QUIC but not setting `transport.tls.server_name`
- reusing the same `proxy.name` across different clients unintentionally
- configuring TCP load-balancing groups without a fixed `remote_port`
- expecting `http` / `https` proxies to work without `custom_domains` or `subdomain`

## 12. Recommended workflow

1. Start with `examples/server.toml` and `examples/client.toml`
2. Verify one simple `tcp` proxy first
3. Move to `wss` for public deployment
4. Add HTTP/HTTPS vhost or QUIC/KCP only after the base path is stable
5. Assign a clear `client_id` for each deployed client instance

## 13. Related files

- `README.md`
- `examples/server.toml`
- `examples/client.toml`
- `examples/server_ws.toml`
- `examples/client_ws.toml`
- `examples/server_prod_wss.toml`
- `examples/client_prod_wss.toml`
- `examples/server_kcp.toml`
- `examples/client_kcp.toml`
- `examples/server_quic.toml`
- `examples/client_quic.toml`
- `examples/client_xtcp_provider.toml`
- `examples/client_xtcp_visitor.toml`
- `examples/server_vhost_http.toml`
- `examples/server_vhost_https.toml`
- `examples/client_http_custom_domain.toml`
- `examples/client_http_subdomain.toml`
- `examples/client_https_custom_domain.toml`
