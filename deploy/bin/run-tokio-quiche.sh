#!/usr/bin/env bash
set -Eeuo pipefail

ENV_FILE="${WT_ENV_FILE:-/etc/quicast/wttest.env}"
if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ENV_FILE}"
  set +a
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
BINARY="${APP_DIR}/target/release/tokio-quiche-wt-echo"
MODE="${1:-}"

: "${WT_CERT:?WT_CERT is required}"
: "${WT_KEY:?WT_KEY is required}"
[[ -x "${BINARY}" ]] || {
  printf 'tokio-quiche backend is missing: %s\n' "${BINARY}" >&2
  exit 1
}

listen_host="${WT_QUICHE_LISTEN_HOST:-::}"
format_listen() {
  local port="$1"
  if [[ "${listen_host}" == \[*\] ]]; then
    printf '%s:%s' "${listen_host}" "${port}"
  elif [[ "${listen_host}" == *:* ]]; then
    printf '[%s]:%s' "${listen_host}" "${port}"
  else
    printf '%s:%s' "${listen_host}" "${port}"
  fi
}

case "${MODE}" in
  control)
    listen="$(format_listen "${WT_QUICHE_CONTROL_PORT:-9447}")"
    grease_flag=--no-grease
    qlog_dir="${WT_QUICHE_CONTROL_QLOG_DIR:-}"
    secrets_log="${WT_QUICHE_CONTROL_SECRETS_LOG:-}"
    ;;
  grease)
    listen="$(format_listen "${WT_QUICHE_GREASE_PORT:-9448}")"
    grease_flag=--grease
    qlog_dir="${WT_QUICHE_GREASE_QLOG_DIR:-}"
    secrets_log="${WT_QUICHE_GREASE_SECRETS_LOG:-}"
    ;;
  *)
    printf 'usage: %s control|grease\n' "$0" >&2
    exit 2
    ;;
esac

args=(
  --listen "${listen}"
  --cert "${WT_CERT}"
  --key "${WT_KEY}"
  --cc "${WT_QUICHE_CC:-cubic}"
  "${grease_flag}"
)

[[ "${WT_QUICHE_RESET_STREAM_AT:-1}" != "0" ]] || args+=(--no-reset-stream-at)
[[ "${WT_QUICHE_RETRY:-0}" != "1" ]] || args+=(--retry)
[[ -z "${qlog_dir}" ]] || args+=(--qlog-dir "${qlog_dir}")
[[ -z "${secrets_log}" ]] || args+=(--secrets-log "${secrets_log}")

export RUST_LOG="${WT_QUICHE_LOG:-info}"
exec "${BINARY}" "${args[@]}"
