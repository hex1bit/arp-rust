#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust XTCP E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
STUB_SRC="$PROJECT_DIR/test/support/tcp_backend_stub.rs"
STUB_BIN="/tmp/tcp_backend_stub"
TEST_CONTROL_PORT=27150
BACKEND_PORT=23331
VISITOR_BIND_PORT=26331
TEST_DASHBOARD_PORT=27550

cleanup() {
  echo "Cleaning up..."
  kill ${SERVER_PID:-0} 2>/dev/null || true
  kill ${PROVIDER_PID:-0} 2>/dev/null || true
  kill ${VISITOR_PID:-0} 2>/dev/null || true
  kill ${VISITOR_BAD_PID:-0} 2>/dev/null || true
  kill ${BACKEND_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "Building backend stub..."
rustc "$STUB_SRC" -O -o "$STUB_BIN"

echo "1. Start provider local backend..."
"$STUB_BIN" "$BACKEND_PORT" X >/tmp/backend_xtcp.log 2>&1 &
BACKEND_PID=$!
sleep 1
kill -0 $BACKEND_PID

echo "2. Start ARP server..."
cat > /tmp/server_test_xtcp.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
dashboard_addr = "127.0.0.1"
dashboard_port = $TEST_DASHBOARD_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_xtcp.toml >/tmp/arp_server_xtcp.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "3. Start xtcp provider client..."
cat > /tmp/client_test_xtcp_provider.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[proxies]]
name = "xtcp_demo"
type = "xtcp"
local_ip = "127.0.0.1"
local_port = $BACKEND_PORT
sk = "xtcp_secret"
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_xtcp_provider.toml >/tmp/arp_client_xtcp_provider.log 2>&1 &
PROVIDER_PID=$!
sleep 2
kill -0 $PROVIDER_PID

echo "4. Start xtcp visitor client..."
cat > /tmp/client_test_xtcp_visitor.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[visitors]]
name = "xtcp_visitor"
type = "xtcp"
server_name = "xtcp_demo"
sk = "xtcp_secret"
bind_addr = "127.0.0.1"
bind_port = $VISITOR_BIND_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_xtcp_visitor.toml >/tmp/arp_client_xtcp_visitor.log 2>&1 &
VISITOR_PID=$!
sleep 3
kill -0 $VISITOR_PID

echo "5. Verify xtcp data path..."
TOKEN=$(printf "hello" | nc 127.0.0.1 "$VISITOR_BIND_PORT" -w 3 | head -c 1 || true)
if [ "$TOKEN" != "X" ]; then
  echo "✗ xtcp relay failed, expected token X got '$TOKEN'"
  echo "--- server log ---"
  tail -n 200 /tmp/arp_server_xtcp.log || true
  echo "--- provider log ---"
  tail -n 200 /tmp/arp_client_xtcp_provider.log || true
  echo "--- visitor log ---"
  tail -n 200 /tmp/arp_client_xtcp_visitor.log || true
  exit 1
fi

echo "6. Abnormal case: wrong sk should fail..."
cat > /tmp/client_test_xtcp_visitor_bad.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[visitors]]
name = "xtcp_visitor_bad"
type = "xtcp"
server_name = "xtcp_demo"
sk = "wrong_sk"
bind_addr = "127.0.0.1"
bind_port = $((VISITOR_BIND_PORT+1))
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_xtcp_visitor_bad.toml >/tmp/arp_client_xtcp_visitor_bad.log 2>&1 &
VISITOR_BAD_PID=$!
sleep 3
kill -0 $VISITOR_BAD_PID

ERR_MSG=$(printf "hello" | nc 127.0.0.1 "$((VISITOR_BIND_PORT+1))" -w 3 || true)
if [[ "$ERR_MSG" != *"xtcp error"* ]]; then
  echo "✗ wrong-sk visitor should return xtcp error"
  echo "response: $ERR_MSG"
  echo "--- bad visitor log ---"
  tail -n 200 /tmp/arp_client_xtcp_visitor_bad.log || true
  exit 1
fi

echo "7. Checking xtcp metrics and recent events..."
XTCP_EVENTS=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/api/v1/xtcp/events" || true)
if [[ "$XTCP_EVENTS" != *"visitor_forwarded"* ]] || [[ "$XTCP_EVENTS" != *"sk_mismatch"* ]]; then
  echo "✗ xtcp events API missing expected stages"
  echo "$XTCP_EVENTS"
  exit 1
fi

XTCP_METRICS=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/metrics" || true)
if [[ "$XTCP_METRICS" != *"arp_xtcp_visitor_requests_total"* ]] || [[ "$XTCP_METRICS" != *"arp_xtcp_sk_mismatch_total"* ]]; then
  echo "✗ xtcp metrics missing"
  echo "$XTCP_METRICS"
  exit 1
fi

echo ""
echo "✓ XTCP E2E passed"
