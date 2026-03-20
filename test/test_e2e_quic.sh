#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust QUIC Transport E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27180
TEST_REMOTE_PORT=26180
TEST_LOCAL_PORT=22680

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
    kill ${BACKEND_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Generating QUIC certificates..."
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout /tmp/arp_quic_server_key.pem \
    -out /tmp/arp_quic_server_cert.pem \
    -days 1 \
    -subj "/CN=arps-quic.local" \
    -addext "subjectAltName=DNS:arps-quic.local,IP:127.0.0.1" \
    -addext "basicConstraints=CA:FALSE" \
    -addext "keyUsage=digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" >/tmp/arp_quic_cert_gen.log 2>&1

echo "2. Starting local backend receiver..."
rm -f /tmp/arp_quic_recv
nc -l "$TEST_LOCAL_PORT" -w 5 > /tmp/arp_quic_recv &
BACKEND_PID=$!
sleep 2

echo "3. Starting ARP server in quic mode..."
cat > /tmp/server_test_quic.toml << EOF_CFG
bind_addr = "0.0.0.0"
bind_port = $TEST_CONTROL_PORT
quic_bind_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "quic"

[transport.tls]
cert_file = "/tmp/arp_quic_server_cert.pem"
key_file = "/tmp/arp_quic_server_key.pem"

[[allow_ports]]
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF_CFG

$SERVER_BIN -v -c /tmp/server_test_quic.toml >/tmp/arp_server_quic.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "4. Starting ARP client in quic mode..."
cat > /tmp/client_test_quic.toml << EOF_CFG
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "quic"
pool_count = 1
tcp_mux = false

[transport.tls]
trusted_ca_file = "/tmp/arp_quic_server_cert.pem"
server_name = "arps-quic.local"

[[proxies]]
name = "quic_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF_CFG

$CLIENT_BIN -v -c /tmp/client_test_quic.toml >/tmp/arp_client_quic.log 2>&1 &
CLIENT_PID=$!
sleep 5
kill -0 $CLIENT_PID

echo "5. Verify forwarding through quic transport..."
echo -n "QUIC_TRANSPORT_OK" | nc 127.0.0.1 "$TEST_REMOTE_PORT" -w 2 || true
sleep 2

if [ ! -f /tmp/arp_quic_recv ]; then
  echo "✗ backend receive file missing"
  tail -n 200 /tmp/arp_server_quic.log || true
  tail -n 200 /tmp/arp_client_quic.log || true
  exit 1
fi

RECV=$(cat /tmp/arp_quic_recv)
if [ "$RECV" != "QUIC_TRANSPORT_OK" ]; then
  echo "✗ quic transport forwarding failed, got '$RECV'"
  echo "--- server log ---"
  tail -n 200 /tmp/arp_server_quic.log || true
  echo "--- client log ---"
  tail -n 200 /tmp/arp_client_quic.log || true
  exit 1
fi

echo ""
echo "✓ QUIC transport E2E passed"
