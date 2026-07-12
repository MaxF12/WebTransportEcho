#!/usr/bin/env python3
import argparse
import asyncio
import logging
from pathlib import Path
from typing import IO
from typing import Callable
from typing import Dict, List, Optional, Tuple

from aioquic.buffer import Buffer
from aioquic.asyncio import QuicConnectionProtocol, serve
from aioquic.h3.connection import H3_ALPN, H3Connection, Setting
from aioquic.h3.events import (
    DatagramReceived,
    H3Event,
    Headers,
    HeadersReceived,
    WebTransportStreamDataReceived,
)
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.connection import QuicConnection
from aioquic.quic.connection import stream_is_unidirectional
from aioquic.quic.events import (
    ConnectionTerminated,
    HandshakeCompleted,
    ProtocolNegotiated,
    QuicEvent,
    StreamDataReceived,
    StreamReset,
)
from aioquic.quic.logger import QuicFileLogger


LOG = logging.getLogger("wt-echo-aioquic")
QUICAST_PROTOCOL = b"quicast-wt-v0"
WEBTRANSPORT_INIT = b"u=8388608, bl=8388608, br=8388608"
SETTINGS_PROFILE_AIOQUIC = "aioquic"
SETTINGS_PROFILE_YGGDRASIL = "yggdrasil"

H3_DATAGRAM_DRAFT04 = 0xFFD277
WT_ENABLED = 0x2C7CF000
WT_MAX_SESSIONS_DRAFT07 = 0xC671706A
WT_MAX_SESSIONS = 0x14E9CD29
WT_INITIAL_MAX_DATA = 0x2B61
WT_INITIAL_MAX_STREAMS_UNI = 0x2B64
WT_INITIAL_MAX_STREAMS_BIDI = 0x2B65
WT_INITIAL_MAX_DATA_VALUE = 8 * 1024 * 1024
WT_INITIAL_MAX_STREAMS_VALUE = 100
RESET_STREAM_AT_TRANSPORT_PARAMETER_ID = 0x17F7586D2CB571


def install_reset_stream_at_transport_parameter() -> None:
    if getattr(QuicConnection, "_wt_echo_reset_stream_at_tp", False):
        return

    original: Callable[[QuicConnection], bytes] = QuicConnection._serialize_transport_parameters

    def serialize_with_reset_stream_at(self: QuicConnection) -> bytes:
        data = original(self)
        buf = Buffer(capacity=len(data) + 16)
        buf.push_bytes(data)
        buf.push_uint_var(RESET_STREAM_AT_TRANSPORT_PARAMETER_ID)
        buf.push_uint_var(0)
        return buf.data

    QuicConnection._serialize_transport_parameters = serialize_with_reset_stream_at
    setattr(QuicConnection, "_wt_echo_reset_stream_at_tp", True)


class EchoH3Connection(H3Connection):
    def __init__(
        self,
        *args,
        settings_profile: str = SETTINGS_PROFILE_AIOQUIC,
        webtransport_max_sessions: int = 1,
        **kwargs,
    ) -> None:
        self._settings_profile = settings_profile
        self._webtransport_max_sessions = webtransport_max_sessions
        super().__init__(*args, **kwargs)

    def _get_local_settings(self) -> Dict[int, int]:
        settings = super()._get_local_settings()
        if self._settings_profile == SETTINGS_PROFILE_YGGDRASIL:
            settings[H3_DATAGRAM_DRAFT04] = 1
            settings[WT_ENABLED] = 1
            settings[WT_MAX_SESSIONS_DRAFT07] = self._webtransport_max_sessions
            settings[WT_MAX_SESSIONS] = self._webtransport_max_sessions
            settings[WT_INITIAL_MAX_DATA] = WT_INITIAL_MAX_DATA_VALUE
            settings[WT_INITIAL_MAX_STREAMS_UNI] = WT_INITIAL_MAX_STREAMS_VALUE
            settings[WT_INITIAL_MAX_STREAMS_BIDI] = WT_INITIAL_MAX_STREAMS_VALUE
        return settings


class WebTransportEchoProtocol(QuicConnectionProtocol):
    def __init__(
        self,
        *args,
        flush_settings_on_negotiate: bool = True,
        settings_profile: str = SETTINGS_PROFILE_AIOQUIC,
        webtransport_max_sessions: int = 1,
        **kwargs,
    ) -> None:
        super().__init__(*args, **kwargs)
        self._http: Optional[H3Connection] = None
        self._sessions: Dict[int, str] = {}
        self._logged_settings = False
        self._flush_settings_on_negotiate = flush_settings_on_negotiate
        self._settings_profile = settings_profile
        self._webtransport_max_sessions = webtransport_max_sessions

    def quic_event_received(self, event: QuicEvent) -> None:
        if isinstance(event, ProtocolNegotiated):
            LOG.info("QUIC negotiated alpn=%s", event.alpn_protocol)
            if event.alpn_protocol in H3_ALPN:
                self._http = EchoH3Connection(
                    self._quic,
                    enable_webtransport=True,
                    settings_profile=self._settings_profile,
                    webtransport_max_sessions=self._webtransport_max_sessions,
                )
                LOG.info(
                    "server H3 settings profile=%s settings=%s",
                    self._settings_profile,
                    format_settings(self._http.sent_settings or {}),
                )
                if self._flush_settings_on_negotiate:
                    self.transmit()
                    LOG.info("server H3 settings flushed immediately after ALPN")
            else:
                LOG.warning("unexpected ALPN %s", event.alpn_protocol)

        if isinstance(event, HandshakeCompleted):
            LOG.info(
                "QUIC handshake completed alpn=%s early_data_accepted=%s session_resumed=%s",
                event.alpn_protocol,
                event.early_data_accepted,
                event.session_resumed,
            )

        if isinstance(event, StreamReset):
            LOG.warning(
                "QUIC stream reset by peer stream=%s error_code=%s",
                event.stream_id,
                event.error_code,
            )

        if isinstance(event, StreamDataReceived):
            LOG.debug(
                "QUIC stream data stream=%s bytes=%s end=%s prefix=%s",
                event.stream_id,
                len(event.data),
                event.end_stream,
                event.data[:24].hex(),
            )

        if isinstance(event, ConnectionTerminated):
            LOG.info(
                "QUIC closed error_code=%s frame_type=%s reason=%r",
                event.error_code,
                event.frame_type,
                event.reason_phrase,
            )

        if self._http is None:
            return

        for http_event in self._http.handle_event(event):
            self._log_settings_once()
            self._handle_h3_event(http_event)

    def _handle_h3_event(self, event: H3Event) -> None:
        if isinstance(event, HeadersReceived):
            self._handle_headers(event)
        elif isinstance(event, DatagramReceived):
            self._handle_datagram(event)
        elif isinstance(event, WebTransportStreamDataReceived):
            self._handle_webtransport_stream(event)

    def _handle_headers(self, event: HeadersReceived) -> None:
        headers = header_map(event.headers)
        LOG.info(
            "H3 headers stream=%s ended=%s headers=%s",
            event.stream_id,
            event.stream_ended,
            format_headers(event.headers),
        )

        method = headers.get(b":method", b"").decode("utf8", "replace")
        protocol = headers.get(b":protocol", b"").decode("utf8", "replace")
        path = headers.get(b":path", b"/").decode("utf8", "replace").split("?", 1)[0]

        if method == "GET" and path == "/healthz":
            body = b"ok\n"
            self._http.send_headers(
                stream_id=event.stream_id,
                headers=[
                    (b":status", b"200"),
                    (b"content-type", b"text/plain; charset=utf-8"),
                    (b"content-length", str(len(body)).encode()),
                    (b"cache-control", b"no-store"),
                ],
            )
            self._http.send_data(stream_id=event.stream_id, data=body, end_stream=True)
            LOG.info("H3 health check accepted stream=%s", event.stream_id)
            self.transmit()
            return

        if method == "CONNECT" and protocol in {"webtransport", "webtransport-h3"} and path.startswith("/wt"):
            response = response_headers(path, event.headers)
            self._sessions[event.stream_id] = path
            LOG.info(
                "WebTransport CONNECT accepted session=%s path=%s response=%s",
                event.stream_id,
                path,
                format_headers(response),
            )
            self._http.send_headers(stream_id=event.stream_id, headers=response)
            self.transmit()
            return

        LOG.warning(
            "non-WebTransport request rejected stream=%s method=%s protocol=%s path=%s",
            event.stream_id,
            method,
            protocol,
            path,
        )
        self._http.send_headers(
            stream_id=event.stream_id,
            headers=[(b":status", b"404"), (b"cache-control", b"no-store")],
            end_stream=True,
        )
        self.transmit()

    def _handle_datagram(self, event: DatagramReceived) -> None:
        session_id = datagram_session_id(event)
        path = self._sessions.get(session_id)
        LOG.info(
            "datagram received session=%s path=%s bytes=%s hex=%s",
            session_id,
            path,
            len(event.data),
            event.data[:24].hex(),
        )
        if path is None:
            return
        # aioquic 1.3 names this HTTP/3 context stream_id, while the local
        # compatibility checkout still calls it flow_id. Positional dispatch
        # works with both APIs.
        self._http.send_datagram(session_id, event.data)
        LOG.info("datagram echoed session=%s bytes=%s", session_id, len(event.data))
        self.transmit()

    def _handle_webtransport_stream(self, event: WebTransportStreamDataReceived) -> None:
        path = self._sessions.get(event.session_id)
        direction = "uni" if stream_is_unidirectional(event.stream_id) else "bidi"
        LOG.info(
            "WT stream data session=%s path=%s stream=%s direction=%s bytes=%s ended=%s hex=%s",
            event.session_id,
            path,
            event.stream_id,
            direction,
            len(event.data),
            event.stream_ended,
            event.data[:24].hex(),
        )
        if path is None:
            return

        if stream_is_unidirectional(event.stream_id):
            stream_id = self._http.create_webtransport_stream(
                event.session_id,
                is_unidirectional=True,
            )
            self._quic.send_stream_data(
                stream_id=stream_id,
                data=event.data,
                end_stream=event.stream_ended,
            )
            LOG.info(
                "WT uni echoed session=%s client_stream=%s server_stream=%s bytes=%s ended=%s",
                event.session_id,
                event.stream_id,
                stream_id,
                len(event.data),
                event.stream_ended,
            )
        else:
            self._quic.send_stream_data(
                stream_id=event.stream_id,
                data=event.data,
                end_stream=event.stream_ended,
            )
            LOG.info(
                "WT bidi echoed session=%s stream=%s bytes=%s ended=%s",
                event.session_id,
                event.stream_id,
                len(event.data),
                event.stream_ended,
            )
        self.transmit()

    def _log_settings_once(self) -> None:
        if self._logged_settings or self._http is None:
            return
        settings = self._http.received_settings
        if settings is None:
            return
        self._logged_settings = True
        LOG.info("client H3 settings=%s", format_settings(settings))


def response_headers(path: str, request_headers: Headers) -> Headers:
    headers: Headers = [(b":status", b"200")]

    if path == "/wt/protocol":
        headers.append((b"wt-protocol", b'"quicast-wt-v0"'))
    elif path == "/wt/capsule":
        headers.extend(
            [
                (b"wt-protocol", b'"quicast-wt-v0"'),
                (b"capsule-protocol", b"?1"),
            ]
        )
    elif path == "/wt/init":
        headers.extend(
            [
                (b"wt-protocol", b'"quicast-wt-v0"'),
                (b"capsule-protocol", b"?1"),
                (b"webtransport-init", WEBTRANSPORT_INIT),
            ]
        )
    elif path == "/wt/draft":
        headers.append((b"sec-webtransport-http3-draft", b"draft02"))
    elif path in {"/wt/basic", "/wt/h3-token", "/wt/yggdrasil", "/wt/auto", "/wt", "/wt/"}:
        pass
    else:
        LOG.warning("unknown /wt path %s; using basic response", path)

    available = header_map(request_headers).get(b"wt-available-protocols", b"")
    if path == "/wt/yggdrasil" or (
        path == "/wt/auto" and available_protocols_contains(available, QUICAST_PROTOCOL)
    ):
        headers.append((b"server", b"yggdrasil"))
        headers.append((b"wt-protocol", b'"quicast-wt-v0"'))
        headers.append((b"capsule-protocol", b"?1"))
        headers.append((b"webtransport-init", WEBTRANSPORT_INIT))
        origin = header_map(request_headers).get(b"origin")
        if origin is not None:
            headers.append((b"access-control-allow-origin", origin))
            headers.append((b"access-control-allow-credentials", b"true"))
            headers.append((b"vary", b"origin"))

    return headers


def available_protocols_contains(value: bytes, protocol: bytes) -> bool:
    for item in value.decode("utf8", "replace").split(","):
        token = item.split(";", 1)[0].strip().strip('"')
        if token.encode() == protocol:
            return True
    return False


def datagram_session_id(event: DatagramReceived) -> int:
    stream_id = getattr(event, "stream_id", None)
    if stream_id is not None:
        return int(stream_id)
    flow_id = getattr(event, "flow_id", None)
    if flow_id is not None:
        return int(flow_id)
    raise AttributeError("DatagramReceived has neither stream_id nor flow_id")


def header_map(headers: Headers) -> Dict[bytes, bytes]:
    return {name.lower(): value for name, value in headers}


def format_headers(headers: Headers) -> List[Tuple[str, str]]:
    return [
        (name.decode("utf8", "replace"), value.decode("utf8", "replace"))
        for name, value in headers
    ]


def format_settings(settings: Dict[int, int]) -> Dict[str, int]:
    names = {
        int(Setting.ENABLE_CONNECT_PROTOCOL): "enable_connect_protocol",
        int(Setting.H3_DATAGRAM): "h3_datagram",
        int(Setting.ENABLE_WEBTRANSPORT): "enable_webtransport",
        int(Setting.QPACK_MAX_TABLE_CAPACITY): "qpack_max_table_capacity",
        int(Setting.QPACK_BLOCKED_STREAMS): "qpack_blocked_streams",
        int(Setting.MAX_FIELD_SECTION_SIZE): "max_field_section_size",
        H3_DATAGRAM_DRAFT04: "h3_datagram_draft04",
        WT_ENABLED: "wt_enabled",
        WT_MAX_SESSIONS_DRAFT07: "webtransport_max_sessions_draft07",
        WT_MAX_SESSIONS: "webtransport_max_sessions",
        WT_INITIAL_MAX_DATA: "wt_initial_max_data",
        WT_INITIAL_MAX_STREAMS_UNI: "wt_initial_max_streams_uni",
        WT_INITIAL_MAX_STREAMS_BIDI: "wt_initial_max_streams_bidi",
    }
    return {names.get(int(key), f"0x{int(key):x}"): value for key, value in settings.items()}


async def run(args: argparse.Namespace) -> None:
    secrets_log: Optional[IO[str]] = None

    if args.reset_stream_at_tp:
        install_reset_stream_at_transport_parameter()
        LOG.info(
            "server QUIC transport parameters include reset_stream_at id=0x%x",
            RESET_STREAM_AT_TRANSPORT_PARAMETER_ID,
        )

    configuration = QuicConfiguration(
        alpn_protocols=H3_ALPN,
        idle_timeout=args.idle_timeout,
        is_client=False,
        max_data=args.max_data,
        max_datagram_frame_size=args.max_datagram_frame_size,
        max_stream_data=args.max_stream_data,
        quantum_readiness_test=args.quantum_readiness,
    )
    if args.qlog_dir:
        Path(args.qlog_dir).mkdir(parents=True, exist_ok=True)
        configuration.quic_logger = QuicFileLogger(args.qlog_dir)
    if args.secrets_log:
        secrets_log = open(args.secrets_log, "a", encoding="utf8")
        configuration.secrets_log_file = secrets_log
    configuration.load_cert_chain(args.cert, args.key)

    await serve(
        args.host,
        args.port,
        configuration=configuration,
        create_protocol=lambda *protocol_args, **protocol_kwargs: WebTransportEchoProtocol(
            *protocol_args,
            flush_settings_on_negotiate=not args.no_flush_settings_on_negotiate,
            settings_profile=args.settings_profile,
            webtransport_max_sessions=args.webtransport_max_sessions,
            **protocol_kwargs,
        ),
        retry=args.retry,
    )
    LOG.info("aioquic WebTransport echo listening on https://%s:%s/wt/basic", args.host, args.port)
    try:
        await asyncio.Future()
    finally:
        if secrets_log is not None:
            secrets_log.close()


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="aioquic WebTransport echo lab")
    parser.add_argument("--host", default="::", help="listen host")
    parser.add_argument("--port", type=int, default=9443, help="listen UDP port")
    parser.add_argument("--cert", default="certs/localhost.pem", help="TLS certificate")
    parser.add_argument("--key", default="certs/localhost-key.pem", help="TLS private key")
    parser.add_argument("--idle-timeout", type=float, default=60.0)
    parser.add_argument("--max-data", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--max-datagram-frame-size", type=int, default=65536)
    parser.add_argument("--max-stream-data", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--no-flush-settings-on-negotiate", action="store_true")
    parser.add_argument("--qlog-dir", help="write one qlog file per QUIC connection")
    parser.add_argument("--quantum-readiness", action="store_true")
    parser.add_argument("--retry", action="store_true", help="enable QUIC retry")
    parser.add_argument(
        "--reset-stream-at-tp",
        action="store_true",
        help="advertise the zero-length RESET_STREAM_AT QUIC transport parameter used by quiche/Yggdrasil",
    )
    parser.add_argument("--secrets-log", help="append TLS traffic secrets for packet capture analysis")
    parser.add_argument(
        "--settings-profile",
        choices=[SETTINGS_PROFILE_AIOQUIC, SETTINGS_PROFILE_YGGDRASIL],
        default=SETTINGS_PROFILE_AIOQUIC,
        help="server HTTP/3 SETTINGS shape to advertise",
    )
    parser.add_argument(
        "--webtransport-max-sessions",
        type=positive_int,
        default=1,
        help="WebTransport max-sessions SETTINGS value for the Yggdrasil profile",
    )
    parser.add_argument("-v", "--verbose", action="store_true")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    logging.basicConfig(
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
        level=logging.DEBUG if args.verbose else logging.INFO,
    )
    try:
        asyncio.run(run(args))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
