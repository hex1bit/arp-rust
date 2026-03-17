# Development Commands

## Core Commands

### Build
```bash
cargo build --workspace
cargo build --workspace --release
```
- `cargo build --workspace`：构建整个 workspace 的调试产物。
- `cargo build --workspace --release`：生成发布版本二进制，产物为 `target/release/arps` 与 `target/release/arpc`。

### Run binaries
```bash
./target/release/arps -c server.toml
./target/release/arpc -c client.toml
```
- 服务端二进制是 `arps`。
- 客户端二进制是 `arpc`。
- 配置文件使用 TOML；常用示例在 `examples/` 目录。

## Code Quality

### Format
```bash
cargo fmt --all
```
用于统一 workspace 内所有 crate 的格式，README 明确列为开发命令。

### Lint
```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
对整个 workspace、所有 target、所有 feature 运行 Clippy，并将 warning 视为失败。

## Unit Tests

### Run all unit tests
```bash
cargo test --workspace
```
覆盖协议编解码、认证、配置校验、压缩/解压、vhost Host/SNI 解析等测试。

### Alternative documented form
```bash
cargo test
```
`docs/TESTING.md` 中使用的是简写形式；在当前 workspace 中可等价理解为运行全量测试。

## End-to-End Tests

E2E 脚本位于 `test/`，通常会先 `cargo build`，再启动临时后端、`arps`、`arpc`，最后校验公网侧连通性。

### Core transport and proxy tests
```bash
bash test/test_e2e.sh
bash test/test_e2e_tcp_mux.sh
bash test/test_e2e_udp.sh
bash test/test_e2e_vhost.sh
bash test/test_e2e_tls.sh
bash test/test_e2e_ws.sh
bash test/test_e2e_wss.sh
bash test/test_e2e_kcp.sh
bash test/test_e2e_quic.sh
bash test/test_e2e_stcp_sudp.sh
bash test/test_e2e_tcp_lb_health.sh
bash test/test_e2e_xtcp.sh
bash test/test_e2e_health.sh
```

### What each script covers
- `test/test_e2e.sh`：基础 TCP 控制连接、代理注册、工作连接、数据转发、管理接口。
- `test/test_e2e_tcp_mux.sh`：`tcp_mux` 并发多连接场景。
- `test/test_e2e_udp.sh`：UDP 持久隧道、报文往返、压缩与加密链路。
- `test/test_e2e_vhost.sh`：HTTP Host 路由与 HTTPS SNI 路由。
- `test/test_e2e_tls.sh`：TLS 控制连接与工作连接。
- `test/test_e2e_ws.sh`：WebSocket 传输。
- `test/test_e2e_wss.sh`：WSS 传输。
- `test/test_e2e_kcp.sh`：KCP 传输。
- `test/test_e2e_quic.sh`：QUIC 传输。
- `test/test_e2e_stcp_sudp.sh`：STCP/SUDP 安全代理与密钥校验。
- `test/test_e2e_tcp_lb_health.sh`：TCP 分组负载均衡与健康检查摘除/恢复。
- `test/test_e2e_xtcp.sh`：XTCP NAT 打洞主流程与 `sk` 错误场景。
- `test/test_e2e_health.sh`：健康检查相关验证。

## Common Local Workflows

### Minimal local TCP flow
```bash
cargo build --workspace
./target/debug/arps -c examples/server.toml
./target/debug/arpc -c examples/client.toml
```
适合先验证基础 TCP 代理链路，再扩展到其他 transport 或 proxy type。

### WSS-oriented flow
```bash
bash examples/gen_self_signed_cert.sh localhost 127.0.0.1
./target/debug/arps -c examples/server_prod_wss.toml
./target/debug/arpc -c examples/client_prod_wss.toml
```
README 将 `wss` 视为首次公网部署的推荐默认选项。

### QUIC-oriented flow
```bash
./target/debug/arps -c examples/server_quic.toml
./target/debug/arpc -c examples/client_quic.toml
```
适合在已确认 UDP 可达的环境中验证 QUIC 通道。

## Helpful Files
- `README.md`：构建、运行、部署建议与传输选型。
- `examples/*.toml`：按 transport/proxy 场景拆分的示例配置。
- `examples/gen_self_signed_cert.sh`：本地 TLS/WSS 测试证书生成脚本。
- `test/support/tcp_backend_stub.rs`：E2E 支撑测试程序。
