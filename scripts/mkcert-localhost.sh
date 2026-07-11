#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERT_DIR="${ROOT}/certs"
MKCERT_ARGS=()
CERT_NAME="localhost"

mkdir -p "${CERT_DIR}"

if [[ "${SKIP_MKCERT_INSTALL:-0}" == "1" ]]; then
  echo "Skipping mkcert -install because SKIP_MKCERT_INSTALL=1"
else
  mkcert -install
fi

if [[ "${MKCERT_ECDSA:-0}" == "1" ]]; then
  MKCERT_ARGS+=("-ecdsa")
  CERT_NAME="localhost-ecdsa"
fi

mkcert \
  "${MKCERT_ARGS[@]}" \
  -cert-file "${CERT_DIR}/${CERT_NAME}.pem" \
  -key-file "${CERT_DIR}/${CERT_NAME}-key.pem" \
  localhost 127.0.0.1 ::1

echo "Wrote:"
echo "  ${CERT_DIR}/${CERT_NAME}.pem"
echo "  ${CERT_DIR}/${CERT_NAME}-key.pem"
