#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust KCP Transport E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27170
TEST_REMOTE_PORT=26170
TEST_LOCAL_PORT=22670

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
    kill ${BACKEND_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Starting local backend receiver..."
rm -f /tmp/arp_kcp_recv
nc -l "$TEST_LOCAL_PORT" -w 20 > /tmp/arp_kcp_recv &
BACKEND_PID=$!
sleep 2

echo "2. Starting ARP server in kcp mode..."
cat > /tmp/server_test_kcp.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
kcp_bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "kcp"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_kcp.toml >/tmp/arp_server_kcp.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "3. Starting ARP client in kcp mode..."
cat > /tmp/client_test_kcp.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "kcp"
pool_count = 1
tcp_mux = false

[[proxies]]
name = "kcp_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_kcp.toml >/tmp/arp_client_kcp.log 2>&1 &
CLIENT_PID=$!
sleep 5
kill -0 $CLIENT_PID

echo "4. Verify forwarding through kcp transport..."
echo -n "KCP_TRANSPORT_OK" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 2 || true
sleep 2

if [ ! -f /tmp/arp_kcp_recv ]; then
  echo "✗ backend receive file missing"
  tail -n 200 /tmp/arp_server_kcp.log || true
  tail -n 200 /tmp/arp_client_kcp.log || true
  exit 1
fi

RECV=$(cat /tmp/arp_kcp_recv)
if [ "$RECV" != "KCP_TRANSPORT_OK" ]; then
  echo "✗ kcp transport forwarding failed, got '$RECV'"
  echo "--- server log ---"
  tail -n 200 /tmp/arp_server_kcp.log || true
  echo "--- client log ---"
  tail -n 200 /tmp/arp_client_kcp.log || true
  exit 1
fi

echo ""
echo "✓ KCP transport E2E passed"
