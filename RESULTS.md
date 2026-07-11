# Results

Date: 2026-06-18

## Read-Only Comparison Against QUICast

Bifrost's local WebTransport option shape is:

```js
{
  allowPooling: false,
  requireUnreliable: true,
  congestionControl: "low-latency",
  protocols: ["quicast-wt-v0"]
}
```

For localhost diagnostics, Bifrost can also add `serverCertificateHashes` using raw SHA-256 DER certificate bytes.

Yggdrasil's negotiated response shape for `wt-available-protocols: "quicast-wt-v0"` is:

```text
:status=200
server=yggdrasil
wt-protocol="quicast-wt-v0"
capsule-protocol=?1
webtransport-init=u=8388608, bl=8388608, br=8388608
access-control-allow-origin=<origin>
access-control-allow-credentials=true
vary=origin
```

Yggdrasil's server-side SETTINGS shape includes the aioquic defaults plus:

```text
h3_datagram_draft04_0xffd277=true
wt_enabled_0x2c7cf000=true
webtransport_max_sessions_draft07_0xc671706a=1
webtransport_max_sessions_0x14e9cd29=1
wt_initial_max_data_0x2b61=8388608
wt_initial_max_streams_uni_0x2b64=100
wt_initial_max_streams_bidi_0x2b65=100
```

Those production repos were only read, not modified.

## Implementation Status

| Item | Result | Evidence |
| --- | --- | --- |
| aioquic server | Pass | `scripts/aioquic-wt-echo.py` accepts CONNECT and echoes datagrams, bidi streams, and uni streams |
| tokio-quiche backend | Pass | `src/bin/tokio_quiche_wt_echo.rs` uses local `../../Multicast/quiche/tokio-quiche`, enables `RESET_STREAM_AT`, disables quiche GREASE by default, mirrors `/wt/*` responses, and logs WT diagnostics |
| Yggdrasil SETTINGS profile | Pass | `--settings-profile yggdrasil` adds the extra H3 settings Yggdrasil advertises |
| short-lived WT certificate | Pass | `scripts/make-short-lived-wt-cert.sh` creates P-256 ECDSA, SAN-only, 7-day cert |
| page query controls | Pass | `public/index.html` supports `paths`, `variants`, `timeout`, `target`, and `autorun` |
| aioquic syntax | Pass | `python -m py_compile scripts/aioquic-wt-echo.py` |
| page server syntax | Pass | `node --check scripts/serve-page.mjs` |
| tokio-quiche backend syntax/build | Pass | `cargo check --bin tokio-quiche-wt-echo`; runtime bind smoke on UDP `:9446` passed |
| Rust/quinn server | Not a valid oracle | Chrome reaches CONNECT with hash cert but rejects `ready` for every response variant |

## Chrome Baseline, aioquic plus short-lived certificate hash

Browser: in-app Chromium 149

Constructor option variant: certificate hash plus QUICast production shape:

```js
{
  allowPooling: false,
  requireUnreliable: true,
  congestionControl: "low-latency",
  protocols: ["quicast-wt-v0"],
  serverCertificateHashes: [{ algorithm: "sha-256", value: <raw DER sha256 bytes> }]
}
```

| Path | Response headers | UDP hits server | CONNECT reaches app | `ready` | `transport.protocol` | Datagram echo | Bidi echo | Uni echo |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `/wt/basic` | `:status=200` | yes | yes | fulfilled | empty string | fulfilled | fulfilled | fulfilled |
| `/wt/protocol` | `:status=200`, `wt-protocol` | yes | yes | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled |
| `/wt/capsule` | `:status=200`, `wt-protocol`, `capsule-protocol` | yes | yes | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled |
| `/wt/init` | `:status=200`, `wt-protocol`, `capsule-protocol`, `webtransport-init` | yes | yes | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled |
| `/wt/draft` | `:status=200`, `sec-webtransport-http3-draft` | yes | yes | fulfilled | empty string | fulfilled | fulfilled | fulfilled |
| `/wt/h3-token` | `:status=200` | yes | yes | fulfilled | empty string | fulfilled | fulfilled | fulfilled |

Observed Chrome request headers:

```text
:scheme=https
:method=CONNECT
:authority=localhost:9443
:path=/wt/basic
:protocol=webtransport
sec-webtransport-http3-draft02=1
origin=https://localhost:8443
wt-available-protocols="quicast-wt-v0"
```

Observed client H3 settings included:

```text
h3_datagram=1
enable_webtransport=1
qpack_max_table_capacity=65536
max_field_section_size=16384
qpack_blocked_streams=100
```

Chrome also sent additional unknown/greased setting IDs.

## Chrome mkcert no-hash check

Browser: in-app Chromium 149

Server certificate: `certs/localhost.pem` from `mkcert`

| Path | Constructor options | UDP hits server | CONNECT reaches app | Request headers/settings | `ready` | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `/wt/basic` | no options | yes | no | none; QUIC TLS failed first | rejected: `Opening handshake failed` | server logged `CERTIFICATE_VERIFY_FAILED` |
| `/wt/basic` | `{ requireUnreliable: true }` | yes | no | none; QUIC TLS failed first | rejected: `Opening handshake failed` | same certificate failure |
| `/wt/basic` | no pooling plus unreliable | yes | no | none; QUIC TLS failed first | rejected: `Opening handshake failed` | same certificate failure |
| `/wt/basic` | low latency | yes | no | none; QUIC TLS failed first | rejected: `Opening handshake failed` | same certificate failure |
| `/wt/basic` | QUICast protocol options | yes | no | none; QUIC TLS failed first | rejected: `Opening handshake failed` | same certificate failure |

Server-side close reason:

```text
TLS handshake failure (ENCRYPTION_HANDSHAKE) 46: certificate unknown
OPENSSL_internal:CERTIFICATE_VERIFY_FAILED
```

This Chromium result is not a Safari conclusion. It only says this in-app Chromium runtime is not accepting the mkcert CA for WebTransport's QUIC TLS path, despite loading the HTTPS page.

## Mode Collision Found From External Chrome Screenshot

The failing screenshot selected the two `serverCertificateHashes` constructor variants while the running server/page pair was in mkcert mode:

```text
page: https://localhost:8443/
WT target: https://localhost:9443/
WT cert: certs/localhost.pem
```

That cannot be the Chrome hash baseline. The page metadata for the mkcert server reports:

```json
{
  "publicKeyType": "rsa",
  "isSelfSigned": false,
  "validityDays": 823,
  "hashUsable": false
}
```

Chrome's hash path needs the short-lived self-signed P-256 certificate. A separate Chrome baseline is now available:

```text
page: https://localhost:8444/
WT target: https://localhost:9444/
WT cert: certs/wt-short.pem
```

The page metadata for that baseline reports:

```json
{
  "publicKeyType": "ec",
  "isSelfSigned": true,
  "validityDays": 7,
  "hashUsable": true
}
```

Verified in in-app Chromium 149 on `https://localhost:8444/?target=https://localhost:9444&paths=/wt/basic&variants=5,6&timeout=4000&autorun=1`:

| Path | Constructor options | `ready` | Datagram echo | Bidi echo | Uni echo | Closed |
| --- | --- | --- | --- | --- | --- | --- |
| `/wt/basic` | cert hash plus unreliable | fulfilled | fulfilled | fulfilled | fulfilled | fulfilled |
| `/wt/basic` | cert hash plus QUICast protocol options | fulfilled | fulfilled | fulfilled | fulfilled | fulfilled |

The server logged H3 CONNECT, `h3_datagram=1`, `enable_webtransport=1`, datagram echo, bidi stream echo, and uni stream echo for both cases.

The page UI now disables certificate-hash variants automatically when the advertised WT cert is not hash-mode compatible.

## Chrome Baseline, Yggdrasil SETTINGS and response shape

Browser: in-app Chromium 149

Server certificate: short-lived self-signed P-256 certificate with `serverCertificateHashes`

Server profile:

```text
--settings-profile yggdrasil
--reset-stream-at-tp
```

Verified on `https://localhost:8444/?target=https://localhost:9444&paths=/wt/yggdrasil,/wt/auto&variants=6&timeout=4000&autorun=1`:

| Path | Constructor options | Response headers | `ready` | `transport.protocol` | Datagram echo | Bidi echo | Uni echo | Closed |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `/wt/yggdrasil` | cert hash plus QUICast protocol options | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled | fulfilled |
| `/wt/auto` | cert hash plus QUICast protocol options | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled | fulfilled |

Server evidence:

```text
server H3 settings profile=yggdrasil settings={..., h3_datagram_draft04=1, wt_enabled=1, webtransport_max_sessions=1, wt_initial_max_data=8388608, wt_initial_max_streams_uni=100, wt_initial_max_streams_bidi=100}
H3 headers stream=0 ... :path=/wt/yggdrasil ... wt-available-protocols="quicast-wt-v0"
WebTransport CONNECT accepted ... response=:status=200,server=yggdrasil,wt-protocol="quicast-wt-v0",capsule-protocol=?1,webtransport-init=...
datagram received/echoed
WT stream data stream=4 direction=bidi
WT stream data stream=14 direction=uni
```

This proves the Yggdrasil-shaped SETTINGS/response/transport-parameter path is still a valid local Chrome oracle when certificate validation is controlled with the short-lived hash certificate.

## Safari comparison from Yggdrasil trace

Safari against production Yggdrasil gets farther than the original local aioquic trust-store run:

| Stack | UDP reaches server | H3 SETTINGS received | CONNECT reaches app | 200 response sent | `ready` / streams |
| --- | --- | --- | --- | --- | --- |
| Yggdrasil production | yes | yes | yes | yes | Safari does not open setup stream; Yggdrasil logs `subscribed=false` |
| local aioquic, native settings profile | yes | no app-visible CONNECT | no | no | Safari resets stream 0 before response variants matter |
| local aioquic, Yggdrasil settings profile | yes | yes | no | no | Safari resets stream 0 before response variants matter |
| local aioquic, Yggdrasil settings plus RESET_STREAM_AT TP | yes | yes | yes | yes | `ready`, bidi echo, and uni echo fulfilled; Safari datagram JS surface unavailable |

The production Yggdrasil log shows:

```text
client HTTP/3 settings received ... h3_datagram=1, webtransport_max_sessions=1, wt_initial_max_data=8388608
client H3 CONNECT request stream=0 ... :protocol=webtransport ... wt_available_protocols="quicast-wt-v0"
WebTransport CONNECT response ... :status=200,server=yggdrasil,wt-protocol="quicast-wt-v0",capsule-protocol=?1,webtransport-init=...,access-control-allow-origin=...
```

Then Yggdrasil remains:

```text
subscribed=false pending_streams=0 next_uni_stream_id=15
```

So there are two separate Safari boundaries:

- Local pre-CONNECT boundary: Safari reaches aioquic CONNECT when the server advertises Yggdrasil-like SETTINGS and the RESET_STREAM_AT QUIC transport parameter.
- Yggdrasil post-200 boundary: after a CONNECT 200, why does Safari not open the WebTransport stream/setup path?

## Safari success, local aioquic with Yggdrasil shape

Browser: Safari 26.5, `Mozilla/5.0 ... Version/26.5 Safari/605.1.15`

Server profile:

```text
--settings-profile yggdrasil
--reset-stream-at-tp
```

Constructor option variant:

```js
{
  allowPooling: false,
  requireUnreliable: true,
  congestionControl: "low-latency",
  protocols: ["quicast-wt-v0"]
}
```

Verified by Safari screenshots and server logs:

| Cert | Path | CONNECT reaches app | Response | `ready` | `transport.protocol` | Datagram | Bidi echo | Uni echo | Closed |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mkcert ECDSA | `/wt/yggdrasil` | yes | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| mkcert ECDSA | `/wt/auto` | yes | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| mkcert RSA | `/wt/yggdrasil` | yes | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| mkcert RSA | `/wt/auto` | yes | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |

Response minimization, mkcert ECDSA target `https://localhost:9445`, constructor variant with `protocols: ["quicast-wt-v0"]`:

| Path | Response | `ready` | `transport.protocol` | Datagram | Bidi echo | Uni echo | Closed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `/wt/basic` | `:status=200` only | fulfilled | empty string | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `/wt/protocol` | `:status=200`, `wt-protocol` | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `/wt/capsule` | `:status=200`, `wt-protocol`, `capsule-protocol` | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `/wt/init` | protocol, capsule, `webtransport-init` | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `/wt/yggdrasil` | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `/wt/auto` | Yggdrasil protocol/capsule/init/CORS | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |

Constructor minimization, mkcert ECDSA target `https://localhost:9445`, path `/wt/yggdrasil`:

| Constructor options | `ready` | `transport.protocol` | Datagram | Bidi echo | Uni echo | Closed |
| --- | --- | --- | --- | --- | --- | --- |
| no options | fulfilled | empty string | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| `{ requireUnreliable: true }` | fulfilled | empty string | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| no pooling plus unreliable | fulfilled | empty string | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| low latency | fulfilled | empty string | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |
| full QUICast protocol options | fulfilled | `quicast-wt-v0` | unavailable in Safari JS | fulfilled | fulfilled | fulfilled |

Server evidence for Safari:

```text
client H3 settings={..., h3_datagram=1, webtransport_max_sessions=1, wt_initial_max_data=8388608, wt_initial_max_streams_uni=100, wt_initial_max_streams_bidi=100}
H3 headers stream=0 ... :method=CONNECT ... :protocol=webtransport ... wt-available-protocols="quicast-wt-v0"
WebTransport CONNECT accepted ... response=:status=200,server=yggdrasil,wt-protocol="quicast-wt-v0",capsule-protocol=?1,webtransport-init=...
WT stream data stream=4 direction=bidi ... echoed
WT stream data stream=10 direction=uni ... echoed
```

The key pre-CONNECT difference was `RESET_STREAM_AT`: with Yggdrasil H3 SETTINGS but without the QUIC transport parameter, Safari reset stream 0 with H3 request-cancelled before sending CONNECT headers. With that transport parameter, Safari sent CONNECT and opened WebTransport streams.

## Current Conclusion

The aioquic server fixes the scratch lab: Chrome has a known-good WebTransport echo baseline, and `/wt/basic` proves Chrome does not require Yggdrasil's extra response headers to resolve `ready`.

Safari in trust-store mode with the original aioquic settings profile, and again with only the Yggdrasil H3 SETTINGS profile, reached QUIC and negotiated `h3`, but reset stream 0 before aioquic emitted any H3 `HeadersReceived` event. In other words:

```text
QUIC/TLS: yes
ALPN h3: yes
H3 CONNECT headers at app: no
response variant selected: no
ready: rejected
```

Adding the zero-length `RESET_STREAM_AT` QUIC transport parameter changed that boundary: Safari now reaches CONNECT and completes `ready` plus bidi and uni stream echo.

The current local Safari handshake shape is:

```text
server QUIC TP: RESET_STREAM_AT present
server H3 SETTINGS: Yggdrasil-like WebTransport settings present
response headers: :status=200 is enough for ready and streams
constructor options: no options are enough on /wt/yggdrasil
protocol negotiation: protocols:["quicast-wt-v0"] plus wt-protocol populates transport.protocol
datagrams: Safari JS datagram streams unavailable in this page
```

That means the local Safari prerequisite is server transport/H3 shape, not the Bifrost constructor option cocktail or the Yggdrasil response-header bundle.

The next isolated runs are now set up:

| Mode | Page | WT target | Cert | Purpose |
| --- | --- | --- | --- | --- |
| mkcert RSA | `https://localhost:8443/` | `https://localhost:9443` | `certs/localhost.pem` | current Safari trust-store baseline with qlog |
| mkcert ECDSA | `https://localhost:8445/` | `https://localhost:9445` | `certs/localhost-ecdsa.pem` | isolate RSA-vs-ECDSA certificate/key behavior |
| short self-signed ECDSA hash | `https://localhost:8444/` | `https://localhost:9444` | `certs/wt-short.pem` | Chrome-only hash baseline |
 
Both mkcert certificates verify for `localhost` with macOS `security verify-cert` reporting `Cert Verify Result: No error`; the remaining CT warning is not a localhost trust-chain failure.

The remaining production question is now Yggdrasil-specific and post-200: production Yggdrasil already logs `transport_reset_stream_at=true`, receives Safari CONNECT, and sends the negotiated 200 response, but Safari does not open the setup stream there. The next comparison target is how Yggdrasil/tokio-quiche/quiche transitions the CONNECT stream into an established WebTransport session after sending 200.

One optional final local cross-check remains: run `/wt/basic` with constructor variants 0 through 4. Based on the completed reductions it should establish, but the exact cross product has not yet been captured in the matrix.

## Secondary backend, local tokio-quiche

Added a second scratch backend to compare against aioquic and production
Yggdrasil without touching production repos:

```sh
cargo run --bin tokio-quiche-wt-echo -- \
  --listen '[::]:9446' \
  --cert certs/localhost.pem \
  --key certs/localhost-key.pem \
  --qlog-dir qlogs/tokio-quiche-rsa \
  --secrets-log qlogs/tokio-quiche-rsa/secrets.log
```

Backend properties:

```text
stack: local ../../Multicast/quiche/tokio-quiche
QUIC TP: RESET_STREAM_AT enabled by default
GREASE: disabled by default in this scratch binary; pass --grease to reproduce quiche's default reserved-stream/reserved-frame behavior
H3 SETTINGS: extended CONNECT plus WebTransport enabled
responses: same /wt/basic, /wt/protocol, /wt/capsule, /wt/init, /wt/draft, /wt/yggdrasil, /wt/auto matrix
diagnostics: ConnectSessionRegistered, H3HeadersFlushedToQuic, WT stream-prefix classification
bidi echo: raw quiche stream_send() on the same WT bidi stream
uni echo: raw quiche stream_send() on server uni stream IDs starting at 15
datagram echo: H3 DATAGRAM flow echo when tokio-quiche exposes a flow
```

Matrix URL:

```text
https://localhost:8443/?target=https://localhost:9446&paths=/wt/basic,/wt/protocol,/wt/capsule,/wt/init,/wt/yggdrasil,/wt/auto&variants=0,1,2,3,4&timeout=5000&autorun=1
```

First external browser screenshots against the mkcert tokio-quiche target:

| Browser | Target | Constructor/path set | Result | Interpretation |
| --- | --- | --- | --- | --- |
| Chrome | `https://localhost:9446` with `certs/localhost.pem` | trust-store variants 0 through 4 | `Opening handshake failed` | Expected mkcert/WebTransport QUIC TLS failure for this Chrome runtime; not a tokio-quiche result |
| Safari | `https://localhost:9446` with `certs/localhost.pem` | trust-store variants 0 through 4 | `ready` timed out | Important: this looks like a post-handshake/post-response stall, matching the Yggdrasil symptom; confirm with tokio-quiche terminal diagnostics |
 
The qlog for the GREASE-enabled tokio-quiche run is the first local reproduction
of the Yggdrasil symptom:

```text
Safari sends CONNECT :protocol=webtransport, :path=/wt/yggdrasil,
origin=https://localhost:8443, wt-available-protocols="quicast-wt-v0"
tokio-quiche sends response HEADERS :status=200, server=yggdrasil,
wt-protocol="quicast-wt-v0", capsule-protocol=?1, webtransport-init=...
Safari ACKs the response packet.
After that, only PING/ACK traffic continues; Safari opens no WT stream.
```

The same qlog shows quiche GREASE before the response HEADERS:

```text
server uni stream id 15, unknown stream type, payload "GREASE is the word"
reserved HTTP/3 frame, length 0
reserved HTTP/3 frame, payload "GREASE is the word"
then response HEADERS on the CONNECT stream
```

The successful aioquic oracle has GREASE disabled. The tokio-quiche scratch
binary now disables quiche GREASE by default and accepts `--grease` to reproduce
the current quiche/Yggdrasil-shaped stall.

Confirmed Safari A/B on the same tokio-quiche backend:

| Browser | Target | quiche GREASE | Paths | Constructor options | `ready` | Protocol | Bidi | Uni | Closed |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Safari 26.5 | `https://localhost:9446` | enabled | `/wt/yggdrasil`, `/wt/auto` | QUICast protocol option shape | timeout | none | skipped | skipped | timeout |
| Safari 26.5 | `https://localhost:9448` | disabled | `/wt/yggdrasil`, `/wt/auto` | QUICast protocol option shape | fulfilled | `quicast-wt-v0` | fulfilled | fulfilled | fulfilled |

The successful no-GREASE qlogs in `qlogs/tokio-quiche-rsa-nogrease/` show
CONNECT request HEADERS followed directly by response HEADERS, with no reserved
HTTP/3 frames and no unknown local uni stream before the 200.

Conclusion: Safari's WebTransport handshake accepts the tokio-quiche/Yggdrasil
settings and response shape when quiche GREASE is disabled. The local
Yggdrasil-like failure is caused by quiche's GREASE behavior, most likely the
reserved HTTP/3 frames before CONNECT response HEADERS and/or the unknown server
uni stream opened before the WebTransport session settles.

Chrome control for tokio-quiche needs the short-lived hash certificate:

```sh
cargo run --bin tokio-quiche-wt-echo -- \
  --listen '[::]:9447' \
  --cert certs/wt-short.pem \
  --key certs/wt-short-key.pem \
  --qlog-dir qlogs/tokio-quiche-short \
  --secrets-log qlogs/tokio-quiche-short/secrets.log

PAGE_PORT=8444 WT_TARGET_CERT=certs/wt-short.pem node scripts/serve-page.mjs
```

Chrome URL:

```text
https://localhost:8444/?target=https://localhost:9447&paths=/wt/yggdrasil,/wt/auto&variants=6&timeout=5000&autorun=1
```

This backend is now the local proof that Yggdrasil should be tested with quiche
GREASE disabled. If production Yggdrasil still stalls after that change, the
next suspect would move back to Yggdrasil application/session wiring; but the
scratch A/B says GREASE is the first fix to try.

## Exhaustive browser matrix harness

The page now has a generated exhaustive mode in addition to the seven original
curated presets. Trust-store mode tests the 16 combinations of
`allowPooling=false`, `requireUnreliable=true`, low-latency congestion control,
and `protocols=["quicast-wt-v0"]`. Compatible short-certificate mode adds
`serverCertificateHashes` as a fifth axis for 32 combinations.

Verified in in-app Chrome 150 against the known-good aioquic backend on one
`/wt/basic` path, using a freshly generated short-lived certificate, Yggdrasil
settings, `RESET_STREAM_AT`, and `--webtransport-max-sessions 1024`:

| Cases | Constructor | Ready | Datagram | Bidi | Uni | Full echo | Automatic signal |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 32/32 | 32/32 | 16/32 | 16/32 | 16/32 | 16/32 | 16/32 | `ready only with serverCertificateHashes=sha-256` |

All 16 hash-authenticated combinations passed every echo stage. All 16 cases
without the hash failed normal trust validation, as expected for the self-signed
certificate. Running hash cases before intentional certificate failures plus a
250 ms test gap removed Chromium's false negative burst after repeated
`CERTIFICATE_VERIFY_FAILED` handshakes; the page defaults to a 150 ms gap.

The harness also now:

- reports browser identity, secure-context state, and API presence before network work;
- distinguishes `WebTransport` unavailable from API-exposed-but-no-ready;
- records reliability, exact options, outcomes, summaries, and per-path option effects;
- supports cancellation without inferring option effects from an incomplete run;
- rejects expired short certificates as hash-mode incompatible;
- accepts `mode=exhaustive` or `exhaustive=1` for autorun.

Brave has not yet been run against this version. If its build does not expose a
constructible `WebTransport`, the page will produce the capability verdict and
zero network cases rather than a misleading handshake matrix.

Future `wttest.quicast.de` hosting is prepared through runtime
`/matrix-config.json`, optional `PAGE_TLS=0` operation behind an HTTPS edge, and
the isolated TCP-page / UDP-H3 deployment blueprint in
`docs/wttest-deployment.md`. No Bifrost or production files were modified.

## Firefox 152 local certificate result

The initial Firefox exhaustive run against the mkcert aioquic endpoint reported
128 constructors and zero successful `ready` promises. Server and Firefox logs
reduce all 128 cases to one pre-CONNECT policy decision:

```text
Firefox: Http3Session::Authenticated error=0x0
Firefox: hasThirdPartyRoots=1, servCertHashesSucceeded=0
Firefox: CloseTransaction reason=NS_ERROR_NET_RESET (0x804b0014)
server: QUIC negotiated h3, then peer close 0x0c; no client H3 SETTINGS or CONNECT
```

Firefox's default
`network.http.http3.disable_when_third_party_roots_found=true` disables H3 when
the valid chain ends at the local mkcert CA. API exposure therefore did not
imply that this certificate mode could establish HTTP/3.

The short-lived self-signed P-256 control with `serverCertificateHashes` passed:

| Browser | Certificate mode | TLS | CONNECT | Ready | Datagram | Bidi | Uni |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Firefox 152 | mkcert trust store, no hash | validated | no | rejected | skipped | skipped | skipped |
| Firefox 152 | 7-day self-signed P-256 plus SHA-256 hash | validated by hash | yes | fulfilled | fulfilled | fulfilled | fulfilled |

The successful request used `:protocol=webtransport`, `/wt/basic`, and Firefox's
legacy `sec-webtransport-http3-draft02: 1` request header. The server returned
only `:status=200`; no protocol, capsule, init, Yggdrasil settings, or
`RESET_STREAM_AT` behavior was required for this Firefox baseline.
