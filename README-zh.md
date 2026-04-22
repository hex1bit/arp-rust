# ARP-Rust

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

ARP-Rust 中的 ARP 代表 **Advance Reverse Proxy**（高级反向代理）。

ARP-Rust 是一个用 Rust 编写的反向代理与隧道工具。它采用 client/server 模型，将私有网络中的 TCP、UDP、HTTP 和 HTTPS 服务暴露到公网，支持多种传输后端：TCP、TLS、KCP、QUIC、WebSocket 以及 WebSocket over TLS（wss）。

## 当前状态

ARP-Rust 已可用于真实的客户端/服务端转发场景，包括公网部署。目前验证最完整的传输路径：

- `tcp`
- `tcp + tls`
- `ws`
- `wss`
- `quic`

首次公网生产部署推荐使用 `wss`，它在兼容性、加密和部署难度上取得了最佳平衡。

## 为什么选择 ARP-Rust

ARP-Rust 不只是个人工具，更面向团队和生产环境设计：

- **动态代理管理** — 通过 REST API 或 `SIGHUP` 运行时增删代理，无需重启
- **内置诊断工具** — `arpc status` / `arpc check` 一键排查问题
- **安全优先** — HMAC 签名认证、结构化审计日志、防重放保护
- **生产级运维** — 优雅退出（连接排水）、配置热重载
- **多租户隔离** — 按 Token 设置连接数上限、带宽上限、端口/域名限制
- **实时可观测性** — 按代理维度指标、Prometheus 端点、SSE 事件流
- **多传输后端** — TCP、TLS、KCP、QUIC、WebSocket、WSS
- **多路复用优先架构** — TCP 多路复用默认开启，降低连接开销
- 零 unsafe 代码的紧凑 Rust 实现

## 典型用途

- 将私网机器的 SSH 通过公网服务器暴露出去
- 通过 HTTP/HTTPS 虚拟主机路由发布内部 Web 服务
- 将私有数据库或缓存端口转发出去供受控远程访问
- 从家庭实验室或办公网络暴露自定义 TCP/UDP 服务
- 将多节点 TCP 服务聚合到一个公网端口并做负载均衡
- 测试 `xtcp` NAT 穿透流程

## 生产部署注意事项

将 ARP-Rust 部署到公网前，请确认以下各项：

- 使用 `wss` 或 `tcp + tls`，不要用明文 `tcp`
- 将 `auth.token` 改为真实密钥（不要用示例中的占位符）
- 生成包含正确 `subjectAltName` 的服务端证书
- 保持 `transport.tls.server_name` 与证书域名一致
- 防火墙只开放控制端口和必要的 `allow_ports` 端口范围
- 先用单个 TCP 代理验证联通性，再扩展到更多传输协议或代理类型
- 开启服务端 `dashboard_port` 用于监控和 `/metrics` 指标采集
- 客户端配置 `admin_port` 用于动态管理和诊断
- 多租户场景使用 `auth.rules` 配置 `max_connections` 和 `bandwidth_limit_bytes`

## 功能特性

- 基于 Tokio 的异步运行时
- TCP 代理转发
- UDP 代理转发与持久 UDP 隧道
- HTTP/HTTPS 虚拟主机路由
- STCP / SUDP 支持
- XTCP NAT 穿透流程
- TCP 负载均衡组
- 基于健康检查的后端摘除/恢复
- TLS 传输
- KCP 传输
- QUIC 传输
- WebSocket 传输（ws / wss）
- **动态代理管理** — 通过 REST API 或 SIGHUP 运行时增删代理，无需重启
- **诊断 CLI** — `arpc status` 和 `arpc check` 检查连接和代理健康状态
- **HMAC-SHA256 认证** — Token 不再明文传输，附带时间戳签名并强制防重放窗口
- **结构化审计日志** — JSON 格式记录登录、断开、代理注册/拒绝等事件
- **优雅退出** — 退出前排水活跃连接（30 秒超时）
- **配置热重载** — `kill -HUP` 重新加载配置，增删代理无需重启
- **按代理统计指标** — 每个代理的入/出字节、活跃/总连接数、错误数均在 `/metrics` 暴露
- **SSE 事件流** — `GET /api/v1/events/stream` 实时监控服务端事件
- **多租户运行时限制** — 按 Token 设置 `max_connections` 和 `bandwidth_limit_bytes`
- **会话级 AES 加密** — STCP/SUDP 避免逐包密钥派生
- **多路复用优先架构** — TCP 类型代理默认开启 TCP 多路复用
- 服务端健康、指标、代理状态管理端点
- 基于 TOML 的配置文件
- 日志文件输出（按天轮转、自动清理旧文件，通过 `log_file`、`log_max_days` 配置）

## 工作区结构

```text
arp-rust/
├── crates/
│   ├── arp-common/     # 公共类型：配置、传输层、协议消息
│   ├── arp-server/     # 服务端二进制 (arps)
│   └── arp-client/     # 客户端二进制 (arpc)
├── examples/           # 各场景配置示例
├── docs/               # 文档（配置指南、开发说明）
└── test/               # E2E 测试脚本
```

## 构建

本机构建：

```bash
cargo build --workspace --release
```

产出二进制：

- `target/release/arps`
- `target/release/arpc`

在 macOS/Linux 上通过 Docker 一键构建 Linux 二进制：

```bash
bash scripts/build-linux.sh
```

产出文件：

- `dist/linux-x86_64/arps`
- `dist/linux-x86_64/arpc`
- `dist/arp-rust-linux-x86_64.tar.gz`

## 快速上手

### 服务端配置（server.toml）

```toml
bind_addr = "0.0.0.0"
bind_port = 17000

# 管理面板（可选）
dashboard_addr = "0.0.0.0"
dashboard_port = 17500

[auth]
method = "token"
token = "替换为你的密钥"

[transport]
protocol = "tcp"
tcp_mux = true

[[allow_ports]]
start = 6001
end = 7000
```

### 客户端配置（client.toml）

```toml
server_addr = "server.example.com"
server_port = 17000

# 客户端管理 API（可选，用于动态管理和诊断）
admin_addr = "127.0.0.1"
admin_port = 7400

[auth]
method = "token"
token = "替换为你的密钥"

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

### 启动

```bash
./arps -c server.toml
./arpc -c client.toml
```

### 诊断

```bash
arpc status -c client.toml      # 查看连接状态和代理列表
arpc check ssh -c client.toml   # 诊断 ssh 代理
```

### 连接测试

```bash
ssh user@server.example.com -p 6001
```

> 上面的快速上手使用明文 TCP，适合本地验证。公网生产环境请参考下方的 WSS 部署流程。

---

## 代理类型

`[[proxies]].type` 支持的值：

| 类型 | 用途 |
|------|------|
| `tcp` | SSH、数据库、自定义 TCP 服务、游戏服务器等 |
| `udp` | DNS 类负载、自定义 UDP 协议等 |
| `http` | 按 `Host` 头做虚拟主机路由的 HTTP 服务 |
| `https` | 按 TLS `SNI` 做虚拟主机路由的 HTTPS 服务 |
| `stcp` | 共享密钥保护的 TCP 代理 |
| `sudp` | 共享密钥保护的 UDP 代理 |
| `xtcp` | NAT 穿透（P2P 打洞 + 中继降级） |

---

## 传输协议

`transport.protocol` 支持的值：

| 协议 | 特点 | 适用场景 |
|------|------|---------|
| `tcp` | 最简单稳定 | 本地实验室、受控内网 |
| `websocket` | 兼容性最佳（+TLS 即为 wss） | **公网生产首选** |
| `quic` | UDP+TLS，低延迟高并发 | UDP 可用时的高性能部署 |
| `kcp` | UDP 传输 | 受控网络 UDP 场景 |

TLS 可在 TCP 和 WebSocket 之上通过 `transport.tls.enable = true` 开启。

**选择建议**：
- 最简/最稳：`tcp`（仅限内网）
- 公网生产安全默认：`wss`
- 高性能且 UDP 可用：`quic`
- 选 `wss` 作为首选，`quic` 作为性能优先备选

---

## 推荐部署路径

1. 先在受控网络用明文 `tcp` 验证基础联通
2. 公网部署前切换到 `tcp + tls` 或 `wss`
3. 需要兼容反向代理、443 端口或受限网络时选 `wss`
4. 确认 UDP 可达后再考虑 `quic`

---

## WSS vs QUIC 对比

| 维度 | WSS | QUIC |
|------|-----|------|
| 底层协议 | TCP + TLS + WebSocket | UDP + 内置 TLS |
| 反向代理兼容性 | 优秀（标准 HTTPS 流量形态） | 较差（依赖 UDP） |
| 握手延迟 | 较高 | 较低 |
| 多路复用 | 通过 tcp_mux 实现 | 原生支持 |
| 网络依赖 | TCP 即可 | 需要 UDP 可达 |

---

## 配置示例文件

详细场景配置指南见 `docs/配置指南.md`（中文）或 `docs/CONFIGURATION_GUIDE.md`（英文）。

通用：
- `examples/server.toml`、`examples/client.toml`

WebSocket / WSS：
- `examples/server_ws.toml`、`examples/client_ws.toml`
- `examples/server_prod_wss.toml`、`examples/client_prod_wss.toml`

KCP / QUIC：
- `examples/server_kcp.toml`、`examples/client_kcp.toml`
- `examples/server_quic.toml`、`examples/client_quic.toml`

XTCP NAT 穿透：
- `examples/client_xtcp_provider.toml`、`examples/client_xtcp_visitor.toml`

HTTP/HTTPS vhost：
- `examples/server_vhost_http.toml`、`examples/server_vhost_https.toml`
- `examples/client_http_custom_domain.toml`
- `examples/client_http_subdomain.toml`
- `examples/client_https_custom_domain.toml`

---

## TLS / WSS 证书

使用 `transport.tls.enable = true` 时，服务端证书需满足：

- 是服务端证书（不是 `CA:TRUE`）
- `subjectAltName` 中包含真实域名或 IP
- 与客户端 `transport.tls.server_name` 一致
- 通过客户端 `trusted_ca_file` 信任

生成自签名证书：

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

或使用内置脚本：

```bash
cd examples
bash gen_self_signed_cert.sh your.server.name 1.2.3.4
```

---

## WSS 部署步骤

1. 生成证书和私钥：

```bash
cd examples
bash gen_self_signed_cert.sh your.server.name 1.2.3.4
```

2. 将证书复制到服务器：

```bash
sudo mkdir -p /etc/arp
sudo cp server.crt /etc/arp/server.crt
sudo cp server.key /etc/arp/server.key
sudo chmod 600 /etc/arp/server.key
```

3. 参考 `examples/server_prod_wss.toml` 准备服务端配置
4. 参考 `examples/client_prod_wss.toml` 准备客户端配置
5. 启动并验证：

```bash
./arps -c examples/server_prod_wss.toml
./arpc -c examples/client_prod_wss.toml
nc -vz your.server.name 6001
```

如果 `nc` 成功但应用流量失败，请检查：
- 客户端 `local_ip` / `local_port` 是否正确
- 服务器防火墙/云安全组是否放行对应端口
- 证书域名是否与 `server_name` 匹配
- 所选 `remote_port` 是否已被其他进程占用

---

## 管理端点

### 服务端 Dashboard

配置 `dashboard_addr` 和 `dashboard_port` 后可用：

- `GET /` — HTML 管理面板
- `GET /healthz` — 健康检查
- `GET /readyz` — 就绪检查
- `GET /metrics` — Prometheus 指标（全局 + 按代理）
- `GET /api/v1/status` — 服务端状态 JSON
- `GET /api/v1/proxies` — 已注册代理列表
- `GET /api/v1/proxies/:name` — 代理详情
- `GET /api/v1/clients` — 已连接客户端列表
- `GET /api/v1/clients/:run_id` — 客户端详情
- `POST /api/v1/clients/:run_id/shutdown` — 强制断开客户端
- `GET /api/v1/xtcp/events` — 最近 XTCP NAT 穿透事件
- `GET /api/v1/events/stream` — SSE 实时事件流

### 客户端管理 API

配置 `admin_addr` 和 `admin_port` 后可用：

- `GET /api/v1/status` — 客户端连接状态
- `GET /api/v1/proxies` — 本地代理列表
- `GET /api/v1/proxies/:name` — 代理详情
- `POST /api/v1/proxies` — 动态添加代理（JSON body）
- `DELETE /api/v1/proxies/:name` — 动态删除代理
- `POST /api/v1/reload` — 重新加载配置文件，差量应用变更

设置 `admin_user` 和 `admin_pwd` 后强制 Basic Auth。

### 客户端 CLI 命令

```bash
arpc run -c client.toml          # 启动客户端（默认）
arpc status -c client.toml       # 查看连接状态和代理列表
arpc check -c client.toml        # 诊断所有代理
arpc check ssh -c client.toml    # 诊断指定代理
```

---

## 配置热重载

客户端支持不重启的代理配置热重载：

```bash
# 通过信号（Unix）
kill -HUP $(pidof arpc)

# 通过 API
curl -X POST http://127.0.0.1:7400/api/v1/reload
```

新增代理自动注册，删除代理自动注销，已有连接不中断。

---

## 优雅退出

服务端和客户端均支持 `Ctrl+C` / `SIGTERM` 优雅退出：

- **服务端**：停止接受新连接，等待活跃连接排水（最多 30 秒）
- **客户端**：正常断开与服务端的连接

---

## 安全

### HMAC 认证

Token 不以明文传输。客户端对 `HMAC-SHA256(token, timestamp)` 签名，服务端验证签名，强制 5 分钟防重放窗口。

### 审计日志

服务端通过 `tracing` 输出结构化 JSON 审计事件：

- `client_login` / `client_login_failed`
- `client_disconnect`
- `proxy_registered` / `proxy_rejected` / `proxy_closed`
- `work_conn_auth_failed`

### 多租户运行时限制

```toml
[[auth.rules]]
token = "team-a"
max_connections = 50
bandwidth_limit_bytes = 10485760  # 10 MB/s
allow_ports = [{ start = 6000, end = 6100 }]
```

---

## 测试

运行完整测试套件（单元测试 + 所有 E2E）：

```bash
bash scripts/test-full.sh
```

各 E2E 场景也可在 `test/` 目录下单独运行。

---

## 开发

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

详细开发文档见 `docs/开发说明.md`（中文）或 `docs/DEVELOPMENT.md`（英文）。

---

## License

MIT
