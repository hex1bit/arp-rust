#!/bin/bash

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "usage: $0 <server_name> [ip]"
    echo "example: $0 example.com 1.2.3.4"
    exit 1
fi

SERVER_NAME="$1"
SERVER_IP="${2:-127.0.0.1}"

openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout server.key \
    -out server.crt \
    -days 365 \
    -subj "/CN=${SERVER_NAME}" \
    -addext "subjectAltName=DNS:${SERVER_NAME},IP:${SERVER_IP}" \
    -addext "basicConstraints=CA:FALSE" \
    -addext "keyUsage=digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth"

echo "generated:"
echo "  $(pwd)/server.crt"
echo "  $(pwd)/server.key"
