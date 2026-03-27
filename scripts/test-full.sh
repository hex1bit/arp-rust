#!/bin/bash

set -euo pipefail

echo "=== ARP-Rust Full Test Suite ==="

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

run_test() {
  local name="$1"
  shift
  echo "--- Running: $name ---"
  "$@"
  echo "✓ Passed: $name"
  echo
}

run_test "cargo test --workspace" cargo test --workspace
run_test "test_e2e.sh" bash test/test_e2e.sh
run_test "test_e2e_ws.sh" bash test/test_e2e_ws.sh
run_test "test_e2e_wss.sh" bash test/test_e2e_wss.sh
run_test "test_e2e_kcp.sh" bash test/test_e2e_kcp.sh
run_test "test_e2e_quic.sh" bash test/test_e2e_quic.sh
run_test "test_e2e_udp.sh" bash test/test_e2e_udp.sh
run_test "test_e2e_stcp_sudp.sh" bash test/test_e2e_stcp_sudp.sh
run_test "test_e2e_tcp_mux.sh" bash test/test_e2e_tcp_mux.sh
run_test "test_e2e_tcp_lb_health.sh" bash test/test_e2e_tcp_lb_health.sh
run_test "test_e2e_vhost.sh" bash test/test_e2e_vhost.sh
run_test "test_e2e_xtcp.sh" bash test/test_e2e_xtcp.sh
run_test "test_e2e_tls.sh" bash test/test_e2e_tls.sh
run_test "test_e2e_health.sh" bash test/test_e2e_health.sh
run_test "test_e2e_auth_acl.sh" bash test/test_e2e_auth_acl.sh

echo "=== Full test suite passed ==="
