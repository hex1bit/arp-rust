#!/bin/bash

set -e

echo "=== ARP-Rust E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27100
TEST_REMOTE_PORT=26100
TEST_LOCAL_PORT=2222
TEST_DASHBOARD_PORT=27500

echo "Building debug binaries..."
cargo build >/dev/null

cleanup() {
    echo "Cleaning up..."
    kill $SERVER_PID 2>/dev/null || true
    kill $CLIENT_PID 2>/dev/null || true
    kill $TEST_SERVER_PID 2>/dev/null || true
}

trap cleanup EXIT

echo "1. Starting test SSH server on port 2222..."
nc -l "$TEST_LOCAL_PORT" -w 3 > /tmp/arp_test_recv &
TEST_SERVER_PID=$!
sleep 4

echo "2. Starting ARP server..."
cat > /tmp/server_test.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
dashboard_addr = "127.0.0.1"
dashboard_port = $TEST_DASHBOARD_PORT

log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF

$SERVER_BIN -v -c /tmp/server_test.toml >/tmp/arp_server.log 2>&1 &
SERVER_PID=$!
sleep 2
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "✗ Server failed to stay running"
    echo "--- Server log ---"
    cat /tmp/arp_server.log || true
    exit 1
fi

echo "3. Starting ARP client..."

cat > /tmp/client_test.toml << EOF
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT

log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "test_nc"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF

$CLIENT_BIN -v -c /tmp/client_test.toml >/tmp/arp_client.log 2>&1 &
CLIENT_PID=$!
sleep 3
if ! kill -0 $CLIENT_PID 2>/dev/null; then
    echo "✗ Client failed to stay running"
    echo "--- Client log ---"
    cat /tmp/arp_client.log || true
    exit 1
fi

echo "4. Testing connection through proxy..."
echo "Hello from ARP!" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 1

sleep 1

echo "5. Testing admin endpoints..."
HEALTHZ=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/healthz" || true)
if [ "$HEALTHZ" != "ok" ]; then
    echo "✗ Admin healthz failed"
    echo "  Response: $HEALTHZ"
    exit 1
fi

METRICS=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/metrics" || true)
if [[ "$METRICS" != *"arp_active_controls"* ]] || [[ "$METRICS" != *"arp_active_proxies"* ]] || [[ "$METRICS" != *"arp_tcp_proxy_connections_total"* ]]; then
    echo "✗ Admin metrics failed"
    echo "$METRICS"
    exit 1
fi

DASHBOARD=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/" || true)
if [[ "$DASHBOARD" != *"ARP Dashboard"* ]]; then
    echo "✗ Dashboard page failed"
    exit 1
fi

STATUS_JSON=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/api/v1/status" || true)
if [[ "$STATUS_JSON" != *"\"status\":\"ok\""* ]]; then
    echo "✗ Admin status API failed"
    echo "$STATUS_JSON"
    exit 1
fi

PROXIES_JSON=$(curl -s "http://127.0.0.1:$TEST_DASHBOARD_PORT/api/v1/proxies" || true)
if [[ "$PROXIES_JSON" != *"test_nc"* ]]; then
    echo "✗ Admin proxies API failed"
    echo "$PROXIES_JSON"
    exit 1
fi

if [ -f /tmp/arp_test_recv ]; then
    RECEIVED=$(cat /tmp/arp_test_recv)
    if [ "$RECEIVED" = "Hello from ARP!" ]; then
        echo "✓ Test PASSED: Message received correctly!"
        echo "  Sent: 'Hello from ARP!'"
        echo "  Received: '$RECEIVED'"
    else
        echo "✗ Test FAILED: Message mismatch"
        echo "  Expected: 'Hello from ARP!'"
        echo "  Received: '$RECEIVED'"
        echo ""
        echo "--- Server log ---"
        cat /tmp/arp_server.log || true
        echo "--- Client log ---"
        cat /tmp/arp_client.log || true
        exit 1
    fi
else
    echo "✗ Test FAILED: No data received"
    echo ""
    echo "--- Server log ---"
    cat /tmp/arp_server.log || true
    echo "--- Client log ---"
    cat /tmp/arp_client.log || true
    exit 1
fi

echo ""
echo "=== All tests passed! ==="
