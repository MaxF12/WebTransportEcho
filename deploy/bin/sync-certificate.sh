#!/usr/bin/env bash
set -Eeuo pipefail
umask 0027

ENV_FILE="${WT_ENV_FILE:-/etc/quicast/wttest.env}"
if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ENV_FILE}"
  set +a
fi

log() {
  printf 'quicast-wttest-cert: %s\n' "$*"
}

fail() {
  log "error: $*" >&2
  exit 1
}

for command in cmp cut dirname find getent head install mktemp mv openssl rm sort systemctl; do
  command -v "${command}" >/dev/null 2>&1 || fail "missing required command: ${command}"
done

WT_CERT_HOST="${WT_CERT_HOST:-wttest.quicast.de}"
WT_CERT="${WT_CERT:-/etc/quicast/wttest/${WT_CERT_HOST}.crt}"
WT_KEY="${WT_KEY:-/etc/quicast/wttest/${WT_CERT_HOST}.key}"
WT_CADDY_CERT_STORE="${WT_CADDY_CERT_STORE:-/var/lib/caddy/.local/share/caddy/certificates}"
WT_CERT_MIN_VALID_SECONDS="${WT_CERT_MIN_VALID_SECONDS:-3600}"

find_caddy_certificate() {
  find "${WT_CADDY_CERT_STORE}" \
    -path "*/${WT_CERT_HOST}/${WT_CERT_HOST}.crt" \
    -type f -printf '%T@ %p\n' 2>/dev/null | sort -nr | head -1 | cut -d' ' -f2-
}

certificate_key_matches() {
  local certificate="$1"
  local key="$2"
  local certificate_hash
  local key_hash

  certificate_hash="$(openssl x509 -in "${certificate}" -pubkey -noout 2>/dev/null | openssl pkey -pubin -outform DER 2>/dev/null | openssl sha256)"
  key_hash="$(openssl pkey -in "${key}" -pubout -outform DER 2>/dev/null | openssl sha256)"
  [[ -n "${certificate_hash}" && "${certificate_hash}" == "${key_hash}" ]]
}

cert_source="${WT_CERT_SOURCE_CERT:-}"
key_source="${WT_CERT_SOURCE_KEY:-}"

if [[ -z "${cert_source}" && -z "${key_source}" ]]; then
  if [[ ! -d "${WT_CADDY_CERT_STORE}" ]]; then
    log "Caddy certificate store is not available yet: ${WT_CADDY_CERT_STORE}"
    exit 0
  fi
  cert_source="$(find_caddy_certificate)"
  if [[ -z "${cert_source}" ]]; then
    log "Caddy has not obtained a certificate for ${WT_CERT_HOST} yet"
    exit 0
  fi
  key_source="$(dirname "${cert_source}")/${WT_CERT_HOST}.key"
elif [[ -z "${cert_source}" || -z "${key_source}" ]]; then
  fail "WT_CERT_SOURCE_CERT and WT_CERT_SOURCE_KEY must either both be set or both be empty"
fi

[[ -f "${cert_source}" ]] || fail "certificate source does not exist: ${cert_source}"
[[ -f "${key_source}" ]] || fail "private-key source does not exist: ${key_source}"
getent group quicast-wttest >/dev/null || fail "quicast-wttest group does not exist"
certificate_key_matches "${cert_source}" "${key_source}" || fail "certificate and private key do not match"
openssl x509 -in "${cert_source}" -noout -checkhost "${WT_CERT_HOST}" >/dev/null 2>&1 ||
  fail "certificate does not cover ${WT_CERT_HOST}"
openssl x509 -in "${cert_source}" -noout -checkend "${WT_CERT_MIN_VALID_SECONDS}" >/dev/null 2>&1 ||
  fail "certificate expires in less than ${WT_CERT_MIN_VALID_SECONDS} seconds"

if [[ -f "${WT_CERT}" && -f "${WT_KEY}" ]] &&
  cmp -s "${cert_source}" "${WT_CERT}" && cmp -s "${key_source}" "${WT_KEY}"; then
  exit 0
fi

install -d -m 0750 -o root -g quicast-wttest "$(dirname "${WT_CERT}")" "$(dirname "${WT_KEY}")"
cert_temp="$(mktemp "$(dirname "${WT_CERT}")/.${WT_CERT_HOST}.crt.XXXXXX")"
key_temp="$(mktemp "$(dirname "${WT_KEY}")/.${WT_CERT_HOST}.key.XXXXXX")"
trap 'rm -f -- "${cert_temp:-}" "${key_temp:-}"' EXIT
install -m 0640 -o root -g quicast-wttest "${cert_source}" "${cert_temp}"
install -m 0640 -o root -g quicast-wttest "${key_source}" "${key_temp}"
mv -f "${cert_temp}" "${WT_CERT}"
mv -f "${key_temp}" "${WT_KEY}"
trap - EXIT

log "synchronized ${WT_CERT_HOST} certificate"
if systemctl is-enabled --quiet quicast-wttest-h3.service; then
  systemctl restart quicast-wttest-h3.service
fi
if systemctl is-active --quiet quicast-wttest-page.service; then
  systemctl restart quicast-wttest-page.service
fi
