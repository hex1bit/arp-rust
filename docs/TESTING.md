# ARP-Rust 测试文档

## 1. 单元测试

```bash
cargo test
```

覆盖内容包括：
- 协议编解码
- 认证
- 配置校验（含 HTTP/HTTPS 代理约束）
- 压缩与解压
- vhost `Host` / `SNI` 解析

## 2. TCP 端到端测试

```bash
bash test/test_e2e.sh
```

验证点：
- 控制连接建立
- TCP 代理注册
- 工作连接建立
- 端到端数据转发
- 管理接口 `/healthz`、`/metrics`、`/api/v1/status`、`/api/v1/proxies` 可访问
- `test/test_e2e_tcp_mux.sh` 覆盖 tcp_mux 并发多连接场景
- `test/test_e2e_stcp_sudp.sh` 覆盖 stcp/sudp 类型与密钥校验场景
- `test/test_e2e_udp.sh` 当前覆盖 UDP 持久隧道模式（单 work conn 复用）
- `test/test_e2e_tcp_lb_health.sh` 覆盖 TCP 分组负载均衡与健康检查摘除/恢复联动
- `test/test_e2e_xtcp.sh` 覆盖 XTCP 服务提供端/访问端 NAT 打洞协商主流程与 `sk` 错误场景
- `test/test_e2e_ws.sh` 覆盖 WebSocket 传输下控制连接、工作连接和 TCP 转发链路
- `test/test_e2e_wss.sh` 覆盖 WSS 传输下控制连接、工作连接和 TCP 转发链路

## 3. HTTP/HTTPS 虚拟主机端到端测试

```bash
bash test/test_e2e_vhost.sh
```

验证点：
- HTTP `Host` 路由到目标客户端代理
- HTTPS `SNI` 路由到目标客户端代理
- 客户端本地服务可收到并返回响应

## 4. UDP 端到端测试

```bash
bash test/test_e2e_udp.sh
```

验证点：
- UDP 报文从公网端口到客户端本地服务
- 本地服务响应报文可回传到请求方
- 启用 `use_compression` + `use_encryption` 时链路可正常收发

## 5. TLS 传输端到端测试

```bash
bash test/test_e2e_tls.sh
```

验证点：
- 控制连接 TLS 握手成功
- 工作连接 TLS 握手成功
- TLS 保护下的端到端数据转发成功

## 6. 常见问题排查

- 端口冲突：先结束历史 `arps/arpc` 进程后重试。
- 首包丢失：检查 `MessageTransport` 是否使用了 `into_inner_with_read_buf()` 并转发预读缓冲。
- 域名无路由：确认 `custom_domains` 或 `subdomain` 配置与请求中的 `Host/SNI` 一致。
- TCP 分组注册失败：确认同组代理设置了固定 `remote_port`（非 0）。
