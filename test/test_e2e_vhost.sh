#!/bin/bash

set -e

echo "=== ARP-Rust VHost E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"

CONTROL_PORT=27200
VHOST_HTTP_PORT=28080
VHOST_HTTPS_PORT=28443
LOCAL_HTTP_PORT=18080
LOCAL_HTTPS_PORT=18443

cleanup() {
    echo "Cleaning up..."
    kill "$SERVER_PID" 2>/dev/null || true
    kill "$CLIENT_PID" 2>/dev/null || true
    kill "$HTTP_BACKEND_PID" 2>/dev/null || true
    kill "$HTTPS_BACKEND_PID" 2>/dev/null || true
}

trap cleanup EXIT

echo "Building debug binaries..."
cargo build >/dev/null

echo "1. Starting local HTTP backend..."
mkdir -p /tmp/arp_http_root
echo "arp-http-ok" > /tmp/arp_http_root/index.html
python3 -m http.server "$LOCAL_HTTP_PORT" --bind 127.0.0.1 --directory /tmp/arp_http_root >/tmp/arp_http_backend.log 2>&1 &
HTTP_BACKEND_PID=$!
sleep 1

echo "2. Starting local HTTPS backend..."
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout /tmp/arp_https_key.pem \
    -out /tmp/arp_https_cert.pem \
    -days 1 \
    -subj "/CN=secure.test.local" >/tmp/arp_https_cert_gen.log 2>&1
openssl s_server -accept "$LOCAL_HTTPS_PORT" -key /tmp/arp_https_key.pem -cert /tmp/arp_https_cert.pem -www >/tmp/arp_https_backend.log 2>&1 &
HTTPS_BACKEND_PID=$!
sleep 1

echo "3. Starting ARP server..."
cat > /tmp/server_vhost_test.toml << EOF
bind_addr = "0.0.0.0"
bind_port = $CONTROL_PORT
vhost_http_port = $VHOST_HTTP_PORT
vhost_https_port = $VHOST_HTTPS_PORT
log_level = "debug"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true
EOF

$SERVER_BIN -v -c /tmp/server_vhost_test.toml >/tmp/arp_server_vhost.log 2>&1 &
SERVER_PID=$!
sleep 2

echo "4. Starting ARP client..."
cat > /tmp/client_vhost_test.toml << EOF
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
name = "app_http"
type = "http"
local_ip = "127.0.0.1"
local_port = $LOCAL_HTTP_PORT
custom_domains = ["app.test.local"]

[[proxies]]
name = "app_https"
type = "https"
local_ip = "127.0.0.1"
local_port = $LOCAL_HTTPS_PORT
custom_domains = ["secure.test.local"]
EOF

$CLIENT_BIN -v -c /tmp/client_vhost_test.toml >/tmp/arp_client_vhost.log 2>&1 &
CLIENT_PID=$!
sleep 3

echo "5. Testing HTTP vhost routing..."
HTTP_RESP=$(curl -s -H "Host: app.test.local" "http://127.0.0.1:$VHOST_HTTP_PORT/" || true)
if [[ "$HTTP_RESP" != *"arp-http-ok"* ]]; then
    echo "✗ HTTP vhost test failed"
    echo "--- HTTP response ---"
    echo "$HTTP_RESP"
    echo "--- Server log ---"
    cat /tmp/arp_server_vhost.log || true
    echo "--- Client log ---"
    cat /tmp/arp_client_vhost.log || true
    exit 1
fi
echo "✓ HTTP vhost test passed"

echo "6. Testing HTTPS vhost routing (SNI)..."
HTTPS_RESP=$(printf "GET / HTTP/1.1\r\nHost: secure.test.local\r\nConnection: close\r\n\r\n" | \
    openssl s_client -connect "127.0.0.1:$VHOST_HTTPS_PORT" -servername secure.test.local -quiet 2>/tmp/arp_https_client.log || true)
if [[ "$HTTPS_RESP" != *"200 ok"* && "$HTTPS_RESP" != *"200 OK"* ]]; then
    echo "✗ HTTPS vhost test failed"
    echo "--- HTTPS response ---"
    echo "$HTTPS_RESP"
    echo "--- OpenSSL client log ---"
    cat /tmp/arp_https_client.log || true
    echo "--- Server log ---"
    cat /tmp/arp_server_vhost.log || true
    echo "--- Client log ---"
    cat /tmp/arp_client_vhost.log || true
    exit 1
fi
echo "✓ HTTPS vhost test passed"

echo ""
echo "=== VHost E2E tests passed! ==="
