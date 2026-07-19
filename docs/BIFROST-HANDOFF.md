# Bifrost Handoff: `wttest.quicast.de`

Use this as the implementation prompt in the Bifrost repository.

---

Integrate `git@github.com:MaxF12/WebTransportEcho.git` on one Bifrost-managed
node as `wttest.quicast.de`.

## Scope

- Clone the repository to `/opt/quicast/webtransport-echo` and update it with
  the same fast-forward-only deployment convention as the other QUICast repos.
- Do not copy the scratch implementation into Bifrost.
- Do not modify Yggdrasil, Ratatoskr, media/MoQ behavior, or existing production
  WebTransport endpoints.
- The page is a loopback HTTP service behind Caddy. aioquic owns public UDP
  9446; isolated tokio-quiche GREASE-off/on services own UDP 9447/9448.
- Keep qlogs, private keys, TLS secrets, and test results off the public vhost.

## Repository Contract

- Git remote: `git@github.com:MaxF12/WebTransportEcho.git`
- Checkout: `/opt/quicast/webtransport-echo`
- Page upstream: `127.0.0.1:8088`
- Public page: `https://wttest.quicast.de/`
- Browser overview: `https://wttest.quicast.de/results.html`
- WebTransport target: `https://wttest.quicast.de:9446/wt/*`
- GREASE control: `https://wttest.quicast.de:9447/wt/yggdrasil`
- GREASE enabled: `https://wttest.quicast.de:9448/wt/yggdrasil`
- Node installer: `deploy/install-node.sh`
- Environment template: `deploy/wttest.env.example`
- Caddy site block: `deploy/caddy/wttest.Caddyfile`
- Node check: `/opt/quicast/webtransport-echo/deploy/check-node.sh`
- Services: `quicast-wttest-page`, `quicast-wttest-h3`,
  `quicast-wttest-quiche@control`, `quicast-wttest-quiche@grease`
- Renewal timer: `quicast-wttest-cert-sync.timer`

The selected node needs Rust/Cargo 1.85 or newer, CMake, and the C/C++/libclang
build toolchain required by the pinned tokio-quiche dependency.

## Bifrost Work

1. Add `wttest.quicast.de` to the correct node's DNS/deployment documentation.
2. Merge the supplied Caddy block into the correct node-specific Caddyfile.
3. Add UDP 9446-9448 to that node's documented Linode and host firewall
   allowlist.
4. Add all four `quicast-wttest` runtime services and UDP 9446-9448 to the
   public-exposure and health audits only when this lab is enabled.
5. Add an idempotent `deploy-wttest.sh` that clones the repository when absent,
   otherwise fetches and fast-forward pulls it, then runs
   `deploy/install-node.sh`.
6. Preserve an existing `/etc/quicast/wttest.env` on upgrades.
7. Ensure Caddy certificate renewal continues to feed the supplied sync timer.
8. Monitor the loopback `/api/browser-results` endpoint and preserve
   `/var/lib/quicast-wttest/browser-results.json` across ordinary updates.
9. Document disable and rollback commands without touching production services.

If the repository is private, use a read-only GitHub deploy key on the node.
Do not advertise UDP 9446-9448 as Caddy `Alt-Svc`; browsers reach them through
the explicit WebTransport URLs in `/matrix-config.json`.

## Acceptance Criteria

- `https://wttest.quicast.de/healthz` returns `ok` through Caddy.
- `/matrix-config.json` names `https://wttest.quicast.de:9446`, all eight
  `/wt/*` response paths, and the 9447/9448 GREASE differential targets.
- `/results.html` and `/api/browser-results` load through Caddy.
- A complete exhaustive run updates only that browser family's latest snapshot;
  an identical rerun adds no change entry, while a material metric change does.
- Stored result JSON includes normalized per-option and GREASE differential
  stages but no raw UA, IP, target URL, platform string, exception text,
  individual case record, or prior full snapshot.
- The node check passes H3 `/healthz`, WebTransport CONNECT, datagram echo, and
  bidirectional stream echo against all three H3 services.
- `ss -lun` shows UDP 9446-9448 owned by the isolated test services and TCP
  8088 remains loopback-only.
- Caddy renewal causes an atomic certificate copy and restart of all H3
  services only when the certificate changes.
- Chrome, Safari, and Firefox each complete a trust-store exhaustive run of 128
  aioquic cases plus the GREASE A/B, or produce an exported JSON report that
  identifies the precise failing stage.
- Stopping all WebTransport test services has no effect on Bifrost, Yggdrasil,
  Ratatoskr, live playback, or any production listener.

Refer to `docs/wttest-deployment.md` in the repository for exact node commands
and service paths.

---
