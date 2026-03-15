#!/bin/bash

set -e

echo "=== ARP-Rust UDP E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"

CONTROL_PORT=27300
REMOTE_UDP_PORT=26300
LOCAL_UDP_PORT=22333

cleanup() {
    echo "Cleaning up..."
    kill "$SERVER_PID" 2>/dev/null || true
    kill "$CLIENT_PID" 2>/dev/null || true
    kill "$UDP_BACKEND_PID" 2>/dev/null || true
}

trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Starting local UDP echo backend..."
python3 -u -c '
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", int("'"$LOCAL_UDP_PORT"'")))
while True:
    data, addr = s.recvfrom(65535)
    s.sendto(data, addr)
' >/tmp/arp_udp_backend.log 2>&1 &
UDP_BACKEND_PID=$!
sleep 1

echo "2. Starting ARP server..."
cat > /tmp/server_udp_test.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $CONTROL_PORT
log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true

[[allow_ports]]
start = $REMOTE_UDP_PORT
end = $REMOTE_UDP_PORT
EOF

$SERVER_BIN -v -c /tmp/server_udp_test.toml >/tmp/arp_server_udp.log 2>&1 &
SERVER_PID=$!
sleep 2

echo "3. Starting ARP client..."
cat > /tmp/client_udp_test.toml << EOF
server_addr = "127.0.0.1"
server_port = $CONTROL_PORT
log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
pool_count = 1

[[proxies]]
name = "udp_echo"
type = "udp"
local_ip = "127.0.0.1"
local_port = $LOCAL_UDP_PORT
remote_port = $REMOTE_UDP_PORT
use_compression = true
use_encryption = true
sk = "udp_secure_key_123"
EOF

$CLIENT_BIN -v -c /tmp/client_udp_test.toml >/tmp/arp_client_udp.log 2>&1 &
CLIENT_PID=$!
sleep 3

echo "4. Testing UDP round-trip..."
UDP_RESP=$(python3 -c '
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(5)
msg = b"hello-udp-arp"
s.sendto(msg, ("127.0.0.1", int("'"$REMOTE_UDP_PORT"'")))
data, _ = s.recvfrom(65535)
print(data.decode("utf-8"))
')

if [ "$UDP_RESP" != "hello-udp-arp" ]; then
    echo "✗ UDP test failed"
    echo "  Expected: hello-udp-arp"
    echo "  Received: $UDP_RESP"
    echo "--- Server log ---"
    cat /tmp/arp_server_udp.log || true
    echo "--- Client log ---"
    cat /tmp/arp_client_udp.log || true
    exit 1
fi

echo "✓ UDP test passed"
echo ""
echo "=== UDP E2E test passed! ==="
