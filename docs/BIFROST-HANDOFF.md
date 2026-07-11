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
- The page is a loopback HTTP service behind Caddy. The aioquic backend owns
  public UDP 9446 directly.
- Keep qlogs, private keys, TLS secrets, and test results off the public vhost.

## Repository Contract

- Git remote: `git@github.com:MaxF12/WebTransportEcho.git`
- Checkout: `/opt/quicast/webtransport-echo`
- Page upstream: `127.0.0.1:8088`
- Public page: `https://wttest.quicast.de/`
- WebTransport target: `https://wttest.quicast.de:9446/wt/*`
- Node installer: `deploy/install-node.sh`
- Environment template: `deploy/wttest.env.example`
- Caddy site block: `deploy/caddy/wttest.Caddyfile`
- Node check: `/opt/quicast/webtransport-echo/deploy/check-node.sh`
- Services: `quicast-wttest-page`, `quicast-wttest-h3`
- Renewal timer: `quicast-wttest-cert-sync.timer`

## Bifrost Work

1. Add `wttest.quicast.de` to the correct node's DNS/deployment documentation.
2. Merge the supplied Caddy block into the correct node-specific Caddyfile.
3. Add UDP 9446 to that node's documented Linode and host firewall allowlist.
4. Add UDP 9446 and both `quicast-wttest` services to the public-exposure and
   health audits only when this lab is enabled.
5. Add an idempotent `deploy-wttest.sh` that clones the repository when absent,
   otherwise fetches and fast-forward pulls it, then runs
   `deploy/install-node.sh`.
6. Preserve an existing `/etc/quicast/wttest.env` on upgrades.
7. Ensure Caddy certificate renewal continues to feed the supplied sync timer.
8. Document disable and rollback commands without touching production services.

If the repository is private, use a read-only GitHub deploy key on the node.
Do not advertise UDP 9446 as Caddy `Alt-Svc`; browsers reach it through the
explicit WebTransport URL in `/matrix-config.json`.

## Acceptance Criteria

- `https://wttest.quicast.de/healthz` returns `ok` through Caddy.
- `/matrix-config.json` names `https://wttest.quicast.de:9446` and all eight
  `/wt/*` response paths.
- The node check passes, including its verified H3 `/healthz` request,
  WebTransport CONNECT, and bidirectional stream echo.
- `ss -lun` shows UDP 9446 owned by the aioquic service and TCP 8088 remains
  loopback-only.
- Caddy renewal causes an atomic certificate copy and H3 restart only when the
  certificate changes.
- Chrome, Safari, and Firefox each complete a trust-store exhaustive run of 128
  cases, or produce an exported JSON report that identifies the precise failing
  stage.
- Stopping both test services has no effect on Bifrost, Yggdrasil, Ratatoskr,
  live playback, or any production listener.

Refer to `docs/wttest-deployment.md` in the repository for exact node commands
and service paths.

---
