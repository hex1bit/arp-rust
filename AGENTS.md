# AGENTS.md

This file provides guidance to codeflicker when working with code in this repository.

## WHY: Purpose and Goals
ARP-Rust 是一个 Rust 编写的反向代理与隧道工具，采用 client/server 模型把私有网络中的 TCP、UDP、HTTP、HTTPS 服务暴露到公网。它的核心价值是同时支持多种代理类型与多种传输后端，覆盖从本地实验到公网部署的场景。

## WHAT: Technical Stack
- Runtime/Language: Rust 2021, Tokio
- Framework: Axum 管理接口，Clap CLI，Serde/TOML 配置
- Key dependencies: tokio-rustls, tokio-tungstenite, quinn, tokio_kcp, dashmap, tracing
- Workspace crates: `arp-common`, `arp-server`, `arp-client`
- Transports: TCP, TLS, WebSocket/WSS, QUIC, KCP
- Proxy types: TCP, UDP, HTTP, HTTPS, STCP, SUDP, XTCP

## HOW: Core Development Workflow
```bash
# Build
cargo build --workspace --release

# Unit tests
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format
cargo fmt --all

# Example E2E
bash test/test_e2e.sh
```

## Progressive Disclosure

For detailed information, consult these documents as needed:

- `docs/agent/development_commands.md` - Build, run, lint, and end-to-end test commands
- `docs/agent/architecture.md` - Workspace layout, connection model, and transport/proxy design
- `docs/agent/testing.md` - Unit and E2E test structure, scripts, and debugging notes
- `docs/agent/conventions.md` - Project-specific config, transport, and proxy conventions

**When working on a task, first determine which documentation is relevant, then read only those files.**
