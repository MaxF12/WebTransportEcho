#!/usr/bin/env python3
import argparse
import asyncio
import json
import ssl
from typing import Dict

from aioquic.asyncio import QuicConnectionProtocol, connect
from aioquic.h3.connection import H3_ALPN, H3Connection
from aioquic.h3.events import DataReceived, HeadersReceived, WebTransportStreamDataReceived
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.events import ConnectionTerminated, QuicEvent, StreamDataReceived


class HealthProtocol(QuicConnectionProtocol):
    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self._http = H3Connection(self._quic, enable_webtransport=True)
        self._requests: Dict[int, Dict[str, object]] = {}
        self._sessions: Dict[int, asyncio.Future[int]] = {}
        self._stream_echoes: Dict[int, Dict[str, object]] = {}

    async def get(self, authority: str, path: str) -> Dict[str, object]:
        stream_id = self._quic.get_next_available_stream_id()
        waiter = self._loop.create_future()
        self._requests[stream_id] = {
            "body": bytearray(),
            "status": None,
            "waiter": waiter,
        }
        self._http.send_headers(
            stream_id=stream_id,
            headers=[
                (b":method", b"GET"),
                (b":scheme", b"https"),
                (b":authority", authority.encode()),
                (b":path", path.encode()),
                (b"user-agent", b"quicast-wttest-health/1"),
            ],
            end_stream=True,
        )
        self.transmit()
        return await asyncio.shield(waiter)

    async def open_webtransport(self, authority: str, path: str) -> int:
        session_id = self._quic.get_next_available_stream_id()
        waiter = self._loop.create_future()
        self._sessions[session_id] = waiter
        self._http.send_headers(
            stream_id=session_id,
            headers=[
                (b":method", b"CONNECT"),
                (b":scheme", b"https"),
                (b":authority", authority.encode()),
                (b":path", path.encode()),
                (b":protocol", b"webtransport"),
                (b"user-agent", b"quicast-wttest-health/1"),
            ],
        )
        self.transmit()
        return await asyncio.shield(waiter)

    async def bidi_echo(self, session_id: int, payload: bytes) -> bytes:
        stream_id = self._http.create_webtransport_stream(session_id)
        waiter = self._loop.create_future()
        self._stream_echoes[stream_id] = {"body": bytearray(), "waiter": waiter}
        self._quic.send_stream_data(stream_id=stream_id, data=payload, end_stream=True)
        self.transmit()
        return await asyncio.shield(waiter)

    def quic_event_received(self, event: QuicEvent) -> None:
        if isinstance(event, StreamDataReceived) and event.stream_id in self._stream_echoes:
            echo = self._stream_echoes[event.stream_id]
            echo["body"].extend(event.data)
            if event.end_stream:
                waiter = echo["waiter"]
                if not waiter.done():
                    waiter.set_result(bytes(echo["body"]))
                self._stream_echoes.pop(event.stream_id, None)
            return

        if isinstance(event, ConnectionTerminated):
            waiters = [request["waiter"] for request in self._requests.values()]
            waiters.extend(self._sessions.values())
            waiters.extend(echo["waiter"] for echo in self._stream_echoes.values())
            for waiter in waiters:
                if not waiter.done():
                    waiter.set_exception(
                        RuntimeError(
                            f"QUIC connection closed: code={event.error_code} "
                            f"reason={event.reason_phrase!r}"
                        )
                    )
            return

        for http_event in self._http.handle_event(event):
            stream_id = getattr(http_event, "stream_id", None)
            if isinstance(http_event, HeadersReceived) and stream_id in self._sessions:
                headers = dict(http_event.headers)
                status = headers.get(b":status")
                waiter = self._sessions.pop(stream_id)
                if status == b"200":
                    waiter.set_result(stream_id)
                else:
                    waiter.set_exception(
                        RuntimeError(f"WebTransport CONNECT returned status {status!r}")
                    )
                continue

            if isinstance(http_event, WebTransportStreamDataReceived):
                echo = self._stream_echoes.get(stream_id)
                if echo is None:
                    continue
                echo["body"].extend(http_event.data)
                if http_event.stream_ended:
                    waiter = echo["waiter"]
                    if not waiter.done():
                        waiter.set_result(bytes(echo["body"]))
                    self._stream_echoes.pop(stream_id, None)
                continue

            request = self._requests.get(stream_id)
            if request is None:
                continue
            if isinstance(http_event, HeadersReceived):
                headers = dict(http_event.headers)
                status = headers.get(b":status")
                request["status"] = int(status) if status is not None else None
            elif isinstance(http_event, DataReceived):
                request["body"].extend(http_event.data)

            if http_event.stream_ended:
                waiter = request["waiter"]
                if not waiter.done():
                    waiter.set_result(
                        {
                            "status": request["status"],
                            "body": bytes(request["body"]).decode("utf8", "replace"),
                        }
                    )
                self._requests.pop(stream_id, None)


async def probe(args: argparse.Namespace) -> None:
    configuration = QuicConfiguration(
        alpn_protocols=H3_ALPN,
        is_client=True,
        max_datagram_frame_size=65536,
        server_name=args.server_name,
    )
    if args.insecure:
        configuration.verify_mode = ssl.CERT_NONE
    elif args.ca_file:
        configuration.load_verify_locations(cafile=args.ca_file)

    authority = args.server_name if args.port == 443 else f"{args.server_name}:{args.port}"
    async with connect(
        args.connect_host,
        args.port,
        configuration=configuration,
        create_protocol=HealthProtocol,
    ) as client:
        result = await asyncio.wait_for(client.get(authority, args.path), args.timeout)
        if not args.skip_webtransport:
            session_id = await asyncio.wait_for(
                client.open_webtransport(authority, args.webtransport_path), args.timeout
            )
            payload = b"quicast-wttest-health"
            echoed = await asyncio.wait_for(client.bidi_echo(session_id, payload), args.timeout)
            result["webtransport"] = {
                "bidiEcho": echoed.decode("ascii", "replace"),
                "path": args.webtransport_path,
                "sessionId": session_id,
            }
            if echoed != payload:
                raise RuntimeError(f"unexpected WebTransport bidi echo: {echoed!r}")

    result.update(
        {
            "connectHost": args.connect_host,
            "port": args.port,
            "serverName": args.server_name,
            "path": args.path,
        }
    )
    print(json.dumps(result, sort_keys=True))
    if result["status"] != 200 or result["body"] != "ok\n":
        raise RuntimeError(f"unexpected H3 health response: {result!r}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Probe the WebTransport echo server over real HTTP/3")
    parser.add_argument("--connect-host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=9446)
    parser.add_argument("--server-name", required=True)
    parser.add_argument("--path", default="/healthz")
    parser.add_argument("--webtransport-path", default="/wt/basic")
    parser.add_argument("--skip-webtransport", action="store_true")
    parser.add_argument("--ca-file")
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--insecure", action="store_true")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        asyncio.run(probe(args))
    except Exception as error:
        raise SystemExit(f"H3 health check failed: {error}") from error


if __name__ == "__main__":
    main()
