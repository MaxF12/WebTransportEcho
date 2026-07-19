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

if [[ "${WT_GREASE_DIFFERENTIAL:-1}" == "1" ]]; then
  public_host="${WT_CERT_HOST:-wttest.quicast.de}"
  export WT_GREASE_CONTROL_BASE="${WT_GREASE_CONTROL_BASE:-https://${public_host}:${WT_QUICHE_CONTROL_PORT:-9447}}"
  export WT_GREASE_ENABLED_BASE="${WT_GREASE_ENABLED_BASE:-https://${public_host}:${WT_QUICHE_GREASE_PORT:-9448}}"
else
  unset WT_GREASE_CONTROL_BASE WT_GREASE_ENABLED_BASE
fi

exec "${NODE_BIN}" "${APP_DIR}/scripts/serve-page.mjs"
