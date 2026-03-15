#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust STCP/SUDP E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27120
TEST_STCP_REMOTE_PORT=26120
TEST_SUDP_REMOTE_PORT=26121
TEST_STCP_LOCAL_PORT=22230
TEST_SUDP_LOCAL_PORT=22240

echo "Building debug binaries..."
cargo build >/dev/null

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
    kill ${STCP_BACKEND_PID:-0} 2>/dev/null || true
    kill ${SUDP_BACKEND_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "1. Starting STCP backend..."
nc -l "$TEST_STCP_LOCAL_PORT" -w 3 > /tmp/arp_stcp_recv &
STCP_BACKEND_PID=$!

echo "2. Starting SUDP backend..."
python3 - <<'PY' &
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", 22240))
while True:
    data, addr = s.recvfrom(65535)
    s.sendto(data, addr)
PY
SUDP_BACKEND_PID=$!
sleep 1

echo "3. Starting ARP server..."
cat > /tmp/server_test_stcp_sudp.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true

[[allow_ports]]
start = $TEST_STCP_REMOTE_PORT
end = $TEST_SUDP_REMOTE_PORT
EOF

$SERVER_BIN -v -c /tmp/server_test_stcp_sudp.toml >/tmp/arp_server_stcp_sudp.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "4. Starting ARP client with STCP/SUDP..."
cat > /tmp/client_test_stcp_sudp.toml << EOF
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
pool_count = 1
tcp_mux = true

[[proxies]]
name = "stcp_test"
type = "stcp"
local_ip = "127.0.0.1"
local_port = $TEST_STCP_LOCAL_PORT
remote_port = $TEST_STCP_REMOTE_PORT
sk = "stcp_secret"

[[proxies]]
name = "sudp_test"
type = "sudp"
local_ip = "127.0.0.1"
local_port = $TEST_SUDP_LOCAL_PORT
remote_port = $TEST_SUDP_REMOTE_PORT
sk = "sudp_secret"
EOF

$CLIENT_BIN -v -c /tmp/client_test_stcp_sudp.toml >/tmp/arp_client_stcp_sudp.log 2>&1 &
CLIENT_PID=$!
sleep 3
kill -0 $CLIENT_PID

echo "5. Testing STCP..."
echo "hello-stcp" | nc 127.0.0.1 "$TEST_STCP_REMOTE_PORT" -w 1
sleep 1
if [ "$(cat /tmp/arp_stcp_recv)" != "hello-stcp" ]; then
    echo "✗ STCP failed"
    exit 1
fi

echo "6. Testing SUDP..."
export TEST_SUDP_REMOTE_PORT
python3 - <<'PY'
import os
import socket
p = int(os.environ["TEST_SUDP_REMOTE_PORT"])
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(3)
s.sendto(b"hello-sudp", ("127.0.0.1", p))
data, _ = s.recvfrom(65535)
if data != b"hello-sudp":
    raise SystemExit(f"unexpected sudp response: {data!r}")
print("sudp pass")
PY

echo ""
echo "=== STCP/SUDP E2E passed ==="
