#!/usr/bin/env bash
set -Eeuo pipefail

ENV_FILE="${WT_ENV_FILE:-/etc/quicast/wttest.env}"
APP_ROOT=/opt/quicast/webtransport-echo

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[[ -f "${ENV_FILE}" ]] || fail "missing environment file: ${ENV_FILE}"
set -a
# shellcheck source=/dev/null
source "${ENV_FILE}"
set +a

for command in awk curl openssl ss systemctl; do
  command -v "${command}" >/dev/null 2>&1 || fail "missing required command: ${command}"
done

for unit in quicast-wttest-page.service quicast-wttest-h3.service quicast-wttest-cert-sync.timer; do
  systemctl is-active --quiet "${unit}" || fail "inactive unit: ${unit}"
done

curl --fail --silent --show-error --max-time 5 "http://${PAGE_HOST:-127.0.0.1}:${PAGE_PORT:-8088}/healthz" >/dev/null ||
  fail "page health check failed"
if [[ "${WT_RESULTS_ENABLED:-1}" == "1" ]]; then
  curl --fail --silent --show-error --max-time 5 "http://${PAGE_HOST:-127.0.0.1}:${PAGE_PORT:-8088}/api/browser-results" >/dev/null ||
    fail "browser results API check failed"
fi

ss -H -lun | awk -v port="${WT_LISTEN_PORT:-9446}" '
  {
    local_address = $4
    if ($1 ~ /^(udp|udp6)$/) {
      local_address = $5
    }
    if (local_address ~ "(^|:|\\])" port "$") {
      found = 1
    }
  }
  END { exit found ? 0 : 1 }
' || fail "UDP ${WT_LISTEN_PORT:-9446} is not listening"

openssl x509 -in "${WT_CERT}" -noout -checkhost "${WT_CERT_HOST}" >/dev/null 2>&1 ||
  fail "installed certificate does not cover ${WT_CERT_HOST}"

health_args=(
  --connect-host "${WT_HEALTH_CONNECT_HOST:-::1}"
  --port "${WT_LISTEN_PORT:-9446}"
  --server-name "${WT_CERT_HOST}"
)
[[ -z "${WT_HEALTH_CA_FILE:-}" ]] || health_args+=(--ca-file "${WT_HEALTH_CA_FILE}")
[[ "${WT_HEALTH_INSECURE:-0}" != "1" ]] || health_args+=(--insecure)
"${APP_ROOT}/.venv/bin/python" "${APP_ROOT}/scripts/aioquic-h3-health.py" "${health_args[@]}"

curl --fail --silent --show-error --max-time 5 \
  "http://${PAGE_HOST:-127.0.0.1}:${PAGE_PORT:-8088}/matrix-config.json"
printf '\nquicast-wttest node checks passed\n'
