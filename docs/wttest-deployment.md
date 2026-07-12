# Deploying `wttest.quicast.de`

The WebTransport browser lab is deployed directly from its GitHub checkout. It
uses aioquic 1.3.0 as the node runtime; the local tokio-quiche backend remains a
development-only differential and is not required on the node.

This repository does not modify Bifrost, Yggdrasil, Ratatoskr, or any
production service. Bifrost owns the node integration described below.

## Network Shape

```text
https://wttest.quicast.de/       TCP 443 -> Caddy -> 127.0.0.1:8088 page service
https://wttest.quicast.de:9446/  UDP 9446 -> aioquic H3/WebTransport service
```

The explicit UDP port avoids Caddy's HTTP/3 listener on UDP 443. Do not add an
`Alt-Svc` mapping from the page origin to 9446: the aioquic process is the test
target, not a replacement origin for the page.

## Node Prerequisites

- Linux with systemd
- Git access to `git@github.com:MaxF12/WebTransportEcho.git`
- Python 3.10 or newer plus the distribution's `python3-venv` package
- Node.js 22 or newer at `/usr/bin/node`
- Caddy using the Bifrost data-directory convention
- DNS `A` and, when the node is ready, `AAAA` for `wttest.quicast.de`
- TCP 80/443 and UDP 9446 permitted by both host and Linode firewalls

The installer downloads the versions in `requirements.lock` from the
configured pip index. It does not install operating-system packages or change
firewall/Caddy files.

## First Deployment

1. Point `wttest.quicast.de` at the selected node.
2. Merge `deploy/caddy/wttest.Caddyfile` into that node's Bifrost Caddyfile and
   deploy/reload Caddy so it obtains the public certificate.
3. Permit inbound UDP 9446 in the Linode Cloud Firewall and the host firewall.
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
state, derived option signals, and per-combination stage outcomes are retained.
Raw user agents, IP addresses, platform strings, target URLs, exception
messages, raw individual case records, and prior full reports are discarded.
Identical reruns update the latest timestamp but do not add a change event. The
default cap is 100 change events.

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

The installer refreshes the pinned Python environment and systemd units before
restarting the two test services. A failed fetch, pull, dependency install, or
service start exits nonzero.

## Certificate Renewal

By default, the sync service locates the newest Caddy certificate for
`wttest.quicast.de`, validates its hostname, expiry, and key match, then copies
it atomically to the service-owned TLS directory. It restarts the H3 service
and refreshes page certificate metadata only when the files change. The timer
checks every six hours.

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

It verifies the services and timer, the loopback page, the UDP listener, the
certificate hostname, a real TLS+QUIC+HTTP/3 request to `/healthz`, a
WebTransport CONNECT, a datagram echo, and a bidirectional stream echo.

Then verify externally:

```bash
curl -fsS https://wttest.quicast.de/healthz
curl -fsS https://wttest.quicast.de/matrix-config.json
curl -fsS https://wttest.quicast.de/api/browser-results
```

Open `https://wttest.quicast.de/?mode=exhaustive` in Chrome, Safari, and
Firefox. Trust-store mode produces 16 constructor combinations across eight
response paths, or 128 sequential cases. Keep `WT_AUTORUN=0` on the public
host so merely loading the page cannot generate the full matrix.

Useful logs:

```bash
sudo journalctl -u quicast-wttest-page -n 100 --no-pager
sudo journalctl -u quicast-wttest-h3 -n 200 --no-pager
sudo journalctl -u quicast-wttest-cert-sync -n 100 --no-pager
```

## Operational Boundaries

- The page service stays on loopback; only Caddy exposes it.
- Only UDP 9446 is added to the public listener allowlist.
- qlog and TLS secrets logging are disabled by default and must never be served
  by Caddy.
- Browser result state is aggregate and anonymous, but it is still operational
  state and must remain writable only by the `quicast-wttest` service account.
- The service has no media, MoQ, multicast, or production Yggdrasil dependency.
- A failure here must not trigger a fallback or change on any production media
  path.
