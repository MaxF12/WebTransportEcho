# WebTransport Echo Lab

Scratch WebTransport handshake lab for Safari bring-up. This repo is intentionally independent of Yggdrasil, Bifrost, Ratatoskr, and production deploys.

## What Works Now

Use the aioquic server as the primary oracle. The first Rust/quinn+h3 attempt remains in `src/main.rs`, but it is not the baseline because Chrome reached CONNECT and then rejected `transport.ready` for every response variant. The aioquic stack gets Chrome through:

- `new WebTransport(...).ready`
- datagram echo
- `createBidirectionalStream()` echo
- `createUnidirectionalStream()` echo

That gives us a minimal known-good server to compare against Safari and Yggdrasil.

## Certificates

Safari should be tested with normal trust-store validation:

```sh
./scripts/mkcert-localhost.sh
```

This writes:

- `certs/localhost.pem`
- `certs/localhost-key.pem`

The certificate covers `localhost`, `127.0.0.1`, and `::1`. The script runs `mkcert -install` unless `SKIP_MKCERT_INSTALL=1` is set; on macOS that trust-store install can ask for your password.

Chromium and Firefox hash-mode diagnostics use a short-lived WebTransport certificate:

```sh
./scripts/make-short-lived-wt-cert.sh
```

This writes:

- `certs/wt-short.pem`
- `certs/wt-short-key.pem`

The generated certificate is P-256 ECDSA, SAN-only, and valid for 7 days. That mirrors Bifrost's local `serverCertificateHashes` path and avoids browser certificate-hash restrictions. This mode is not the Safari trust-store path.

### Firefox and mkcert

Firefox accepts the mkcert chain, but it disables HTTP/3 by default when the
validated chain ends at a non-built-in root. The controlling preference is:

```text
network.http.http3.disable_when_third_party_roots_found = true
```

That policy closes the QUIC connection before WebTransport `CONNECT`, so an
mkcert matrix can show `WebTransport` as available while every `ready` promise
fails. Constructor options and response paths cannot affect that boundary.

The preferred Firefox localhost route is the short-lived self-signed P-256
certificate plus `serverCertificateHashes`, using the hash-mode server/page
pair below. For a dedicated diagnostic Firefox profile only, setting the
preference to `false` also permits the mkcert H3 endpoint; restore the default
after the test. A publicly trusted `wttest.quicast.de` certificate does not need
either localhost workaround.

## Dependencies

Fast path with an installed aioquic:

```sh
python3 -m venv .venv
./.venv/bin/python -m pip install -r requirements.txt
```

If you want to use the local aioquic checkout from `/Users/mfranke/Devtools/Multicast/aioquic`, prepend it to `PYTHONPATH`:

```sh
PYTHONPATH=scripts/aioquic_stubs:/Users/mfranke/Devtools/Multicast/aioquic/src \
  ./.venv/bin/python scripts/aioquic-wt-echo.py --cert certs/localhost.pem --key certs/localhost-key.pem
```

The `scripts/aioquic_stubs` path only exists because that local checkout imports a multicast helper not needed by this lab. Upstream aioquic from `requirements.txt` should not need the stub path.

## Run

There are two intentionally separate modes.

### quiche / tokio-quiche Backend

The secondary backend uses the local checkout at
`../../Multicast/quiche/tokio-quiche`. It is intended to sit between the
working aioquic oracle and production Yggdrasil: same scratch page and response
variants, but quiche/tokio-quiche transport, H3, WebTransport session
registration, and response-flush behavior.

Run it on a separate port:

```sh
cargo run --bin tokio-quiche-wt-echo -- \
  --listen '[::]:9446' \
  --cert certs/localhost.pem \
  --key certs/localhost-key.pem \
  --qlog-dir qlogs/tokio-quiche-rsa \
  --secrets-log qlogs/tokio-quiche-rsa/secrets.log
```

This backend disables quiche GREASE by default. The current reducer is based on
the qlog observation that quiche sends an unknown server uni stream and reserved
HTTP/3 frames before the CONNECT response HEADERS, while the working aioquic
oracle has GREASE disabled. To reproduce the current quiche/Yggdrasil-shaped
post-200 Safari stall, add:

```sh
--grease
```

Use the existing HTTPS page server and point the matrix at the quiche backend:

```text
https://localhost:8443/?target=https://localhost:9446&paths=/wt/basic,/wt/protocol,/wt/capsule,/wt/init,/wt/yggdrasil,/wt/auto&variants=0,1,2,3,4&timeout=5000&autorun=1
```

For the narrow Safari/Yggdrasil-shaped comparison:

```text
https://localhost:8443/?target=https://localhost:9446&paths=/wt/yggdrasil,/wt/auto&variants=4&timeout=5000&autorun=1
```

Chrome should not use this mkcert/trust-store tokio-quiche target in the
in-app browser: that runtime has already rejected mkcert for WebTransport's
QUIC TLS path. For a Chrome tokio-quiche control, use the short-lived hash cert
on another port:

```sh
cargo run --bin tokio-quiche-wt-echo -- \
  --listen '[::]:9447' \
  --cert certs/wt-short.pem \
  --key certs/wt-short-key.pem \
  --qlog-dir qlogs/tokio-quiche-short \
  --secrets-log qlogs/tokio-quiche-short/secrets.log

PAGE_PORT=8444 WT_TARGET_CERT=certs/wt-short.pem node scripts/serve-page.mjs
```

Open:

```text
https://localhost:8444/?target=https://localhost:9447&paths=/wt/yggdrasil,/wt/auto&variants=6&timeout=5000&autorun=1
```

This backend enables `RESET_STREAM_AT` by default because that was the local
Safari gate. To reproduce the pre-CONNECT Safari failure, pass:

```sh
--no-reset-stream-at
```

The backend logs tokio-quiche's WebTransport diagnostics, including
`ConnectSessionRegistered`, `H3HeadersFlushedToQuic`, and WT stream-prefix
classification. It also echoes:

- H3 datagrams when tokio-quiche exposes a datagram flow
- WebTransport bidirectional streams by writing payload bytes back on the same
  QUIC stream
- WebTransport unidirectional streams by opening server uni stream IDs starting
  at 15, matching the usual post-control-stream WebTransport shape

### Safari / Trust-Store Mode

Use mkcert and do not select the certificate-hash variants. This is the mode Safari needs:

```sh
PYTHONPATH=scripts/aioquic_stubs:/Users/mfranke/Devtools/Multicast/aioquic/src \
  ./.venv/bin/python scripts/aioquic-wt-echo.py \
  --cert certs/localhost.pem \
  --key certs/localhost-key.pem \
  --host :: \
  --port 9443 \
  --settings-profile yggdrasil \
  --reset-stream-at-tp
```

Start the HTTPS test page:

```sh
node scripts/serve-page.mjs
```

Open:

```text
https://localhost:8443/
```

The page disables the certificate-hash variants in this mode because the mkcert certificate is CA-issued, RSA, and long-lived.

The trust-store server supports qlog diagnostics:

```sh
PYTHONPATH=scripts/aioquic_stubs:/Users/mfranke/Devtools/Multicast/aioquic/src \
  ./.venv/bin/python scripts/aioquic-wt-echo.py \
  --cert certs/localhost.pem \
  --key certs/localhost-key.pem \
  --host :: \
  --port 9443 \
  --qlog-dir qlogs/mkcert-rsa \
  --secrets-log qlogs/mkcert-rsa/secrets.log \
  --settings-profile yggdrasil \
  --reset-stream-at-tp
```

To isolate whether Safari dislikes the RSA mkcert certificate shape, generate and test an ECDSA mkcert certificate:

```sh
SKIP_MKCERT_INSTALL=1 MKCERT_ECDSA=1 ./scripts/mkcert-localhost.sh

PYTHONPATH=scripts/aioquic_stubs:/Users/mfranke/Devtools/Multicast/aioquic/src \
  ./.venv/bin/python scripts/aioquic-wt-echo.py \
  --cert certs/localhost-ecdsa.pem \
  --key certs/localhost-ecdsa-key.pem \
  --host :: \
  --port 9445 \
  --qlog-dir qlogs/mkcert-ecdsa \
  --secrets-log qlogs/mkcert-ecdsa/secrets.log \
  --settings-profile yggdrasil \
  --reset-stream-at-tp

PAGE_PORT=8445 WT_CERT=certs/localhost-ecdsa.pem WT_KEY=certs/localhost-ecdsa-key.pem \
  node scripts/serve-page.mjs
```

Open this ECDSA Safari comparison:

```text
https://localhost:8445/?target=https://localhost:9445&paths=/wt/basic,/wt/protocol,/wt/capsule,/wt/init,/wt/yggdrasil,/wt/auto&variants=0,1,2,3,4&timeout=5000&autorun=1
```

The fastest Safari discriminator after the Yggdrasil comparison is the narrow
Yggdrasil-shaped case:

```text
https://localhost:8445/?target=https://localhost:9445&paths=/wt/yggdrasil,/wt/auto&variants=4&timeout=5000&autorun=1
```

Then repeat with RSA:

```text
https://localhost:8443/?target=https://localhost:9443&paths=/wt/yggdrasil,/wt/auto&variants=4&timeout=5000&autorun=1
```

### Chrome and Firefox / Hash-Mode Baseline

Use a separate port pair for the Chrome and Firefox `serverCertificateHashes` path:

```sh
./scripts/make-short-lived-wt-cert.sh

PYTHONPATH=scripts/aioquic_stubs:/Users/mfranke/Devtools/Multicast/aioquic/src \
  ./.venv/bin/python scripts/aioquic-wt-echo.py \
  --cert certs/wt-short.pem \
  --key certs/wt-short-key.pem \
  --host :: \
  --port 9444 \
  --settings-profile yggdrasil \
  --reset-stream-at-tp \
  --webtransport-max-sessions 1024

PAGE_PORT=8444 \
WT_TARGET_BASE=https://localhost:9444 \
WT_TARGET_CERT=certs/wt-short.pem \
node scripts/serve-page.mjs
```

Open:

```text
https://localhost:8444/?target=https://localhost:9444&paths=/wt/basic&variants=5,6&timeout=4000&autorun=1
```

The Chrome and Firefox control for the Yggdrasil-shaped response/settings path is:

```text
https://localhost:8444/?target=https://localhost:9444&paths=/wt/yggdrasil,/wt/auto&variants=6&timeout=4000&autorun=1
```

Use this Safari-focused URL against the mkcert server:

```text
https://localhost:8443/?paths=/wt/basic,/wt/protocol,/wt/capsule,/wt/init&variants=0,1,2,3,4&timeout=5000&autorun=1
```

## Browser Test Matrix

The page targets `https://localhost:9443` by default and tests:

- `/wt/basic`
- `/wt/protocol`
- `/wt/capsule`
- `/wt/init`
- `/wt/draft`
- `/wt/h3-token`
- `/wt/yggdrasil`
- `/wt/auto`

`Run Selected` preserves the original constructor presets and numbered
`variants=` query links:

- no options
- `{ requireUnreliable: true }`
- `{ allowPooling: false, requireUnreliable: true }`
- `{ allowPooling: false, requireUnreliable: true, congestionControl: "low-latency" }`
- same plus `protocols: ["quicast-wt-v0"]`
- Chromium-only short-lived certificate hash plus `{ requireUnreliable: true }`
- Chromium-only short-lived certificate hash plus the QUICast production option shape

`Run All Combinations` generates the full power set of these independent option
axes:

- `allowPooling: false`
- `requireUnreliable: true`
- `congestionControl: "low-latency"`
- `protocols: ["quicast-wt-v0"]`
- `serverCertificateHashes` when the page server advertises a compatible short-lived ECDSA certificate

That is 16 combinations per response path in normal trust-store mode and 32 in
certificate-hash mode. The empty set is the real no-options constructor call.
Cases run sequentially and can be stopped while a failure-heavy matrix is in
progress. Hash-authenticated cases run before intentionally untrusted cases so
Chromium's certificate-failure throttling cannot poison valid hash cases. The
default 150 ms gap between cases is adjustable in the page or with `delay=`.

The page detects the current browser, secure-context status, and whether a
constructible `WebTransport` API exists before starting. An API-absent browser
such as a Brave build with WebTransport disabled produces one explicit
capability verdict and starts no network cases. If the API exists but every
handshake fails, the verdict instead says that the API is exposed but no tested
combination reached `transport.ready`.

Each result logs constructor status, `transport.ready`, `transport.closed`,
`transport.protocol`, `transport.reliability`, datagram echo, bidi stream echo,
and uni stream echo. Exhaustive runs also compute per-path option effects using
observed wording such as `ready only with requireUnreliable=true`; the copied
JSON includes the browser profile, runtime config, exact options, summary, and
all cases.

## Anonymous Browser Overview

Complete exhaustive runs against the page's configured target can publish an
anonymous aggregate snapshot to `/results.html`. Sharing is enabled by default
and can be disabled before a run or with `?publish=0`.

The server retains one latest snapshot for each browser family and a bounded
change log containing only material differences from the prior snapshot. It
stores the browser family and major version, API availability, aggregate stage
counts, per-path counts, derived option signals, and the normalized stage
outcome for every exhaustive option combination. The overview renders that
matrix per browser and response path. It does not retain raw user agents, IP
addresses, platform strings, target URLs, exception text, individual case
payloads, or prior full reports. Repeated results with identical behavior only
refresh the latest timestamp and do not add a change event.

Selected, cancelled, incomplete, partial-path, and custom-target runs stay
local. The server validates the reduced schema again before writing it
atomically to `WT_RESULTS_FILE`; the public deployment uses
`/var/lib/quicast-wttest/browser-results.json`.

Exhaustive autorun example:

```text
https://localhost:8443/?target=https://localhost:9446&paths=/wt/basic,/wt/yggdrasil&mode=exhaustive&timeout=5000&delay=150&autorun=1
```

The alias `exhaustive=1` is also accepted. Query `target`, `paths`, `timeout`,
and `delay` continue to override page defaults.

## Deployable wttest Host

The page is no longer bound to a compile-time localhost target. The local page
server exposes `/matrix-config.json`, populated by `WT_TARGET_BASE`,
`WT_DEFAULT_TIMEOUT_MS`, `WT_BETWEEN_CASES_MS`, and `WT_DEFAULT_PATHS`. It can
also run without local TLS via `PAGE_TLS=0` when an existing HTTPS edge proxies
to its loopback TCP listener.

The isolated `wttest.quicast.de` layout and environment example are in
[`docs/wttest-deployment.md`](docs/wttest-deployment.md) and
[`deploy/wttest.env.example`](deploy/wttest.env.example). The recommended split
keeps the page on HTTPS/TCP 443 and sends WebTransport directly to the scratch
backend on UDP 9446, avoiding a collision with an existing Caddy HTTP/3 listener
on UDP 443. No Bifrost or production files are changed by this repository.

Deploy the repository on the selected node with:

```sh
sudo git clone git@github.com:MaxF12/WebTransportEcho.git \
  /opt/quicast/webtransport-echo
sudo /opt/quicast/webtransport-echo/deploy/install-node.sh
```

The Git checkout uses pinned aioquic 1.3.0, installs isolated systemd services,
synchronizes a public Caddy certificate without exposing Caddy's private data
tree to the runtime, and includes a real H3 health probe. The probe also opens
a WebTransport session and verifies datagram and bidirectional echoes. Later
deploys are a fast-forward pull followed by another idempotent
`deploy/install-node.sh` run.
The copy/paste integration brief is
[`docs/BIFROST-HANDOFF.md`](docs/BIFROST-HANDOFF.md).

## Response Variants

The aioquic server accepts `CONNECT` on `/wt/*` and changes only response headers by path:

| Path | Response headers |
| --- | --- |
| `/wt/basic` | `:status=200` only |
| `/wt/protocol` | `wt-protocol: "quicast-wt-v0"` |
| `/wt/capsule` | `wt-protocol: "quicast-wt-v0"`, `capsule-protocol: ?1` |
| `/wt/init` | protocol, capsule, `webtransport-init: u=8388608, bl=8388608, br=8388608` |
| `/wt/draft` | `sec-webtransport-http3-draft: draft02` |
| `/wt/h3-token` | same as basic; the server accepts both `webtransport` and `webtransport-h3`, but cannot force the client token |
| `/wt/yggdrasil` | `server: yggdrasil`, protocol, capsule, init, and reflected CORS origin in Yggdrasil's negotiated-protocol shape |
| `/wt/auto` | Yggdrasil shape only when `wt-available-protocols` contains `quicast-wt-v0` |

## H3 Settings Profiles

The server defaults to aioquic's native H3/WebTransport SETTINGS profile. For
Safari comparison, pass:

```sh
--settings-profile yggdrasil
```

That adds the WebTransport SETTINGS shape observed from Yggdrasil:

- `h3_datagram_draft04=1`
- `wt_enabled=1`
- `webtransport_max_sessions_draft07=1`
- `webtransport_max_sessions=1`
- `wt_initial_max_data=8388608`
- `wt_initial_max_streams_uni=100`
- `wt_initial_max_streams_bidi=100`

The default max-sessions value remains `1` for handshake fidelity. A high-volume
matrix control can opt into `--webtransport-max-sessions 1024`; the advertised
value is logged with the rest of the server SETTINGS.

For the closer Yggdrasil transport-parameter match, also pass:

```sh
--reset-stream-at-tp
```

That advertises the zero-length QUIC transport parameter
`0x17f7586d2cb571`, matching Yggdrasil/quiche's `RESET_STREAM_AT` support.

The server logs:

- QUIC ALPN negotiation and close reason
- client HTTP/3 SETTINGS
- raw CONNECT headers, including `:protocol`, `origin`, and `wt-available-protocols`
- selected response headers
- datagram/session flow IDs
- WebTransport stream IDs, direction, byte counts, and echo events

Upstream aioquic 1.3 exposes the HTTP/3 datagram context as `stream_id`; the
local compatibility checkout calls the same value `flow_id`. The echo handler
accepts both event shapes and uses positional `send_datagram` dispatch so the
two supported aioquic runtimes behave identically. The node health probe tests
datagram echo as well as CONNECT and stream echo.

## Current Conclusion

Chrome 149 establishes and echoes successfully against aioquic with the Bifrost local hash option shape. Chrome does not need `wt-protocol`, `capsule-protocol`, or `webtransport-init` to resolve `ready`; `/wt/basic` with only `:status=200` is enough in hash mode.

Firefox 152 also establishes against `/wt/basic` in short-certificate hash mode
and completes datagram, bidirectional stream, and unidirectional stream echo.
Against the same server with an mkcert certificate and no hash option, Firefox
validates TLS successfully and then applies its third-party-root HTTP/3 policy,
closing before H3 `SETTINGS` or WebTransport `CONNECT` reaches the server.

The in-app Chromium process still rejects mkcert for WebTransport's QUIC TLS path before H3 CONNECT, even though it loads the HTTPS page from the same mkcert CA. That makes it a poor oracle for Safari's normal trust-store path.

Safari 26.5 now establishes against the mkcert aioquic server when the server uses:

```text
--settings-profile yggdrasil
--reset-stream-at-tp
```

The decisive local Safari gate was the zero-length `RESET_STREAM_AT` QUIC transport parameter, paired with the Yggdrasil-like WebTransport H3 settings. Without that transport parameter Safari reached QUIC/TLS and H3, then reset stream 0 before app-visible CONNECT. With it, Safari sends CONNECT, resolves `transport.ready`, and opens bidi and uni WebTransport streams.

Response minimization is now reduced: with constructor variant `protocols: ["quicast-wt-v0"]`, Safari succeeds on `/wt/basic`, `/wt/protocol`, `/wt/capsule`, `/wt/init`, `/wt/yggdrasil`, and `/wt/auto`. That means `:status=200` alone is enough for `ready` and stream creation in the scratch aioquic lab; `wt-protocol` only controls whether `transport.protocol` is populated.

Constructor minimization is also reduced: against `/wt/yggdrasil`, Safari succeeds with no options, `requireUnreliable`, no-pooling plus unreliable, low-latency, and the full QUICast protocol option shape. The Bifrost option cocktail is useful for production probing, but it is not the local prerequisite for `ready` once the server transport/H3 shape is accepted.

Safari still reports the datagram JS streams as unavailable on this page, even though it advertises H3 datagram support in SETTINGS. Local Safari is therefore through `ready`, bidi, and uni; datagram API exposure remains a separate Safari/browser-surface issue.

Production Yggdrasil already advertises `RESET_STREAM_AT`, sends the WebTransport H3 settings, accepts CONNECT, and returns 200, but Safari still does not open the setup stream there. The local tokio-quiche reducer reproduced that exact post-200 stall with quiche GREASE enabled.

The scratch repo now has a tokio-quiche backend that confirms the decisive
local A/B:

```sh
cargo run --bin tokio-quiche-wt-echo -- --listen '[::]:9446'
```

With default `grease=false`, Safari 26.5 resolves `ready` and opens bidi/uni
streams against `/wt/yggdrasil` and `/wt/auto`. With `--grease`, Safari receives
and ACKs the 200 response but never opens the WebTransport stream. The likely
Yggdrasil fix to test is disabling quiche GREASE for the WebTransport endpoint,
or at least suppressing the reserved H3 frames/unknown server uni stream before
the CONNECT response HEADERS.
