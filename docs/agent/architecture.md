# Architecture

## Workspace Overview

项目是一个 Rust workspace，由三个 crate 组成：
- `crates/arp-common`：共享协议、配置、传输、认证、加密能力
- `crates/arp-server`：服务端二进制 `arps`
- `crates/arp-client`：客户端二进制 `arpc`

根 `Cargo.toml` 统一管理 workspace 依赖与版本，核心运行时是 Tokio，管理接口基于 Axum。

## Structure Philosophy

代码结构围绕“共享协议层 + 服务端控制面 + 客户端执行面”展开：
- `arp-common` 提供两端共享的数据模型与传输抽象，避免 server/client 各自维护一套协议实现。
- `arp-server` 负责接入公网流量、管理控制连接、调度工作连接、暴露管理接口。
- `arp-client` 负责连回服务端、注册代理、把服务端请求转发到本地服务。

这种分层使新增 transport、proxy type 或消息类型时，通常先改 `arp-common`，再在 server/client 两侧接入。

## Shared Layer: `arp-common`

`crates/arp-common/src/lib.rs` 暴露以下模块：
- `auth`：认证实现，目前以 token 模式为主
- `config`：服务端、客户端、代理、visitor 配置定义与校验
- `crypto`：压缩与加密能力
- `error`：统一错误类型
- `protocol`：消息定义与编解码
- `transport`：多种底层连接的统一抽象

### Configuration model
配置使用 TOML，关键类型在 `crates/arp-common/src/config/mod.rs`：
- `ServerConfig`
- `ClientConfig`
- `ProxyConfig`
- `VisitorConfig`

配置对象在加载后会执行校验，重点约束包括：
- server/client 基础地址与端口是否合法
- transport 与 TLS 配置是否一致
- `http`/`https` 代理必须配置 `custom_domains` 或 `subdomain`
- 某些代理场景需要固定 `remote_port`

### Protocol model
协议层在 `crates/arp-common/src/protocol/`：
- `message.rs`：消息枚举与结构体
- `codec.rs`：消息编解码

消息覆盖登录、代理注册、工作连接请求、UDP 数据包、NAT 打洞等控制面场景。服务端与客户端都通过该统一消息协议通信。

### Transport abstraction
传输抽象位于 `crates/arp-common/src/transport/mod.rs`。

核心思路：把不同底层通道统一成可异步读写的流，再包装成消息传输层。
- `AsyncStream`：统一异步读写 trait 抽象
- `BoxedStream`：对具体流类型做 trait object 封装
- `MessageTransport`：在通用流之上提供消息发送/接收能力

已接入的底层通道包括：
- TCP
- TLS（基于 `tokio-rustls`）
- WebSocket / WSS（`ws_stream.rs` 负责把二进制帧桥接成字节流）
- QUIC（`quic_stream.rs`）
- KCP

`mux.rs` 提供 TCP 多路复用帧协议；`udp_mux.rs` 支持 UDP 持久隧道的多报文复用。

## Server Architecture: `arp-server`

服务端入口是 `crates/arp-server/src/service.rs`，职责包括：
- 加载配置
- 初始化监听器与依赖组件
- 接受客户端控制连接
- 启动 HTTP 管理接口

### Control plane
控制面核心分布在：
- `crates/arp-server/src/control/mod.rs`
- `crates/arp-server/src/service.rs`

职责包括：
- 接收 `Login`、`NewProxy`、`CloseProxy`、`Ping` 等消息
- 管理客户端会话
- 在需要时向客户端发送 `ReqWorkConn`
- 协调 NAT 打洞流程相关消息

### Proxy management
代理相关代码位于 `crates/arp-server/src/proxy/`：
- `tcp.rs`：TCP / STCP 转发、分组负载均衡、健康检查
- `udp.rs`：UDP / SUDP 转发与持久工作连接
- `vhost.rs`：HTTP/HTTPS 虚拟主机路由
- `mod.rs`：统一管理代理实例注册与分发

关键模式：
- 服务端持有公网监听端口或共享监听器
- 当有外部请求到达时，请求对应客户端建立 work connection
- work connection 建立后，服务端把公网流量与客户端本地服务之间做双向转发

### Virtual host routing
`crates/arp-server/src/proxy/vhost.rs` 负责虚拟主机模式：
- HTTP：读取请求头并解析 `Host`
- HTTPS：读取 TLS ClientHello 并解析 SNI
- 然后把请求路由到对应客户端代理

这使多个站点可以共享同一组 `vhost_http_port` / `vhost_https_port`。

### Resource and admin surface
- `crates/arp-server/src/resource/mod.rs`：资源控制与端口范围限制
- `crates/arp-server/src/metrics.rs`：控制连接、工作连接、TCP 字节数等指标
- `crates/arp-server/src/web/mod.rs`：管理接口，包含 `/healthz`、`/metrics`、`/api/v1/status`、`/api/v1/proxies`

### NAT traversal
`crates/arp-server/src/nathole.rs` 实现 XTCP 的 NAT 打洞协调逻辑：
- visitor 向服务端发起访问
- 服务端把打洞请求转给 provider
- provider 建立临时 P2P 监听并回传信息
- visitor 尝试直连 provider

## Client Architecture: `arp-client`

客户端入口是 `crates/arp-client/src/service.rs`，主要职责：
- 读取客户端配置
- 建立到服务端的控制连接
- 注册本地代理
- 响应服务端的工作连接请求

### Control loop
`crates/arp-client/src/control/mod.rs` 负责：
- 登录与保活
- 注册 `NewProxy`
- 响应 `ReqWorkConn`
- 创建并初始化工作连接
- 处理 UDP、XTCP 等特殊消息

### Local proxy execution
`crates/arp-client/src/proxy/` 按代理类型拆分：
- `tcp.rs`
- `udp.rs`
- `xtcp.rs`
- `mod.rs` 负责统一选择与组织

客户端侧代理的本质是把服务端下发的流量转到本地 `local_ip/local_port` 服务，或在 XTCP 模式下扮演 provider / visitor 角色。

## Connection Model

### Control connection
控制连接是 client/server 之间的持久连接，承担控制面协议：
- 建立登录会话
- 注册或关闭代理
- 心跳保活
- 请求新的工作连接
- 传输 UDP/NAT 打洞相关控制消息

控制连接可以运行在 TCP、TLS、WebSocket/WSS、QUIC、KCP 之上。

### Work connection
工作连接是为单个代理请求动态建立的数据通道：
- 服务端收到公网访问
- 服务端向客户端发送 `ReqWorkConn`
- 客户端建立新的 work connection
- 双方完成 `NewWorkConn` / `StartWorkConn` 一类握手
- 后续进入实际数据透传阶段

`transport.pool_count` 支持预热工作连接池，减少实时建连开销。

## Data Flow Patterns

### Standard TCP/UDP proxy flow
1. 客户端先通过控制连接登录并注册代理。
2. 外部用户访问服务端公网端口或 vhost 入口。
3. 服务端根据命中的代理向客户端请求工作连接。
4. 客户端建立 work connection 并绑定到对应代理。
5. 服务端与客户端执行双向数据转发。

### UDP persistent tunnel
UDP 模式不是每个报文建一条连接，而是通过持久工作连接复用多条 UDP 消息。这降低了频繁建连的成本，也为压缩/加密提供了统一承载通道。

### TCP load balancing
同一 `remote_port` 下、同一 `load_balancer.group/group_key` 的 `tcp/stcp` 代理会共享服务端监听端口：
- 服务端按轮询选择后端
- 健康检查失败的后端会被临时摘除
- 窗口期后可恢复参与调度

## Configuration and Examples

项目把不同 transport 与部署方式拆成独立示例：
- `examples/server.toml` / `examples/client.toml`：基础 TCP
- `examples/server_prod_wss.toml` / `examples/client_prod_wss.toml`：WSS
- `examples/server_quic.toml` / `examples/client_quic.toml`：QUIC
- `examples/server_kcp.toml` / `examples/client_kcp.toml`：KCP
- `examples/client_xtcp_provider.toml` / `examples/client_xtcp_visitor.toml`：XTCP

README 的建议是：先从基础 TCP 跑通，再迁移到 `tcp + tls` 或 `wss`，最后按网络条件选择 `quic`。

## Key Files to Read First
- `Cargo.toml`
- `README.md`
- `crates/arp-common/src/config/mod.rs`
- `crates/arp-common/src/protocol/mod.rs`
- `crates/arp-common/src/transport/mod.rs`
- `crates/arp-server/src/service.rs`
- `crates/arp-server/src/proxy/mod.rs`
- `crates/arp-client/src/control/mod.rs`
