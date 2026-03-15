#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust TCP MUX E2E Test ==="
echo ""

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$PROJECT_DIR/target/debug/arps"
CLIENT_BIN="$PROJECT_DIR/target/debug/arpc"
TEST_CONTROL_PORT=27110
TEST_REMOTE_PORT=26110
TEST_LOCAL_PORT=22221
PARALLEL_CONN=20

echo "Building debug binaries..."
cargo build >/dev/null

cleanup() {
    echo "Cleaning up..."
    kill ${SERVER_PID:-0} 2>/dev/null || true
    kill ${CLIENT_PID:-0} 2>/dev/null || true
    kill ${ECHO_PID:-0} 2>/dev/null || true
}
trap cleanup EXIT

echo "1. Starting local concurrent echo server on ${TEST_LOCAL_PORT}..."
python3 - <<'PY' &
import socketserver
import threading

HOST = "127.0.0.1"
PORT = 22221

class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            data = self.request.recv(65535)
            if not data:
                return
            self.request.sendall(data)

class ThreadedTCPServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True

server = ThreadedTCPServer((HOST, PORT), Echo)
t = threading.Thread(target=server.serve_forever, daemon=False)
t.start()
t.join()
PY
ECHO_PID=$!
sleep 1

echo "2. Starting ARP server..."
cat > /tmp/server_test_mux.toml << EOF
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
start = $TEST_REMOTE_PORT
end = $TEST_REMOTE_PORT
EOF

$SERVER_BIN -v -c /tmp/server_test_mux.toml >/tmp/arp_server_mux.log 2>&1 &
SERVER_PID=$!
sleep 2
kill -0 $SERVER_PID

echo "3. Starting ARP client with tcp_mux..."
cat > /tmp/client_test_mux.toml << EOF
server_addr = "127.0.0.1"
server_port = $TEST_CONTROL_PORT
log_level = "info"

[auth]
method = "token"
token = "test_token_123456"

[transport]
protocol = "tcp"
tcp_mux = true
pool_count = 1

[[proxies]]
name = "test_mux_echo"
type = "tcp"
local_ip = "127.0.0.1"
local_port = $TEST_LOCAL_PORT
remote_port = $TEST_REMOTE_PORT
EOF

$CLIENT_BIN -v -c /tmp/client_test_mux.toml >/tmp/arp_client_mux.log 2>&1 &
CLIENT_PID=$!
sleep 3
kill -0 $CLIENT_PID

echo "4. Running ${PARALLEL_CONN} parallel connections through one mux tunnel..."
export TEST_REMOTE_PORT
export PARALLEL_CONN
python3 - <<'PY'
import os
import socket
import threading
import sys

remote_port = int(os.environ["TEST_REMOTE_PORT"])
parallel = int(os.environ["PARALLEL_CONN"])
errs = []

def worker(i: int):
    payload = f"mux-msg-{i}".encode()
    s = socket.create_connection(("127.0.0.1", remote_port), timeout=5)
    try:
        s.sendall(payload)
        got = s.recv(65535)
        if got != payload:
            errs.append(f"idx={i} got={got!r} expected={payload!r}")
    finally:
        s.close()

threads = [threading.Thread(target=worker, args=(i,)) for i in range(parallel)]
for t in threads:
    t.start()
for t in threads:
    t.join()

if errs:
    print("FAIL")
    for e in errs:
        print(e)
    sys.exit(1)
print("PASS")
PY

echo "5. Sanity check: client/server still alive..."
kill -0 $SERVER_PID
kill -0 $CLIENT_PID

echo ""
echo "=== TCP MUX E2E passed ==="
