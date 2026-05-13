#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust STCP + UDP E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27120
TEST_STCP_REMOTE_PORT=26120
TEST_SUDP_REMOTE_PORT=26121
TEST_STCP_LOCAL_PORT=22230
TEST_SUDP_LOCAL_PORT=22240
# Visitor binds a local port that proxies inbound traffic to the STCP/SUDP service
STCP_VISITOR_BIND_PORT=22250
SUDP_VISITOR_BIND_PORT=22251

echo "Building debug binaries..."
cargo build >/dev/null 2>&1

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0}        2>/dev/null || true
    kill ${CLIENT_PID:-0}        2>/dev/null || true
    kill ${VISITOR_PID:-0}       2>/dev/null || true
    kill ${STCP_BACKEND_PID:-0}  2>/dev/null || true
    kill ${SUDP_BACKEND_PID:-0}  2>/dev/null || true
}
trap cleanup EXIT

# ── 1. Start the STCP echo backend (python persistent TCP echo server) ──────
echo "1. Starting STCP TCP echo backend on ${TEST_STCP_LOCAL_PORT}..."
python3 - <<'PY' &
import socketserver, threading
class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            data = self.request.recv(65535)
            if not data:
                return
            self.request.sendall(data)
class ThreadedTCP(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
srv = ThreadedTCP(("127.0.0.1", 22230), Echo)
t = threading.Thread(target=srv.serve_forever, daemon=False)
t.start()
t.join()
PY
STCP_BACKEND_PID=$!

# ── 2. Start the SUDP echo backend ──────────────────────────────────────────
echo "2. Starting SUDP echo backend on ${TEST_SUDP_LOCAL_PORT}..."
python3 - <<'PY' &
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 22240))
while True:
    data, addr = s.recvfrom(65535)
    s.sendto(data, addr)
PY
SUDP_BACKEND_PID=$!
sleep 1

# ── 3. Start ARP server ──────────────────────────────────────────────────────
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

# ── 4. Start provider client (exposes the local STCP/SUDP services) ──────────
echo "4. Starting provider ARP client with STCP/SUDP..."
cat > /tmp/client_test_stcp_sudp_provider.toml << EOF
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
type = "udp"
local_ip = "127.0.0.1"
local_port = $TEST_SUDP_LOCAL_PORT
remote_port = $TEST_SUDP_REMOTE_PORT
EOF

$CLIENT_BIN -v -c /tmp/client_test_stcp_sudp_provider.toml >/tmp/arp_client_stcp_sudp_provider.log 2>&1 &
CLIENT_PID=$!
sleep 3
kill -0 $CLIENT_PID

# ── 5. Start visitor client (binds local ports to access STCP/SUDP) ──────────
echo "5. Starting visitor ARP client..."
cat > /tmp/client_test_stcp_sudp_visitor.toml << EOF
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

[[visitors]]
name = "stcp_visitor"
type = "stcp"
server_name = "stcp_test"
sk = "stcp_secret"
bind_addr = "127.0.0.1"
bind_port = $STCP_VISITOR_BIND_PORT
EOF

$CLIENT_BIN -v -c /tmp/client_test_stcp_sudp_visitor.toml >/tmp/arp_client_stcp_sudp_visitor.log 2>&1 &
VISITOR_PID=$!
sleep 3
kill -0 $VISITOR_PID

# ── 6. Test STCP (via visitor local port → STCP tunnel → backend echo) ───────
echo "6. Testing STCP (via visitor port ${STCP_VISITOR_BIND_PORT})..."
export STCP_VISITOR_BIND_PORT
STCP_RESULT=$(python3 - <<'PY'
import os, socket, sys
port = int(os.environ["STCP_VISITOR_BIND_PORT"])
payload = b"hello-stcp"
try:
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    s.sendall(payload)
    got = s.recv(65535)
    s.close()
    if got != payload:
        print(f"FAIL: got={got!r}")
        sys.exit(1)
    print("PASS")
except Exception as e:
    print(f"FAIL: {e}")
    sys.exit(1)
PY
)
if [ "$STCP_RESULT" != "PASS" ]; then
    echo "✗ STCP failed: $STCP_RESULT"
    echo "--- provider log ---"
    tail -30 /tmp/arp_client_stcp_sudp_provider.log || true
    echo "--- visitor log ---"
    tail -30 /tmp/arp_client_stcp_sudp_visitor.log || true
    exit 1
fi
echo "✓ STCP passed"

# ── 7. Test UDP (via server's UDP port — plain udp proxy) ────────────────────
echo "7. Testing plain UDP proxy (direct to server port ${TEST_SUDP_REMOTE_PORT})..."
export TEST_SUDP_REMOTE_PORT
python3 - <<'PY'
import os, socket, sys
port = int(os.environ["TEST_SUDP_REMOTE_PORT"])
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(3)
payload = b"hello-udp"
try:
    s.sendto(payload, ("127.0.0.1", port))
    data, _ = s.recvfrom(65535)
    if data != payload:
        raise SystemExit(f"unexpected response: {data!r}")
    print("✓ UDP proxy passed")
except Exception as e:
    print(f"✗ UDP proxy failed: {e}", file=sys.stderr)
    sys.exit(1)
finally:
    s.close()
PY

echo ""
echo "=== STCP + UDP E2E passed ==="
