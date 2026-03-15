# ARP-Rust 开发文档

## 当前实现状态

### 已完成
- TCP 控制连接与工作连接模型
- TCP 代理转发
- UDP 代理转发
- HTTP 虚拟主机代理（按 `Host` 路由）
- HTTPS 虚拟主机代理（按 TLS `SNI` 路由）
- Token 认证

### 关键实现点（VHost）
- 服务端在 `vhost_http_port` 与 `vhost_https_port` 启动共享监听器。
- 客户端注册 `type = "http"` 或 `type = "https"` 时，服务端记录域名到代理实例的映射。
- HTTP 连接读取请求头，解析 `Host` 后转发到对应代理。
- HTTPS 连接读取 TLS ClientHello，解析 `SNI` 后转发到对应代理。

### 关键实现点（UDP）
- 服务端 `udp` 代理监听公网 UDP 端口。
- UDP 请求通过持久工作连接复用传输（UDP mux），避免每包建连。
- 客户端 `udp` 代理将报文发往本地 UDP 服务，并将响应报文回传。
- UDP 数据链路支持可选 `use_compression` 与 `use_encryption`（AES-GCM）。

### 关键实现点（连接池）
- 客户端根据 `transport.pool_count` 预热工作连接，并在连接消费后自动补充。
- 服务端在无待处理请求时缓存空闲工作连接，优先复用。

### 关键实现点（TCP/STCP 负载均衡）
- 同 `remote_port` + `load_balancer.group/group_key` 的 `tcp/stcp` 代理共享一个服务端监听端口。
- 服务端按轮询挑选分组后端并下发带 `proxy_name` 的 `ReqWorkConn`。
- 客户端在 `ReqWorkConn(proxy_name)` 阶段依据健康状态决定是否创建工作连接。
- 服务端对失败后端执行临时摘除（eject），窗口后自动恢复参与调度。

### 关键实现点（XTCP）
- `xtcp` provider 注册时不占用 `remote_port`，由 visitor 触发会话。
- visitor 发起 `NatHoleVisitor`，服务端校验 `sk` 并转发 `NatHoleClient` 给 provider。
- provider 创建临时 P2P 监听并回传 `NatHoleResp(client_addr)`，visitor 直连该地址后与 provider 本地服务透传。
- 当前实现包含 NAT 打洞协商与 best-effort UDP 打洞包；在受限 NAT 环境下可能仍需后续增强（如 KCP/QUIC 中继回退）。

### 关键实现点（WebSocket 传输）
- 当 `transport.protocol = "websocket"` 时，控制连接与工作连接均通过 WS 建立。
- 通过 `ws_stream` 桥接器把 WS 二进制帧映射为字节流，复用现有消息编解码与代理数据通道逻辑。
- 当前仅支持 `ws`（明文）；`wss` 将在后续阶段补齐。

### 关键实现点（管理接口）
- 服务端在 `dashboard_port > 0` 时启动管理接口。
- `GET /healthz` 返回服务健康状态。
- `GET /metrics` 输出 Prometheus 指标（控制连接、代理数、工作连接、TCP 字节数、tcp_mux 流计数等）。
- `GET /api/v1/status` 返回 JSON 状态信息。
- `GET /api/v1/proxies` 返回当前代理键列表。

### 关键实现点（TLS传输）
- 服务端在 `transport.tls.enable = true` 时加载证书和私钥，对控制连接和工作连接执行 TLS 握手。
- 客户端在 `transport.tls.enable = true` 时使用 `trusted_ca_file` 与 `server_name` 建立 TLS 连接。
- 传输层已改为通用异步流封装，可在同一消息编解码层兼容 TCP/TLS。

## 代码结构（与本次改动相关）

- 服务端 vhost 实现：
  `crates/arp-server/src/proxy/vhost.rs`
- 服务端代理管理：
  `crates/arp-server/src/proxy/mod.rs`
- 客户端代理类型选择：
  `crates/arp-client/src/proxy/mod.rs`
- 传输层切换到原始流时保留预读缓冲：
  `crates/arp-common/src/transport/mod.rs`

## 开发注意事项

- `MessageTransport` 切换到原始 `TcpStream` 时必须处理 codec 预读缓冲，否则可能丢失 `StartWorkConn` 后的首包数据。
- HTTP/HTTPS vhost 必须配置域名（`custom_domains` 或 `subdomain`）。
- `http/https` 客户端代理当前复用 TCP 本地转发逻辑。
