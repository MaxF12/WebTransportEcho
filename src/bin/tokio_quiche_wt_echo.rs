use std::{
    collections::{BTreeMap, HashMap},
    fs,
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::UdpSocket;
use tokio_quiche::{
    http3::{
        driver::{
            H3Event, InboundFrame, IncomingH3Headers, OutboundFrame, OutboundFrameSender,
            ServerH3Event, WebTransportDiagnosticKind, WebTransportStreamDirection,
        },
        settings::Http3Settings,
    },
    listen,
    metrics::DefaultMetrics,
    quic::QuicCommand,
    quiche::h3::{Header, NameValue},
    settings::{CertificateKind, Hooks, QuicSettings, TlsCertificatePaths},
    ConnectionParams, ServerH3Driver,
};

const DEFAULT_LISTEN: &str = "[::]:9446";
const DEFAULT_CERT: &str = "certs/localhost.pem";
const DEFAULT_KEY: &str = "certs/localhost-key.pem";
const WT_UNI_STREAM_TYPE: u64 = 0x54;
const FIRST_SERVER_WT_UNI_STREAM_ID: u64 = 15;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let options = Options::parse()?;
    options.ensure_output_dirs()?;
    let socket = UdpSocket::bind(options.listen)
        .await
        .with_context(|| format!("binding UDP {}", options.listen))?;

    let mut quic_settings = QuicSettings::default();
    quic_settings.enable_dgram = true;
    quic_settings.enable_reset_stream_at = options.reset_stream_at;
    quic_settings.initial_max_data = 8_388_608;
    quic_settings.initial_max_stream_data_bidi_local = 8_388_608;
    quic_settings.initial_max_stream_data_bidi_remote = 8_388_608;
    quic_settings.initial_max_stream_data_uni = 8_388_608;
    quic_settings.initial_max_streams_bidi = 100;
    quic_settings.initial_max_streams_uni = 100;
    quic_settings.max_idle_timeout = Some(Duration::from_secs(56));
    quic_settings.disable_client_ip_validation = options.disable_client_ip_validation;
    quic_settings.qlog_dir = options.qlog_dir.clone();
    quic_settings.keylog_file = options.secrets_log.clone();
    quic_settings.cc_algorithm = options.cc_algorithm.clone();
    quic_settings.grease = options.grease;

    let cert_path = options.cert.to_string_lossy().into_owned();
    let key_path = options.key.to_string_lossy().into_owned();
    let params = ConnectionParams::new_server(
        quic_settings,
        TlsCertificatePaths {
            cert: &cert_path,
            private_key: &key_path,
            kind: CertificateKind::X509,
        },
        Hooks::default(),
    );

    let mut listeners =
        listen([socket], params, DefaultMetrics).context("creating tokio-quiche listener")?;
    let mut incoming = listeners
        .pop()
        .ok_or_else(|| anyhow!("tokio-quiche returned no listeners"))?;

    eprintln!(
        "tokio-quiche WebTransport echo listening on https://localhost:{}/wt/basic",
        options.listen.port()
    );
    log::info!(
        "tokio-quiche WT echo listening listen={} cert={} key={} reset_stream_at={} grease={}",
        options.listen,
        options.cert.display(),
        options.key.display(),
        options.reset_stream_at,
        options.grease
    );

    loop {
        tokio::select! {
            accepted = incoming.next() => {
                let Some(accepted) = accepted else {
                    break;
                };

                match accepted {
                    Ok(conn) => {
                        log::info!("QUIC initial accepted by tokio-quiche");
                        let (driver, mut controller) = ServerH3Driver::new(http3_settings());
                        let cmd_sender = controller.cmd_sender();
                        conn.start(driver);

                        tokio::spawn(async move {
                            if let Err(error) = serve_connection(
                                controller.event_receiver_mut(),
                                cmd_sender,
                            ).await {
                                log::error!("connection task failed: {error:#}");
                            }
                        });
                    }
                    Err(error) => {
                        log::warn!("failed accepting QUIC connection: {error}");
                    }
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("waiting for ctrl-c")?;
                log::info!("shutdown requested");
                break;
            }
        }
    }

    Ok(())
}

fn http3_settings() -> Http3Settings {
    Http3Settings {
        enable_extended_connect: true,
        enable_webtransport: true,
        qpack_max_table_capacity: Some(65_536),
        qpack_blocked_streams: Some(100),
        post_accept_timeout: Some(Duration::from_secs(20)),
        ..Default::default()
    }
}

struct Session {
    _path: String,
    _send: OutboundFrameSender,
}

async fn serve_connection(
    events: &mut tokio_quiche::http3::driver::ServerEventStream,
    cmd_sender: tokio_quiche::http3::driver::RequestSender<
        tokio_quiche::http3::driver::ServerH3Command,
        QuicCommand,
    >,
) -> Result<()> {
    let mut sessions = BTreeMap::<u64, Session>::new();
    let mut uni_payloads = HashMap::<u64, (u64, Vec<u8>)>::new();
    let mut next_server_uni_stream_id = FIRST_SERVER_WT_UNI_STREAM_ID;

    while let Some(event) = events.recv().await {
        match event {
            ServerH3Event::Headers {
                incoming_headers, ..
            } => {
                handle_headers(incoming_headers, &mut sessions).await?;
            }
            ServerH3Event::Core(core) => match core {
                H3Event::IncomingSettings { settings } => {
                    log::info!(
                        "client H3 SETTINGS received: {}",
                        format_settings(&settings)
                    );
                }
                H3Event::NewFlow {
                    flow_id,
                    send,
                    recv,
                } => {
                    log::info!("H3 DATAGRAM flow opened flow_id={flow_id}");
                    tokio::spawn(echo_datagram_flow(flow_id, recv, send));
                }
                H3Event::WebTransportDiagnostic(diagnostic) => {
                    log::info!(
                        "WT diagnostic kind={:?} stream_id={:?} session_id={:?} direction={:?} bytes={:?} fin={:?} initial_headers={:?} header_count={:?} stream_type={:?} expected_stream_type={:?}",
                        diagnostic.kind,
                        diagnostic.stream_id,
                        diagnostic.session_id,
                        diagnostic.direction,
                        diagnostic.bytes,
                        diagnostic.fin,
                        diagnostic.initial_headers,
                        diagnostic.header_count,
                        diagnostic.stream_type,
                        diagnostic.expected_stream_type
                    );
                    if diagnostic.kind == WebTransportDiagnosticKind::H3HeadersFlushedToQuic {
                        log::info!("CONNECT response headers reached quiche for flushing");
                    }
                }
                H3Event::WebTransportStreamData {
                    session_id,
                    stream_id,
                    direction,
                    data,
                    fin,
                } => {
                    log::info!(
                        "WT stream data session_id={session_id} stream_id={stream_id} direction={direction:?} bytes={} fin={fin} hex={}",
                        data.len(),
                        hex_preview(&data)
                    );

                    match direction {
                        WebTransportStreamDirection::Bidi => {
                            queue_stream_send(&cmd_sender, stream_id, data, fin, "bidi echo");
                        }
                        WebTransportStreamDirection::Uni => {
                            let entry = uni_payloads
                                .entry(stream_id)
                                .or_insert_with(|| (session_id, Vec::new()));
                            entry.1.extend_from_slice(&data);

                            if fin {
                                let Some((session_id, payload)) = uni_payloads.remove(&stream_id)
                                else {
                                    continue;
                                };
                                let server_stream_id = next_server_uni_stream_id;
                                next_server_uni_stream_id += 4;
                                let mut out = Vec::new();
                                write_varint(WT_UNI_STREAM_TYPE, &mut out);
                                write_varint(session_id, &mut out);
                                out.extend_from_slice(&payload);

                                queue_stream_send(
                                    &cmd_sender,
                                    server_stream_id,
                                    Bytes::from(out),
                                    true,
                                    "uni echo",
                                );
                                log::info!(
                                    "queued WT uni echo client_stream_id={stream_id} server_stream_id={server_stream_id} session_id={session_id} payload_bytes={}",
                                    payload.len()
                                );
                            }
                        }
                    }
                }
                H3Event::RawStreamData {
                    stream_id,
                    data,
                    fin,
                } => {
                    log::debug!(
                        "raw stream data stream_id={stream_id} bytes={} fin={fin} hex={}",
                        data.len(),
                        hex_preview(&data)
                    );
                }
                H3Event::BodyBytesReceived {
                    stream_id,
                    num_bytes,
                    fin,
                } => {
                    log::debug!("H3 body bytes stream_id={stream_id} bytes={num_bytes} fin={fin}");
                }
                H3Event::ResetStream { stream_id } => {
                    log::warn!("H3 reset stream_id={stream_id}");
                    sessions.remove(&stream_id);
                    uni_payloads.remove(&stream_id);
                }
                H3Event::StreamClosed { stream_id } => {
                    log::info!("H3 stream closed stream_id={stream_id}");
                    sessions.remove(&stream_id);
                    uni_payloads.remove(&stream_id);
                }
                H3Event::ConnectionError(error) => {
                    bail!("H3 connection error: {error}");
                }
                H3Event::ConnectionShutdown(error) => {
                    log::info!("H3 connection shutdown error={error:?}");
                    break;
                }
                other => {
                    log::debug!("unhandled H3 event: {other:?}");
                }
            },
        }
    }

    Ok(())
}

async fn handle_headers(
    incoming: IncomingH3Headers,
    sessions: &mut BTreeMap<u64, Session>,
) -> Result<()> {
    let IncomingH3Headers {
        stream_id,
        headers,
        mut send,
        recv,
        read_fin,
        ..
    } = incoming;
    let request = RequestHeaders::from_headers(&headers);
    log::info!(
        "H3 headers stream_id={stream_id} method={} protocol={} path={} origin={} wt_available_protocols={} read_fin={read_fin}",
        request.method.as_deref().unwrap_or("<none>"),
        request.protocol.as_deref().unwrap_or("<none>"),
        request.path.as_deref().unwrap_or("<none>"),
        request.origin.as_deref().unwrap_or("<none>"),
        request
            .wt_available_protocols
            .as_deref()
            .unwrap_or("<none>")
    );
    log::debug!("raw H3 headers stream_id={stream_id}: {:?}", request.raw);

    let is_health =
        request.method.as_deref() == Some("GET") && request.path.as_deref() == Some("/healthz");
    let is_webtransport = request.method.as_deref() == Some("CONNECT")
        && matches!(
            request.protocol.as_deref(),
            Some("webtransport" | "webtransport-h3")
        )
        && request
            .path
            .as_deref()
            .is_some_and(|path| path.starts_with("/wt"));
    let status = if is_health || is_webtransport {
        200
    } else {
        404
    };

    let path = request.path.as_deref().unwrap_or("/");
    let response = response_headers(path, &request, status);
    log::info!(
        "CONNECT response stream_id={stream_id} path={path} status={status} headers={}",
        format_headers(&response)
    );

    send.send(OutboundFrame::Headers(response, None))
        .await
        .map_err(|error| anyhow!("sending response headers: {error}"))?;

    if is_health {
        send.send(OutboundFrame::Body(Bytes::from_static(b"ok\n"), true))
            .await
            .map_err(|error| anyhow!("sending health response body: {error}"))?;
        return Ok(());
    }

    if status != 200 {
        send.send(OutboundFrame::Body(Bytes::new(), true))
            .await
            .ok();
        return Ok(());
    }

    tokio::spawn(log_connect_body(stream_id, recv));
    sessions.insert(
        stream_id,
        Session {
            _path: path.to_string(),
            _send: send,
        },
    );
    log::info!("WebTransport CONNECT accepted stream_id={stream_id} path={path}");
    Ok(())
}

async fn log_connect_body(
    stream_id: u64,
    mut recv: tokio_quiche::http3::driver::InboundFrameStream,
) {
    while let Some(frame) = recv.recv().await {
        match frame {
            InboundFrame::Body(bytes, fin) => {
                log::info!(
                    "CONNECT body stream_id={stream_id} bytes={} fin={fin} hex={}",
                    bytes.len(),
                    hex_preview(bytes.as_ref())
                );
                if fin {
                    break;
                }
            }
            InboundFrame::Datagram(dgram) => {
                log::info!(
                    "CONNECT-associated datagram stream_id={stream_id} bytes={} hex={}",
                    dgram.as_ref().len(),
                    hex_preview(dgram.as_ref())
                );
            }
        }
    }
    log::info!("CONNECT body reader ended stream_id={stream_id}");
}

async fn echo_datagram_flow(
    flow_id: u64,
    mut recv: tokio_quiche::http3::driver::InboundFrameStream,
    mut send: OutboundFrameSender,
) {
    while let Some(frame) = recv.recv().await {
        match frame {
            InboundFrame::Datagram(dgram) => {
                let bytes = dgram.as_ref().len();
                log::info!(
                    "datagram received flow_id={flow_id} bytes={bytes} hex={}",
                    hex_preview(dgram.as_ref())
                );
                if let Err(error) = send.send(OutboundFrame::Datagram(dgram, flow_id)).await {
                    log::warn!("datagram echo failed flow_id={flow_id}: {error}");
                    break;
                }
                log::info!("datagram echoed flow_id={flow_id} bytes={bytes}");
            }
            InboundFrame::Body(bytes, fin) => {
                log::debug!(
                    "unexpected body on datagram flow flow_id={flow_id} bytes={} fin={fin}",
                    bytes.len()
                );
            }
        }
    }
    log::info!("datagram flow ended flow_id={flow_id}");
}

fn queue_stream_send(
    cmd_sender: &tokio_quiche::http3::driver::RequestSender<
        tokio_quiche::http3::driver::ServerH3Command,
        QuicCommand,
    >,
    stream_id: u64,
    data: Bytes,
    fin: bool,
    label: &'static str,
) {
    let bytes = data.len();
    let result = cmd_sender.send(QuicCommand::Custom(Box::new(move |qconn| {
        match qconn.stream_send(stream_id, data.as_ref(), fin) {
            Ok(sent) => log::info!(
                "{label} sent stream_id={stream_id} requested_bytes={bytes} sent_bytes={sent} fin={fin}"
            ),
            Err(error) => log::warn!(
                "{label} send failed stream_id={stream_id} bytes={bytes} fin={fin}: {error:?}"
            ),
        }
    })));

    if let Err(error) = result {
        log::warn!(
            "{label} command queue failed stream_id={stream_id} bytes={bytes} fin={fin}: {error}"
        );
    }
}

#[derive(Default)]
struct RequestHeaders {
    raw: BTreeMap<String, String>,
    method: Option<String>,
    protocol: Option<String>,
    path: Option<String>,
    origin: Option<String>,
    wt_available_protocols: Option<String>,
}

impl RequestHeaders {
    fn from_headers(headers: &[Header]) -> Self {
        let mut out = Self::default();
        for header in headers {
            let name = String::from_utf8_lossy(header.name()).to_string();
            let value = String::from_utf8_lossy(header.value()).to_string();
            match name.as_str() {
                ":method" => out.method = Some(value.clone()),
                ":protocol" => out.protocol = Some(value.clone()),
                ":path" => out.path = Some(value.clone()),
                "origin" => out.origin = Some(value.clone()),
                "wt-available-protocols" => out.wt_available_protocols = Some(value.clone()),
                _ => {}
            }
            out.raw.insert(name, value);
        }
        out
    }
}

fn response_headers(path: &str, request: &RequestHeaders, status: u16) -> Vec<Header> {
    let mut headers = vec![Header::new(b":status", status.to_string().as_bytes())];
    if status != 200 {
        return headers;
    }

    let negotiated = request
        .wt_available_protocols
        .as_deref()
        .is_some_and(|value| value.contains("quicast-wt-v0"));
    let yggdrasil_shape = path == "/wt/yggdrasil" || (path == "/wt/auto" && negotiated);

    match path {
        "/wt/basic" | "/wt/h3-token" | "/wt" | "/wt/" => {}
        "/wt/protocol" => {
            headers.push(Header::new(b"wt-protocol", b"\"quicast-wt-v0\""));
        }
        "/wt/capsule" => {
            headers.push(Header::new(b"wt-protocol", b"\"quicast-wt-v0\""));
            headers.push(Header::new(b"capsule-protocol", b"?1"));
        }
        "/wt/init" => push_protocol_capsule_init(&mut headers),
        "/wt/draft" => headers.push(Header::new(b"sec-webtransport-http3-draft", b"draft02")),
        "/wt/yggdrasil" | "/wt/auto" if yggdrasil_shape => {
            headers.push(Header::new(b"server", b"yggdrasil"));
            push_protocol_capsule_init(&mut headers);
            if let Some(origin) = &request.origin {
                headers.push(Header::new(
                    b"access-control-allow-origin",
                    origin.as_bytes(),
                ));
                headers.push(Header::new(b"access-control-allow-credentials", b"true"));
                headers.push(Header::new(b"vary", b"origin"));
            }
        }
        _ => {}
    }

    headers
}

fn push_protocol_capsule_init(headers: &mut Vec<Header>) {
    headers.push(Header::new(b"wt-protocol", b"\"quicast-wt-v0\""));
    headers.push(Header::new(b"capsule-protocol", b"?1"));
    headers.push(Header::new(
        b"webtransport-init",
        b"u=8388608, bl=8388608, br=8388608",
    ));
}

fn format_headers(headers: &[Header]) -> String {
    headers
        .iter()
        .map(|header| {
            format!(
                "{}={}",
                String::from_utf8_lossy(header.name()),
                String::from_utf8_lossy(header.value())
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_settings(settings: &[(u64, u64)]) -> String {
    settings
        .iter()
        .map(|(id, value)| format!("{}(0x{id:x})={value}", setting_name(*id)))
        .collect::<Vec<_>>()
        .join(",")
}

fn setting_name(id: u64) -> &'static str {
    match id {
        0x1 => "qpack_max_table_capacity",
        0x6 => "max_field_section_size",
        0x7 => "qpack_blocked_streams",
        0x8 => "enable_connect_protocol",
        0x33 => "h3_datagram",
        0x2b60_3742 => "enable_webtransport_legacy",
        0x2c7c_f000 => "wt_enabled",
        0xffd277 => "h3_datagram_draft04",
        0xc671_706a => "webtransport_max_sessions_draft07",
        0x14e9_cd29 => "webtransport_max_sessions",
        0x2b61 => "wt_initial_max_data",
        0x2b64 => "wt_initial_max_streams_uni",
        0x2b65 => "wt_initial_max_streams_bidi",
        _ => "unknown",
    }
}

fn write_varint(value: u64, out: &mut Vec<u8>) {
    if value < (1 << 6) {
        out.push(value as u8);
    } else if value < (1 << 14) {
        out.push(((value >> 8) as u8) | 0x40);
        out.push(value as u8);
    } else if value < (1 << 30) {
        out.push(((value >> 24) as u8) | 0x80);
        out.push((value >> 16) as u8);
        out.push((value >> 8) as u8);
        out.push(value as u8);
    } else {
        out.push(((value >> 56) as u8) | 0xc0);
        out.push((value >> 48) as u8);
        out.push((value >> 40) as u8);
        out.push((value >> 32) as u8);
        out.push((value >> 24) as u8);
        out.push((value >> 16) as u8);
        out.push((value >> 8) as u8);
        out.push(value as u8);
    }
}

fn hex_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(24)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Debug)]
struct Options {
    listen: SocketAddr,
    cert: PathBuf,
    key: PathBuf,
    qlog_dir: Option<String>,
    secrets_log: Option<String>,
    reset_stream_at: bool,
    grease: bool,
    disable_client_ip_validation: bool,
    cc_algorithm: String,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            listen: DEFAULT_LISTEN.parse()?,
            cert: PathBuf::from(DEFAULT_CERT),
            key: PathBuf::from(DEFAULT_KEY),
            qlog_dir: None,
            secrets_log: None,
            reset_stream_at: true,
            grease: false,
            disable_client_ip_validation: true,
            cc_algorithm: "cubic".to_string(),
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--listen" => {
                    let value = need_arg(&mut args, "--listen")?;
                    options.listen = value
                        .parse()
                        .with_context(|| format!("parsing --listen {value}"))?;
                }
                "--cert" => {
                    options.cert = PathBuf::from(need_arg(&mut args, "--cert")?);
                }
                "--key" => {
                    options.key = PathBuf::from(need_arg(&mut args, "--key")?);
                }
                "--qlog-dir" => {
                    options.qlog_dir = Some(need_arg(&mut args, "--qlog-dir")?);
                }
                "--secrets-log" => {
                    options.secrets_log = Some(need_arg(&mut args, "--secrets-log")?);
                }
                "--cc" => {
                    options.cc_algorithm = need_arg(&mut args, "--cc")?;
                }
                "--no-reset-stream-at" => {
                    options.reset_stream_at = false;
                }
                "--grease" => {
                    options.grease = true;
                }
                "--no-grease" => {
                    options.grease = false;
                }
                "--retry" => {
                    options.disable_client_ip_validation = false;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other}"),
            }
        }

        Ok(options)
    }

    fn ensure_output_dirs(&self) -> Result<()> {
        if let Some(qlog_dir) = &self.qlog_dir {
            fs::create_dir_all(qlog_dir)
                .with_context(|| format!("creating qlog dir {qlog_dir}"))?;
        }

        if let Some(secrets_log) = &self.secrets_log {
            if let Some(parent) = PathBuf::from(secrets_log).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("creating secrets-log dir {}", parent.display())
                    })?;
                }
            }
        }

        Ok(())
    }
}

fn need_arg(args: &mut impl Iterator<Item = String>, flag: &'static str) -> Result<String> {
    args.next().ok_or_else(|| anyhow!("{flag} needs a value"))
}

fn print_help() {
    println!(
        "Usage: cargo run --bin tokio-quiche-wt-echo -- \\
  [--listen {DEFAULT_LISTEN}] \\
  [--cert {DEFAULT_CERT}] \\
  [--key {DEFAULT_KEY}] \\
  [--qlog-dir DIR] \\
  [--secrets-log FILE] \\
  [--cc cubic|reno|bbr2] \\
  [--no-reset-stream-at] \\
  [--grease] \\
  [--retry]"
    );
}
