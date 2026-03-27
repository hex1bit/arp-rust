#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust WebSocket Transport E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27160
TEST_REMOTE_PORT=26160
TEST_LOCAL_PORT=2260

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
rm -f /tmp/arp_ws_recv
nc -l "$TEST_LOCAL_PORT" -w 5 > /tmp/arp_ws_recv &
BACKEND_PID=$!
sleep 2

echo "2. Starting ARP server in websocket mode..."
cat > /tmp/server_test_ws.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "websocket"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_ws.toml >/tmp/arp_server_ws.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "3. Starting ARP client in websocket mode..."
cat > /tmp/client_test_ws.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "websocket"
pool_count = 1
tcp_mux = false

[[proxies]]
name = "ws_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_ws.toml >/tmp/arp_client_ws.log 2>&1 &
CLIENT_PID=$!
sleep 5
kill -0 $CLIENT_PID

echo "4. Verify forwarding through websocket transport..."
echo -n "WS_TRANSPORT_OK" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 2 || true
sleep 2

if [ ! -f /tmp/arp_ws_recv ]; then
  echo "✗ backend receive file missing"
  tail -n 200 /tmp/arp_server_ws.log || true
  tail -n 200 /tmp/arp_client_ws.log || true
  exit 1
fi

RECV=$(cat /tmp/arp_ws_recv)
if [ "$RECV" != "WS_TRANSPORT_OK" ]; then
  echo "✗ websocket transport forwarding failed, got '$RECV'"
  echo "--- server log ---"
  tail -n 200 /tmp/arp_server_ws.log || true
  echo "--- client log ---"
  tail -n 200 /tmp/arp_client_ws.log || true
  exit 1
fi

echo ""
echo "✓ WebSocket transport E2E passed"
