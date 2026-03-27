#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust WSS Transport E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27161
TEST_REMOTE_PORT=26161
TEST_LOCAL_PORT=2261

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
    kill ${BACKEND_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Generating WSS certificates..."
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout /tmp/arp_wss_server_key.pem \
    -out /tmp/arp_wss_server_cert.pem \
    -days 1 \
    -subj "/CN=arps-wss.local" \
    -addext "subjectAltName=DNS:arps-wss.local,IP:127.0.0.1" \
    -addext "basicConstraints=CA:FALSE" \
    -addext "keyUsage=digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" >/tmp/arp_wss_cert_gen.log 2>&1

echo "2. Starting local backend receiver..."
rm -f /tmp/arp_wss_recv
nc -l "$TEST_LOCAL_PORT" -w 5 > /tmp/arp_wss_recv &
BACKEND_PID=$!
sleep 2

echo "3. Starting ARP server in wss mode..."
cat > /tmp/server_test_wss.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "websocket"

[transport.tls]
enable = true
cert_file = "/tmp/arp_wss_server_cert.pem"
key_file = "/tmp/arp_wss_server_key.pem"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_wss.toml >/tmp/arp_server_wss.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "4. Starting ARP client in wss mode..."
cat > /tmp/client_test_wss.toml << EOF_CFG
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

[transport.tls]
enable = true
trusted_ca_file = "/tmp/arp_wss_server_cert.pem"
server_name = "arps-wss.local"

[[proxies]]
name = "wss_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_wss.toml >/tmp/arp_client_wss.log 2>&1 &
CLIENT_PID=$!
sleep 5
kill -0 $CLIENT_PID

echo "5. Verify forwarding through wss transport..."
echo -n "WSS_TRANSPORT_OK" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 2 || true
sleep 2

if [ ! -f /tmp/arp_wss_recv ]; then
  echo "✗ backend receive file missing"
  tail -n 200 /tmp/arp_server_wss.log || true
  tail -n 200 /tmp/arp_client_wss.log || true
  exit 1
fi

RECV=$(cat /tmp/arp_wss_recv)
if [ "$RECV" != "WSS_TRANSPORT_OK" ]; then
  echo "✗ wss transport forwarding failed, got '$RECV'"
  echo "--- server log ---"
  tail -n 200 /tmp/arp_server_wss.log || true
  echo "--- client log ---"
  tail -n 200 /tmp/arp_client_wss.log || true
  exit 1
fi

echo ""
echo "✓ WSS transport E2E passed"
