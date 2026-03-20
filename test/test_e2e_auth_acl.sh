#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust Auth ACL E2E Test ==="

echo ""
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27180
TEST_REMOTE_PORT=26180

echo "Building debug binaries..."
cargo build >/dev/null

cleanup() {
  echo "Cleaning up..."
  kill ${SERVER_PID:-0} 2>/dev/null || true
  kill ${CLIENT_OK_PID:-0} 2>/dev/null || true
  kill ${CLIENT_BAD_TYPE_PID:-0} 2>/dev/null || true
  kill ${CLIENT_BAD_PORT_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "1. Start server with token ACL rules..."
cat > /tmp/server_test_auth_acl.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "root_token"

[[auth.rules]]
token = "scoped_token"
allow_proxy_types = ["tcp"]
max_pool_count = 1

[[auth.rules.allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT

[transport]
protocol = "tcp"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_auth_acl.toml >/tmp/arp_server_auth_acl.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "2. Start allowed client..."
cat > /tmp/client_test_auth_acl_ok.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "scoped_token"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "ok_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_auth_acl_ok.toml >/tmp/arp_client_auth_acl_ok.log 2>&1 &
CLIENT_OK_PID=$!
sleep 3
kill -0 $CLIENT_OK_PID
if ! grep -q "registered successfully" /tmp/arp_client_auth_acl_ok.log; then
  echo "✗ allowed client did not register proxy"
  tail -n 120 /tmp/arp_client_auth_acl_ok.log || true
  exit 1
fi
kill $CLIENT_OK_PID 2>/dev/null || true
wait $CLIENT_OK_PID 2>/dev/null || true

echo "3. Start disallowed proxy type client..."
cat > /tmp/client_test_auth_acl_bad_type.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "scoped_token"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "bad_udp"
type = "udp"
local_ip = "127.0.0.1"
local_port = 53
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_auth_acl_bad_type.toml >/tmp/arp_client_auth_acl_bad_type.log 2>&1 &
CLIENT_BAD_TYPE_PID=$!
sleep 3
kill -0 $CLIENT_BAD_TYPE_PID || true
if ! grep -q "proxy type udp is not allowed" /tmp/arp_client_auth_acl_bad_type.log; then
  echo "✗ disallowed proxy type should be rejected"
  tail -n 120 /tmp/arp_client_auth_acl_bad_type.log || true
  exit 1
fi
kill $CLIENT_BAD_TYPE_PID 2>/dev/null || true
wait $CLIENT_BAD_TYPE_PID 2>/dev/null || true

echo "4. Start disallowed remote port client..."
cat > /tmp/client_test_auth_acl_bad_port.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "scoped_token"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "bad_port"
type = "tcp"
local_ip = "127.0.0.1"
local_port = 22
remote_port = $((TEST_REMOTE_PORT+1))
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_auth_acl_bad_port.toml >/tmp/arp_client_auth_acl_bad_port.log 2>&1 &
CLIENT_BAD_PORT_PID=$!
sleep 3
kill -0 $CLIENT_BAD_PORT_PID || true
if ! grep -q "remote_port .* is not allowed" /tmp/arp_client_auth_acl_bad_port.log; then
  echo "✗ disallowed remote port should be rejected"
  tail -n 120 /tmp/arp_client_auth_acl_bad_port.log || true
  exit 1
fi

echo ""
echo "✓ Auth ACL E2E passed"
