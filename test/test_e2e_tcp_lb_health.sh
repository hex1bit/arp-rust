#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust TCP LB + Health E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
STUB_SRC="$PROJECT_DIR/test/support/tcp_backend_stub.rs"
STUB_BIN="/tmp/tcp_backend_stub"
TEST_CONTROL_PORT=27140
TEST_REMOTE_PORT=26140
BACKEND_A_PORT=23141
BACKEND_B_PORT=23142
DASHBOARD_PORT=27540

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_A_PID:-0} 2>/dev/null || true
    kill ${CLIENT_B_PID:-0} 2>/dev/null || true
    kill ${BAD_CLIENT_PID:-0} 2>/dev/null || true
    kill ${BACKEND_A_PID:-0} 2>/dev/null || true
    kill ${BACKEND_B_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "Building backend stub..."
rustc "$STUB_SRC" -O -o "$STUB_BIN"

read_remote_token() {
    printf "probe" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 2 2>/dev/null | head -c 1 || true
}

echo "1. Starting backend A only..."
"$STUB_BIN" "$BACKEND_A_PORT" A >/tmp/backend_a.log 2>&1 &
BACKEND_A_PID=$!
sleep 1
kill -0 $BACKEND_A_PID

echo "2. Starting ARP server..."
cat > /tmp/server_test_tcp_lb_health.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
dashboard_addr = "127.0.0.1"
dashboard_port = $DASHBOARD_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_tcp_lb_health.toml >/tmp/arp_server_tcp_lb_health.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "3. Starting healthy client A in lb group..."
cat > /tmp/client_test_tcp_lb_a.toml << EOF_CFG
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
name = "lb_tcp_a"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $BACKEND_A_PORT
remote_port = $TEST_REMOTE_PORT

[proxies.load_balancer]
group = "ssh_group"
group_key = "k1"

[proxies.health_check]
enable = true
check_type = "tcp"
timeout_seconds = 1
interval_seconds = 1
max_failed = 1
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_tcp_lb_a.toml >/tmp/arp_client_tcp_lb_a.log 2>&1 &
CLIENT_A_PID=$!
sleep 2
kill -0 $CLIENT_A_PID

echo "4. Starting initially-unhealthy client B in same lb group..."
cat > /tmp/client_test_tcp_lb_b.toml << EOF_CFG
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
name = "lb_tcp_b"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $BACKEND_B_PORT
remote_port = $TEST_REMOTE_PORT

[proxies.load_balancer]
group = "ssh_group"
group_key = "k1"

[proxies.health_check]
enable = true
check_type = "tcp"
timeout_seconds = 1
interval_seconds = 1
max_failed = 1
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_tcp_lb_b.toml >/tmp/arp_client_tcp_lb_b.log 2>&1 &
CLIENT_B_PID=$!
sleep 3
kill -0 $CLIENT_B_PID

echo "5. Verify unhealthy backend gets ejected and traffic still works via A..."
A_COUNT=0
for i in $(seq 1 10); do
    T=$(read_remote_token)
    if [ "$T" = "A" ]; then
        A_COUNT=$((A_COUNT+1))
    fi
    sleep 0.2
done
if [ "$A_COUNT" -lt 5 ]; then
    echo "✗ expected traffic through backend A while backend B unhealthy, got A_COUNT=$A_COUNT"
    echo "--- server log ---"
    tail -n 120 /tmp/arp_server_tcp_lb_health.log || true
    echo "--- client B log ---"
    tail -n 120 /tmp/arp_client_tcp_lb_b.log || true
    exit 1
fi

if ! grep -q "unhealthy" /tmp/arp_client_tcp_lb_b.log; then
    echo "✗ expected unhealthy log in client B"
    tail -n 120 /tmp/arp_client_tcp_lb_b.log || true
    exit 1
fi

echo "6. Start backend B and verify auto recovery + load balancing..."
"$STUB_BIN" "$BACKEND_B_PORT" B >/tmp/backend_b.log 2>&1 &
BACKEND_B_PID=$!
sleep 8

B_COUNT=0
A2_COUNT=0
for i in $(seq 1 16); do
    T=$(read_remote_token)
    if [ "$T" = "A" ]; then
        A2_COUNT=$((A2_COUNT+1))
    fi
    if [ "$T" = "B" ]; then
        B_COUNT=$((B_COUNT+1))
    fi
    sleep 0.2
done

if [ "$B_COUNT" -lt 2 ]; then
    echo "✗ expected recovered backend B to receive traffic, B_COUNT=$B_COUNT"
    echo "A_COUNT_AFTER=$A2_COUNT"
    echo "--- server log ---"
    tail -n 200 /tmp/arp_server_tcp_lb_health.log || true
    echo "--- client B log ---"
    tail -n 200 /tmp/arp_client_tcp_lb_b.log || true
    exit 1
fi

echo "7. Abnormal case: lb group with remote_port=0 should fail registration..."
cat > /tmp/client_test_tcp_lb_bad.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"

[[proxies]]
name = "lb_tcp_bad"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $BACKEND_A_PORT
remote_port = 0

[proxies.load_balancer]
group = "ssh_group"
group_key = "k1"
EOF_CFG

set +e
$CLIENT_BIN -v -c /tmp/client_test_tcp_lb_bad.toml >/tmp/arp_client_tcp_lb_bad.log 2>&1 &
BAD_CLIENT_PID=$!
sleep 3
kill -0 $BAD_CLIENT_PID 2>/dev/null
BAD_ALIVE=$?
set -e

if [ "$BAD_ALIVE" -eq 0 ]; then
    echo "✗ expected bad lb client to exit with registration error"
    tail -n 120 /tmp/arp_client_tcp_lb_bad.log || true
    exit 1
fi
if ! grep -q "requires fixed remote_port" /tmp/arp_client_tcp_lb_bad.log; then
    echo "✗ expected fixed remote_port validation error"
    tail -n 120 /tmp/arp_client_tcp_lb_bad.log || true
    exit 1
fi

echo "8. Verify admin API shows both group members registered..."
PROXIES_JSON=$(curl -s "http://127.0.0.1:$DASHBOARD_PORT/api/v1/proxies" || true)
if [[ "$PROXIES_JSON" != *"lb_tcp_a"* ]] || [[ "$PROXIES_JSON" != *"lb_tcp_b"* ]]; then
    echo "✗ expected both lb_tcp_a and lb_tcp_b in admin proxies"
    echo "$PROXIES_JSON"
    exit 1
fi

echo ""
echo "✓ TCP LB group + health ejection/recovery test passed"
