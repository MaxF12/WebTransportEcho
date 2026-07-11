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
PYTHON_BIN="${APP_DIR}/.venv/bin/python"

: "${WT_CERT:?WT_CERT is required}"
: "${WT_KEY:?WT_KEY is required}"

args=(
  "${APP_DIR}/scripts/aioquic-wt-echo.py"
  --host "${WT_LISTEN_HOST:-::}"
  --port "${WT_LISTEN_PORT:-9446}"
  --cert "${WT_CERT}"
  --key "${WT_KEY}"
  --idle-timeout "${WT_IDLE_TIMEOUT:-60}"
  --max-data "${WT_MAX_DATA:-8388608}"
  --max-stream-data "${WT_MAX_STREAM_DATA:-8388608}"
  --max-datagram-frame-size "${WT_MAX_DATAGRAM_FRAME_SIZE:-65536}"
  --settings-profile "${WT_SETTINGS_PROFILE:-yggdrasil}"
  --webtransport-max-sessions "${WT_WEBTRANSPORT_MAX_SESSIONS:-1024}"
)

[[ "${WT_RESET_STREAM_AT:-1}" != "1" ]] || args+=(--reset-stream-at-tp)
[[ "${WT_FLUSH_SETTINGS_ON_NEGOTIATE:-1}" == "1" ]] || args+=(--no-flush-settings-on-negotiate)
[[ "${WT_RETRY:-0}" != "1" ]] || args+=(--retry)
[[ "${WT_QUANTUM_READINESS:-0}" != "1" ]] || args+=(--quantum-readiness)
[[ "${WT_VERBOSE:-0}" != "1" ]] || args+=(--verbose)
[[ -z "${WT_QLOG_DIR:-}" ]] || args+=(--qlog-dir "${WT_QLOG_DIR}")
[[ -z "${WT_SECRETS_LOG:-}" ]] || args+=(--secrets-log "${WT_SECRETS_LOG}")

exec "${PYTHON_BIN}" "${args[@]}"
