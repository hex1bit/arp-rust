#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust Health Check E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27130
TEST_REMOTE_PORT=26130
UNHEALTHY_LOCAL_PORT=22999

echo "Building debug binaries..."
cargo build >/dev/null

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "1. Starting ARP server..."
cat > /tmp/server_test_health.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF

$SERVER_BIN -v -c /tmp/server_test_health.toml >/tmp/arp_server_health.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "2. Starting ARP client with enabled health check (unhealthy local port)..."
cat > /tmp/client_test_health.toml << EOF
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "health_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $UNHEALTHY_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT

[proxies.health_check]
enable = true
check_type = "tcp"
timeout_seconds = 1
interval_seconds = 1
max_failed = 1
EOF

$CLIENT_BIN -v -c /tmp/client_test_health.toml >/tmp/arp_client_health.log 2>&1 &
CLIENT_PID=$!
sleep 3
kill -0 $CLIENT_PID

echo "3. Triggering remote access to force health gate..."
echo "health-probe" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 1 || true
sleep 1

echo "4. Verifying unhealthy log..."
if ! grep -q "is unhealthy" /tmp/arp_client_health.log; then
    echo "✗ expected unhealthy log not found"
    echo "--- client log ---"
    tail -n 120 /tmp/arp_client_health.log || true
    exit 1
fi

echo "✓ health check rejected unhealthy backend as expected"
echo ""
echo "=== Health Check E2E passed ==="
