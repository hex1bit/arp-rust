# Testing

## Test Stack

项目测试分为两层：
- Rust 内嵌单元测试：使用 `#[test]` 与 `#[tokio::test]`
- Shell 端到端测试：位于 `test/`，通过脚本拉起 `arps`、`arpc` 和临时后端进行验证

单元测试更偏协议、配置、加密与解析逻辑；E2E 测试覆盖实际链路与不同 transport/proxy 组合。

## Run Tests

### All Rust tests
```bash
cargo test --workspace
```
`docs/TESTING.md` 中也使用 `cargo test` 作为简写。

### Common E2E entry points
```bash
bash test/test_e2e.sh
bash test/test_e2e_vhost.sh
bash test/test_e2e_udp.sh
bash test/test_e2e_tls.sh
bash test/test_e2e_ws.sh
bash test/test_e2e_wss.sh
bash test/test_e2e_kcp.sh
bash test/test_e2e_quic.sh
bash test/test_e2e_stcp_sudp.sh
bash test/test_e2e_tcp_mux.sh
bash test/test_e2e_tcp_lb_health.sh
bash test/test_e2e_xtcp.sh
bash test/test_e2e_health.sh
```

## Unit Test Locations

当前内嵌测试主要分布在以下文件：
- `crates/arp-common/src/auth/mod.rs`
- `crates/arp-common/src/config/mod.rs`
- `crates/arp-common/src/crypto/mod.rs`
- `crates/arp-common/src/protocol/codec.rs`
- `crates/arp-common/src/transport/mod.rs`
- `crates/arp-server/src/proxy/vhost.rs`

### What unit tests cover
- 认证逻辑：token 校验
- 配置校验：server/client/proxy 配置约束
- 编解码：协议消息 encode/decode
- 压缩与加密：compress/decompress、encrypt/decrypt
- vhost 解析：HTTP Host / HTTPS SNI
- 传输层：消息发送接收与流包装行为

## E2E Test Layout

E2E 脚本集中在 `test/`：
- `test_e2e.sh`：基础 TCP 主流程
- `test_e2e_health.sh`：健康相关验证
- `test_e2e_kcp.sh`：KCP 传输
- `test_e2e_quic.sh`：QUIC 传输
- `test_e2e_stcp_sudp.sh`：STCP/SUDP
- `test_e2e_tcp_lb_health.sh`：TCP 负载均衡与健康检查
- `test_e2e_tcp_mux.sh`：TCP 多路复用
- `test_e2e_tls.sh`：TLS 传输
- `test_e2e_udp.sh`：UDP 持久隧道
- `test_e2e_vhost.sh`：HTTP/HTTPS 虚拟主机
- `test_e2e_ws.sh`：WebSocket
- `test_e2e_wss.sh`：WSS
- `test_e2e_xtcp.sh`：XTCP NAT 打洞

测试支撑代码：
- `test/support/tcp_backend_stub.rs`

## Typical E2E Pattern

多数脚本遵循相似流程：
1. `cargo build`
2. 启动临时本地后端（例如 `nc` 或测试 stub）
3. 生成或写入临时 server/client 配置
4. 启动 `arps`
5. 启动 `arpc`
6. 从公网端口、vhost 入口或 visitor 侧发起请求
7. 校验响应、日志或管理接口
8. 通过 `trap` 做清理

这意味着脚本运行对本机端口、临时文件和进程清理比较敏感。

## What Each E2E Group Verifies

### Basic TCP
`test/test_e2e.sh` 验证：
- 控制连接建立
- TCP 代理注册
- 工作连接建立
- 端到端数据转发
- `/healthz`、`/metrics`、`/api/v1/status`、`/api/v1/proxies` 可访问

### Virtual host
`test/test_e2e_vhost.sh` 验证：
- HTTP 按 `Host` 路由
- HTTPS 按 TLS `SNI` 路由
- 目标客户端本地服务能返回响应

### UDP
`test/test_e2e_udp.sh` 验证：
- UDP 报文经公网端口进入客户端本地服务
- 响应报文能回传到请求方
- 开启 `use_compression` 与 `use_encryption` 后链路仍可工作

### TLS / WS / WSS / KCP / QUIC
这些脚本分别验证在不同 transport 下：
- 控制连接能建立
- 工作连接能建立
- 实际代理转发链路能跑通

### tcp_mux and load balancing
- `test/test_e2e_tcp_mux.sh`：验证多路复用并发连接场景
- `test/test_e2e_tcp_lb_health.sh`：验证同组后端轮询、失败摘除、窗口后恢复

### Secure proxy and NAT traversal
- `test/test_e2e_stcp_sudp.sh`：验证带共享密钥的安全代理
- `test/test_e2e_xtcp.sh`：验证 XTCP provider/visitor 协商与错误 `sk` 场景

## Test Conventions

### Naming
- Rust 单元测试以内嵌模块方式放在对应实现文件中，而不是单独 `tests/` crate。
- Shell E2E 统一采用 `test/test_e2e*.sh` 命名，名称直接体现验证特性。

### Config-driven tests
很多 E2E 测试依赖临时 TOML 配置，说明项目的重要行为都是配置驱动的。排查失败时，先看脚本里生成的 server/client 配置，再看日志。

## Debugging Failed Tests

常见排查点来自现有 `docs/TESTING.md` 与 `docs/DEVELOPMENT.md`：
- 端口冲突：先清理历史 `arps/arpc` 进程
- 首包丢失：检查 `MessageTransport` 切换原始流时是否保留预读缓冲
- 域名无路由：确认 `custom_domains`/`subdomain` 与请求中的 `Host`/`SNI` 一致
- TCP 分组注册失败：确认同组代理使用固定 `remote_port`

### Useful env vars
```bash
RUST_LOG=debug cargo test --workspace
RUST_BACKTRACE=1 cargo test --workspace
```
当需要查看 transport、握手或代理注册细节时，优先开启日志与回溯。
