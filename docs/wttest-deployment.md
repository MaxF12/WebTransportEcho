# Deploying `wttest.quicast.de`

The WebTransport browser lab is deployed directly from its GitHub checkout. It
uses aioquic 1.3.0 for the exhaustive baseline and two instances of the pinned
QUICast tokio-quiche backend for a GREASE-off/on differential.

This repository does not modify Bifrost, Yggdrasil, Ratatoskr, or any
production service. Bifrost owns the node integration described below.

## Network Shape

```text
https://wttest.quicast.de/       TCP 443 -> Caddy -> 127.0.0.1:8088 page service
https://wttest.quicast.de:9446/  UDP 9446 -> aioquic H3/WebTransport service
https://wttest.quicast.de:9447/  UDP 9447 -> tokio-quiche, GREASE disabled
https://wttest.quicast.de:9448/  UDP 9448 -> tokio-quiche, GREASE enabled
```

The explicit UDP ports avoid Caddy's HTTP/3 listener on UDP 443. Do not add an
`Alt-Svc` mapping from the page origin to these ports: they are test targets,
not replacement origins for the page.

## Node Prerequisites

- Linux with systemd
- Git access to `git@github.com:MaxF12/WebTransportEcho.git`
- Python 3.10 or newer plus the distribution's `python3-venv` package
- Node.js 22 or newer at `/usr/bin/node`
- Rust/Cargo 1.85 or newer, CMake, and a C/C++/libclang build toolchain for the
  pinned tokio-quiche build
- Caddy using the Bifrost data-directory convention
- DNS `A` and, when the node is ready, `AAAA` for `wttest.quicast.de`
- TCP 80/443 and UDP 9446-9448 permitted by both host and Linode firewalls

The installer downloads the versions in `requirements.lock` from the
configured pip index and builds the exact public QUICast quiche revision pinned
in `Cargo.lock`. It does not install operating-system packages or change
firewall/Caddy files.

## First Deployment

1. Point `wttest.quicast.de` at the selected node.
2. Merge `deploy/caddy/wttest.Caddyfile` into that node's Bifrost Caddyfile and
   deploy/reload Caddy so it obtains the public certificate.
3. Permit inbound UDP 9446-9448 in the Linode Cloud Firewall and host firewall.
4. Clone this repository at its fixed service path and run the installer.

```bash
sudo git clone git@github.com:MaxF12/WebTransportEcho.git \
  /opt/quicast/webtransport-echo
sudo /opt/quicast/webtransport-echo/deploy/install-node.sh
```

If the repository is private, give the node a read-only GitHub deploy key. Do
not place a personal writable SSH key on the node.

The installer creates:

```text
/opt/quicast/webtransport-echo/.venv    pinned Python environment
/etc/quicast/wttest.env                 operator-owned configuration
/etc/quicast/wttest/                    copied certificate and private key
/var/lib/quicast-wttest/                anonymous result state and optional debug state
```

It also creates the unprivileged `quicast-wttest` account and installs these
units:

```text
quicast-wttest-page.service
quicast-wttest-h3.service
quicast-wttest-quiche@control.service
quicast-wttest-quiche@grease.service
quicast-wttest-cert-sync.service
quicast-wttest-cert-sync.timer
```

Existing `/etc/quicast/wttest.env` configuration is retained on later runs.

## Anonymous Result State

`https://wttest.quicast.de/results.html` shows the latest complete exhaustive
result for each browser family and a bounded material-change log. The page
server writes this state atomically to:

```text
/var/lib/quicast-wttest/browser-results.json
```

Only normalized aggregate counts, browser family and major version, per-path
state, derived option signals, per-combination stage outcomes, and normalized
GREASE control/test stages are retained. The GREASE verdict is derived again
by the server. Raw user agents, IP addresses, platform strings, target URLs,
exception messages, raw individual case records, and prior full reports are
discarded. Identical reruns update the latest timestamp but do not add a change
event. The default cap is 100 change events.

Only same-origin POST requests are accepted. Complete exhaustive runs against
the configured target and all eight paths are eligible; selected, cancelled,
incomplete, partial-path, and custom-target runs remain in the browser.

## Updating

Bifrost should own an idempotent deploy wrapper that uses its existing
fast-forward-only Git helper, then reruns the installer:

```bash
sudo git -C /opt/quicast/webtransport-echo fetch --prune
sudo git -C /opt/quicast/webtransport-echo pull --ff-only
sudo /opt/quicast/webtransport-echo/deploy/install-node.sh
```

The installer refreshes the pinned Python environment, pinned release-mode Rust
binary, and systemd units before restarting the test services. A failed fetch,
pull, dependency install, build, or service start exits nonzero.

## Certificate Renewal

By default, the sync service locates the newest Caddy certificate for
`wttest.quicast.de`, validates its hostname, expiry, and key match, then copies
it atomically to the service-owned TLS directory. It restarts all three H3
services and refreshes page certificate metadata only when the files change.
The timer checks every six hours.

For a non-Caddy certificate source, set both values in
`/etc/quicast/wttest.env`:

```env
WT_CERT_SOURCE_CERT=/path/to/fullchain.pem
WT_CERT_SOURCE_KEY=/path/to/privkey.pem
```

Never point the unprivileged services directly at Caddy's private data tree.

## Verification

Run the complete local node check:

```bash
sudo /opt/quicast/webtransport-echo/deploy/check-node.sh
```

It verifies the services and timer, the loopback page, all three UDP listeners,
the certificate hostname, and a real TLS+QUIC+HTTP/3 `/healthz`, WebTransport
CONNECT, datagram echo, and bidirectional stream echo against every backend.

Then verify externally:

```bash
curl -fsS https://wttest.quicast.de/healthz
curl -fsS https://wttest.quicast.de/matrix-config.json
curl -fsS https://wttest.quicast.de/api/browser-results
```

Open `https://wttest.quicast.de/?mode=exhaustive` in Chrome, Safari, and
Firefox. Trust-store mode produces 16 constructor combinations across eight
response paths, or 128 sequential aioquic cases, followed by one non-pooled
tokio-quiche control/GREASE pair. Keep `WT_AUTORUN=0` on the public host so
merely loading the page cannot generate the full matrix.

Useful logs:

```bash
sudo journalctl -u quicast-wttest-page -n 100 --no-pager
sudo journalctl -u quicast-wttest-h3 -n 200 --no-pager
sudo journalctl -u quicast-wttest-quiche@control -n 200 --no-pager
sudo journalctl -u quicast-wttest-quiche@grease -n 200 --no-pager
sudo journalctl -u quicast-wttest-cert-sync -n 100 --no-pager
```

The page wrapper derives the two public GREASE targets from `WT_CERT_HOST` and
the quiche ports, so an existing environment file gains the differential after
the installer enables the two services. Set `WT_GREASE_DIFFERENTIAL=0` and
rerun the installer to disable both instances. Explicit target overrides remain
available in `deploy/wttest.env.example`.

## Operational Boundaries

- The page service stays on loopback; only Caddy exposes it.
- Only UDP 9446-9448 are added to the public listener allowlist.
- qlog and TLS secrets logging are disabled by default and must never be served
  by Caddy.
- Browser result state is aggregate and anonymous, but it is still operational
  state and must remain writable only by the `quicast-wttest` service account.
- The service has no media, MoQ, multicast, or production Yggdrasil dependency.
- A failure here must not trigger a fallback or change on any production media
  path.
