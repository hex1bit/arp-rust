# ARP-Rust

[English README](README_EN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

> 使用Rust语言实现的高性能内网穿透工具

ARP-Rust 中的 `ARP` 指 `Advance Reverse Proxy`。项目使用 Rust 实现，目标是提供快速、安全、高性能的反向代理能力，用于将本地服务安全暴露到公网。

## ✨ 特性

- 🚀 **高性能**: 基于Tokio异步运行时，充分利用Rust的零成本抽象
- 🔒 **内存安全**: Rust的所有权系统确保无内存泄漏和数据竞争
- 📦 **轻量级**: 单二进制文件，无运行时依赖，体积小
- 🔐 **安全认证**: 支持Token认证
- 🌐 **TCP代理**: 支持TCP端口映射
- 📡 **UDP代理**: 支持UDP端口映射与回包
- ⚡ **UDP持久隧道**: UDP请求复用单条工作连接（减少每包建连开销）
- 🌍 **HTTP/HTTPS虚拟主机**: 支持基于Host/SNI的域名路由
- 🔐 **数据安全（部分）**: UDP链路支持 `use_compression` + `use_encryption`
- 🔐 **STCP/SUDP（基础）**: 支持 `stcp` 与 `sudp` 代理类型（`sudp` 强制加密）
- 🕳️ **XTCP（NAT 打洞流程）**: 支持 provider/visitor 协商、打洞协商与点对点转发
- ⚖️ **TCP/STCP 负载均衡分组**: `load_balancer.group/group_key` 支持同端口分组轮询
- 🩺 **健康检查联动摘除/恢复**: 后端异常自动临时摘除，恢复后自动回流
- 🔒 **TLS传输**: 控制连接与工作连接支持TLS
- 🚄 **KCP 传输**: 支持 `transport.protocol = "kcp"`
- 🚀 **QUIC 传输**: 支持 `transport.protocol = "quic"`
- 🔌 **WebSocket 传输**: 支持 `transport.protocol = "websocket"`（控制/工作连接）
- 📈 **管理接口**: 支持 `/healthz`、`/metrics`、`/api/v1/status`、`/api/v1/proxies`
- 📝 **易于配置**: 使用TOML配置文件

## 🏗️ 项目架构

```
arp-rust/
├── crates/
│   ├── arp-common/     # 公共库 (协议、传输层、认证)
│   ├── arp-server/     # 服务端 (arps)
│   └── arp-client/     # 客户端 (arpc)
├── examples/           # 配置文件示例
└── README.md
```

### 核心组件

- **消息协议**: 基于JSON的消息格式，支持Login、NewProxy、StartWorkConn等消息类型
- **传输层**: 基于tokio-util的编解码器，支持消息序列化和反序列化
- **认证系统**: 可扩展的认证框架，当前支持Token认证
- **代理管理**: 动态代理注册和管理，支持TCP代理类型

## 🚀 快速开始

### 安装

#### 从源码编译

```bash
git clone https://github.com/hex1bit/arp-rust.git
cd arp-rust
cargo build --release
```

编译后的二进制文件位于 `target/release/`:
- `arps` - 服务端
- `arpc` - 客户端

### 配置

#### 服务端配置 (server.toml)

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

#### 客户端配置 (client.toml)

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

### 运行

#### 1. 启动服务端

```bash
./target/release/arps -c server.toml
```

#### 2. 启动客户端

```bash
./target/release/arpc -c client.toml
```

#### 3. 连接到内网服务

```bash
ssh user@server.example.com -p 6001
```

### XTCP 示例

服务提供端（暴露本地 22）可参考：
- `examples/client_xtcp_provider.toml`

访问端（本地监听 6001）可参考：
- `examples/client_xtcp_visitor.toml`

### KCP / QUIC 示例

- `examples/server_kcp.toml`
- `examples/client_kcp.toml`
- `examples/server_quic.toml`
- `examples/client_quic.toml`

## 📊 测试结果

### 基础功能测试

✓ 服务端启动正常  
✓ 客户端连接成功  
✓ 代理注册成功  
✓ TCP端口转发工作正常  
✓ UDP端口转发工作正常  
✓ HTTP虚拟主机路由正常  
✓ HTTPS SNI虚拟主机路由正常  
✓ 多连接并发处理正常  

### 测试日志示例

```
Server Log:
2026-03-13T17:21:50.414796Z  INFO arps::service: Client login from 127.0.0.1:60862
2026-03-13T17:21:50.415108Z  INFO arps::resource: Allocated TCP port: 6100
2026-03-13T17:21:50.415308Z  INFO arps::proxy::tcp: TCP proxy echo_test listening on 0.0.0.0:6100
2026-03-13T17:21:50.415340Z  INFO arps::proxy: Proxy registered successfully

Client Log:
2026-03-13T17:21:50.411218Z  INFO arpc::control: Connecting to server: 127.0.0.1:17000
2026-03-13T17:21:50.414996Z  INFO arpc::control: Login successful, run_id: 622efa5a-3f94-48c0-9329-241b988976eb
2026-03-13T17:21:50.415397Z  INFO arpc::control: Proxy echo_test registered successfully, remote address: 0.0.0.0:6100
```

## 🔧 开发

### 运行测试

```bash
# 单元测试
cargo test

# TCP端到端测试
bash test/test_e2e.sh

# TCP mux 并发端到端测试
bash test/test_e2e_tcp_mux.sh

# HTTP/HTTPS虚拟主机端到端测试
bash test/test_e2e_vhost.sh

# UDP端到端测试
bash test/test_e2e_udp.sh

# STCP/SUDP 端到端测试
bash test/test_e2e_stcp_sudp.sh

# TCP 负载均衡 + 健康联动端到端测试
bash test/test_e2e_tcp_lb_health.sh

# XTCP NAT 打洞端到端测试
bash test/test_e2e_xtcp.sh

# WebSocket 传输端到端测试
bash test/test_e2e_ws.sh

# TLS传输端到端测试
bash test/test_e2e_tls.sh

# KCP传输端到端测试
bash test/test_e2e_kcp.sh

# QUIC传输端到端测试
bash test/test_e2e_quic.sh
```

### 管理接口

服务端配置 `dashboard_addr` 与 `dashboard_port` 后可用：

- `GET /healthz` 健康检查
- `GET /metrics` 指标输出（包含连接/字节/tcp_mux流计数）
- `GET /api/v1/status` 服务状态（JSON）
- `GET /api/v1/proxies` 代理列表（JSON）

### 代码检查

```bash
# 格式化
cargo fmt

# Lint检查
cargo clippy

# 编译检查
cargo check
```

## 📖 技术栈

- **运行时**: [Tokio](https://tokio.rs/) - 异步运行时
- **序列化**: [Serde](https://serde.rs/) - 序列化/反序列化
- **配置解析**: [TOML](https://github.com/toml-rs/toml) - 配置文件格式
- **日志**: [Tracing](https://github.com/tokio-rs/tracing) - 结构化日志
- **命令行**: [Clap](https://github.com/clap-rs/clap) - 命令行参数解析
- **并发**: [DashMap](https://github.com/xacrimon/dashmap) - 并发HashMap

## 🗺️ 路线图

### 已完成 ✅
- [x] 基础架构设计
- [x] 消息协议实现
- [x] TCP代理支持
- [x] Token认证
- [x] 服务端/客户端基础功能
- [x] 端口资源管理
- [x] E2E测试验证
- [x] HTTP/HTTPS虚拟主机支持
- [x] UDP代理支持
- [x] TLS加密传输
- [x] 健康检查
- [x] 负载均衡（TCP/STCP 分组 + VHost 多后端轮询）
- [x] XTCP NAT 打洞基础流程（服务提供端/访问端）

### 计划中 📋
- [ ] WebSocket over TLS (`wss`)

## 🤝 贡献

欢迎贡献代码、报告问题或提出新功能建议！

## 📄 许可证

本项目采用 MIT 许可证 - 详见 [LICENSE](LICENSE) 文件

## 🙏 致谢

- Rust社区的所有贡献者

## 📞 联系方式

- 项目地址: https://github.com/hex1bit/arp-rust
- Issue追踪: https://github.com/hex1bit/arp-rust/issues

---

**注意**: 这是一个学习项目，用于探索Rust在网络编程领域的应用。如需生产环境使用，请进行充分测试。
