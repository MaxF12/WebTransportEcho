#!/usr/bin/env bash
set -Eeuo pipefail
umask 0022

log() {
  printf '\n==> %s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[[ "${EUID}" -eq 0 ]] || fail "run this installer as root, for example with sudo"

for command in basename getent git groupadd id install node python3 systemctl useradd; do
  command -v "${command}" >/dev/null 2>&1 || fail "missing required command: ${command}"
done

SOURCE_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
APP_ROOT=/opt/quicast/webtransport-echo
ENV_FILE=/etc/quicast/wttest.env
TLS_DIR=/etc/quicast/wttest
STATE_DIR=/var/lib/quicast-wttest

[[ "${SOURCE_ROOT}" == "${APP_ROOT}" ]] ||
  fail "clone this repository to ${APP_ROOT}; current checkout is ${SOURCE_ROOT}"
[[ -d "${SOURCE_ROOT}/.git" ]] || fail "${SOURCE_ROOT} is not a Git checkout"

python3 -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)' ||
  fail "Python 3.10 or newer is required"
node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 22 ? 0 : 1)' ||
  fail "Node.js 22 or newer is required"
[[ -x /usr/bin/node ]] || fail "Node.js must be available at /usr/bin/node for the systemd service"

if ! getent group quicast-wttest >/dev/null; then
  log "Creating quicast-wttest system group"
  groupadd --system quicast-wttest
fi
if ! id quicast-wttest >/dev/null 2>&1; then
  log "Creating quicast-wttest system user"
  useradd --system --home "${STATE_DIR}" --shell /usr/sbin/nologin --gid quicast-wttest quicast-wttest
fi

log "Installing pinned Python environment"
if [[ ! -x "${SOURCE_ROOT}/.venv/bin/python" ]]; then
  python3 -m venv "${SOURCE_ROOT}/.venv" ||
    fail "could not create a virtual environment; install the distribution's python3-venv package"
fi
"${SOURCE_ROOT}/.venv/bin/python" -m pip install \
  --disable-pip-version-check \
  --requirement "${SOURCE_ROOT}/requirements.lock"

install -d -m 0750 -o root -g quicast-wttest "${TLS_DIR}"
install -d -m 0750 -o quicast-wttest -g quicast-wttest "${STATE_DIR}"
if [[ ! -f "${ENV_FILE}" ]]; then
  log "Installing initial ${ENV_FILE}"
  install -m 0640 -o root -g quicast-wttest "${SOURCE_ROOT}/deploy/wttest.env.example" "${ENV_FILE}"
else
  log "Keeping existing ${ENV_FILE}"
fi

set -a
# shellcheck source=/dev/null
source "${ENV_FILE}"
set +a

grease_enabled="${WT_GREASE_DIFFERENTIAL:-1}"
if [[ "${grease_enabled}" == "1" ]]; then
  for command in cargo cmake; do
    command -v "${command}" >/dev/null 2>&1 ||
      fail "missing tokio-quiche build dependency: ${command}"
  done
  log "Building pinned tokio-quiche GREASE backend"
  cargo build \
    --locked \
    --release \
    --manifest-path "${SOURCE_ROOT}/Cargo.toml" \
    --bin tokio-quiche-wt-echo
fi

log "Installing systemd units"
for unit in "${SOURCE_ROOT}"/deploy/systemd/*; do
  install -m 0644 "${unit}" "/etc/systemd/system/$(basename "${unit}")"
done
systemctl daemon-reload
systemctl enable quicast-wttest-page.service quicast-wttest-h3.service >/dev/null
if [[ "${grease_enabled}" == "1" ]]; then
  systemctl enable \
    quicast-wttest-quiche@control.service \
    quicast-wttest-quiche@grease.service >/dev/null
else
  systemctl disable --now \
    quicast-wttest-quiche@control.service \
    quicast-wttest-quiche@grease.service >/dev/null 2>&1 || true
fi
systemctl enable --now quicast-wttest-cert-sync.timer >/dev/null
systemctl restart quicast-wttest-page.service
systemctl start quicast-wttest-cert-sync.service

if [[ -f "${WT_CERT:-}" && -f "${WT_KEY:-}" ]]; then
  services=(quicast-wttest-h3.service)
  if [[ "${grease_enabled}" == "1" ]]; then
    services+=(
      quicast-wttest-quiche@control.service
      quicast-wttest-quiche@grease.service
    )
  fi
  systemctl restart "${services[@]}"
else
  log "The WebTransport services are enabled but waiting for their certificate"
  log "Install the Caddy vhost, then run: systemctl start quicast-wttest-cert-sync.service"
fi

commit="$(git -C "${SOURCE_ROOT}" rev-parse --short HEAD)"
log "Deployed Git commit ${commit}"
printf 'Config: %s\n' "${ENV_FILE}"
printf 'Page:   systemctl status quicast-wttest-page.service\n'
printf 'H3:     systemctl status quicast-wttest-h3.service\n'
if [[ "${grease_enabled}" == "1" ]]; then
  printf 'Control: systemctl status quicast-wttest-quiche@control.service\n'
  printf 'GREASE: systemctl status quicast-wttest-quiche@grease.service\n'
fi
printf 'Check:  %s\n' "${APP_ROOT}/deploy/check-node.sh"
