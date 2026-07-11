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
NODE_BIN="${NODE_BIN:-/usr/bin/node}"

[[ -x "${NODE_BIN}" ]] || {
  printf 'Node.js executable is missing: %s\n' "${NODE_BIN}" >&2
  exit 1
}

exec "${NODE_BIN}" "${APP_DIR}/scripts/serve-page.mjs"
