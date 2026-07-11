#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERT_DIR="${ROOT}/certs"

mkdir -p "${CERT_DIR}"

openssl ecparam \
  -name prime256v1 \
  -genkey \
  -noout \
  -out "${CERT_DIR}/wt-short-key.pem"

openssl req \
  -x509 \
  -new \
  -sha256 \
  -days 7 \
  -key "${CERT_DIR}/wt-short-key.pem" \
  -out "${CERT_DIR}/wt-short.pem" \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1"

echo "Wrote:"
echo "  ${CERT_DIR}/wt-short.pem"
echo "  ${CERT_DIR}/wt-short-key.pem"
