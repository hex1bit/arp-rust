#!/bin/bash

set -e

echo "=== ARP-Rust TLS E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"

CONTROL_PORT=27400
REMOTE_PORT=26400
LOCAL_PORT=22422

cleanup() {
    echo "Cleaning up..."
    kill "$SERVER_PID" 2>/dev/null || true
    kill "$CLIENT_PID" 2>/dev/null || true
    kill "$TEST_SERVER_PID" 2>/dev/null || true
}

trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Generating TLS certificates..."
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout /tmp/arp_tls_server_key.pem \
    -out /tmp/arp_tls_server_cert.pem \
    -days 1 \
    -subj "/CN=arps.local" \
    -addext "subjectAltName=DNS:arps.local,IP:127.0.0.1" \
    -addext "basicConstraints=CA:FALSE" \
    -addext "keyUsage=digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" >/tmp/arp_tls_cert_gen.log 2>&1

echo "2. Starting local TCP backend..."
nc -l "$LOCAL_PORT" -w 5 > /tmp/arp_tls_test_recv &
TEST_SERVER_PID=$!
sleep 1

echo "3. Starting ARP server with TLS..."
cat > /tmp/server_tls_test.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $CONTROL_PORT
log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true

[transport.tls]
enable = true
cert_file = "/tmp/arp_tls_server_cert.pem"
key_file = "/tmp/arp_tls_server_key.pem"

[[allow_ports]]
start = $REMOTE_PORT
end = $REMOTE_PORT
EOF

$SERVER_BIN -v -c /tmp/server_tls_test.toml >/tmp/arp_server_tls.log 2>&1 &
SERVER_PID=$!
sleep 2

echo "4. Starting ARP client with TLS..."
cat > /tmp/client_tls_test.toml << EOF
server_addr = "127.0.0.1"
server_port = $CONTROL_PORT
log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
pool_count = 1

[transport.tls]
enable = true
trusted_ca_file = "/tmp/arp_tls_server_cert.pem"
server_name = "arps.local"

[[proxies]]
name = "tls_tcp"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $LOCAL_PORT
remote_port = $REMOTE_PORT
EOF

$CLIENT_BIN -v -c /tmp/client_tls_test.toml >/tmp/arp_client_tls.log 2>&1 &
CLIENT_PID=$!
sleep 3

echo "5. Testing TLS protected proxy data path..."
echo "Hello from TLS ARP!" | nc 127.0.0.1 "$REMOTE_PORT" -w 1
sleep 1

if [ -f /tmp/arp_tls_test_recv ]; then
    RECEIVED=$(cat /tmp/arp_tls_test_recv)
    if [ "$RECEIVED" = "Hello from TLS ARP!" ]; then
        echo "✓ TLS E2E passed"
        echo "  Sent: 'Hello from TLS ARP!'"
        echo "  Received: '$RECEIVED'"
    else
        echo "✗ TLS E2E failed: response mismatch"
        echo "  Expected: 'Hello from TLS ARP!'"
        echo "  Received: '$RECEIVED'"
        echo "--- Server log ---"
        cat /tmp/arp_server_tls.log || true
        echo "--- Client log ---"
        cat /tmp/arp_client_tls.log || true
        exit 1
    fi
else
    echo "✗ TLS E2E failed: no backend data"
    echo "--- Server log ---"
    cat /tmp/arp_server_tls.log || true
    echo "--- Client log ---"
    cat /tmp/arp_client_tls.log || true
    exit 1
fi

echo ""
echo "=== TLS E2E test passed! ==="
