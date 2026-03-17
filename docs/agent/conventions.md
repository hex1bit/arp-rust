# Conventions

## Configuration Conventions

项目强依赖 TOML 配置，新增功能时优先考虑是否应以配置项驱动，而不是硬编码行为。

关键配置类型集中在 `crates/arp-common/src/config/mod.rs`：
- `ServerConfig`
- `ClientConfig`
- `ProxyConfig`
- `VisitorConfig`

约定上，配置读取后应尽早执行校验；很多错误会在 `validate()` 阶段被拦截，而不是延迟到运行时。

## Transport Selection Conventions

`transport.protocol` 当前支持：
- `tcp`
- `websocket`
- `quic`
- `kcp`

额外约定：
- `transport.tls.enable = true` 可把 TCP 提升为 TLS，把 WebSocket 提升为 WSS
- README 推荐首次公网部署优先用 `wss`
- `quic` 更适合已确认 UDP 可达的环境
- `tcp` 更适合本地或受控网络中的最小化验证

新增 transport 时，应优先复用 `arp-common/src/transport/` 的统一流抽象，而不是在 server/client 两侧单独分叉协议处理。

## Proxy Type Conventions

`[[proxies]].type` 支持：
- `tcp`
- `udp`
- `http`
- `https`
- `stcp`
- `sudp`
- `xtcp`

现有代码体现的使用约定：
- `tcp`：通用 TCP 服务
- `udp`：通用 UDP 服务
- `http` / `https`：依赖 Host/SNI 的虚拟主机模式
- `stcp` / `sudp`：带共享密钥的安全代理
- `xtcp`：provider/visitor 模式的 NAT 打洞访问

## File Organization Conventions

Workspace 按职责拆 crate，而不是按部署环境拆仓库：
- 共享类型与协议放在 `arp-common`
- 服务端控制面与公网接入放在 `arp-server`
- 客户端执行面与本地服务转发放在 `arp-client`

在 crate 内部再按领域拆模块，例如：
- `protocol/`
- `transport/`
- `proxy/`
- `control/`
- `web/`
- `resource/`

新增能力时，优先放入现有领域目录，避免创建含义模糊的新顶级模块。

## Messaging and Connection Conventions

项目区分两类连接：
- control connection：持久控制面连接
- work connection：按需建立的数据连接

实现新代理能力时，通常遵循既有模式：
1. 通过 control connection 完成登录、注册与调度
2. 在真正转发数据前建立 work connection
3. 在 work connection 上透传实际业务流量

如果需要在控制面上增加新能力，优先扩展共享消息定义与 codec，而不是直接在某一侧塞入特殊分支。

## Buffer-handling Convention

`docs/DEVELOPMENT.md` 特别强调：
- `MessageTransport` 切换回原始流时，必须保留 codec 预读缓冲
- 否则 `StartWorkConn` 之后的首包数据可能丢失

因此，任何涉及 transport 解包、升级、桥接的修改，都要优先检查预读缓冲传递是否完整。

## VHost Convention

`http` / `https` 代理必须配置域名：
- `custom_domains`
- 或 `subdomain`

新增 vhost 相关能力时，默认假设服务端使用共享监听端口，客户端通过域名映射完成分发，而不是为每个站点分配独立公网端口。

## Naming and Examples

示例配置遵循“按 transport/场景命名”的约定：
- `server_quic.toml`
- `client_ws.toml`
- `server_prod_wss.toml`
- `client_xtcp_provider.toml`

新增示例时，保持这种直接表达 transport 或角色的命名方式，便于从文件名快速定位用途。
