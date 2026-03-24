#!/bin/bash
# Build release binaries for Mac ARM and Linux (x86_64 + arm64).
#
# Prerequisites:
#   brew install zig
#   cargo install cargo-zigbuild
#   rustup target add aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
#
# Usage:
#   bash scripts/build-release.sh

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
echo "=== ARP-Rust Release Build v${VERSION} ==="

mkdir -p dist

build_and_pack() {
  local target="$1"
  local label="$2"
  local compiler="$3"  # "cargo" or "zigbuild"

  echo "--- Building target: $target ($label) ---"
  if [ "$compiler" = "zigbuild" ]; then
    cargo zigbuild --workspace --release --target "$target"
  else
    cargo build --workspace --release --target "$target"
  fi

  local pkg="arp-rust-${VERSION}-${label}"
  local dir="dist/${pkg}"
  mkdir -p "$dir"
  cp "target/${target}/release/arps" "$dir/"
  cp "target/${target}/release/arpc" "$dir/"
  cp -r examples "$dir/"
  cp README.md LICENSE "$dir/"
  tar -czf "dist/${pkg}.tar.gz" -C dist "$pkg"
  rm -rf "$dir"
  echo "✓ dist/${pkg}.tar.gz"
}

build_and_pack "aarch64-apple-darwin"      "mac-arm64"     "cargo"
build_and_pack "x86_64-unknown-linux-gnu"  "linux-x86_64"  "zigbuild"
build_and_pack "aarch64-unknown-linux-gnu" "linux-arm64"   "zigbuild"

echo ""
echo "=== Release archives ==="
ls -lh dist/arp-rust-"${VERSION}"-*.tar.gz
