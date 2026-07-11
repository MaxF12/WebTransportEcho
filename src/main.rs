use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use bytes::{Buf, Bytes, BytesMut};
use h3::ext::Protocol;
use http::{Method, Request, Response, StatusCode};
use quinn::{
    crypto::rustls::QuicServerConfig,
    rustls::pki_types::{CertificateDer, PrivateKeyDer},
    Endpoint, RecvStream, SendStream, ServerConfig, TransportConfig, VarInt,
};
use tokio::time;
use tracing::{debug, error, info, warn};

const DEFAULT_LISTEN: &str = "[::]:9443";
const DEFAULT_CERT: &str = "certs/localhost.pem";
const DEFAULT_KEY: &str = "certs/localhost-key.pem";
const WT_BIDI_STREAM_TYPE: u64 = 0x41;
const WT_UNI_STREAM_TYPE: u64 = 0x54;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let options = Options::parse()?;
    let endpoint = Endpoint::server(server_config(&options.cert, &options.key)?, options.listen)
        .with_context(|| format!("binding QUIC endpoint on {}", options.listen))?;

    let local_addr = endpoint.local_addr()?;
    eprintln!("WebTransport echo server listening on https://{local_addr}/wt/basic");
    info!(listen = %local_addr, "webtransport echo server listening");
    info!(cert = %options.cert.display(), key = %options.key.display(), "using certificate");

    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    break;
                };
                info!(remote = %incoming.remote_address(), "udp/quic packet accepted by endpoint");
                tokio::spawn(async move {
                    if let Err(error) = handle_incoming(incoming).await {
                        error!(error = %format_error_chain(&error), "connection task failed");
                    }
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("waiting for ctrl-c")?;
                info!("shutting down");
                endpoint.close(VarInt::from_u32(0), b"shutdown");
                break;
            }
        }
    }

    endpoint.wait_idle().await;
    Ok(())
}

async fn handle_incoming(incoming: quinn::Incoming) -> Result<()> {
    let remote = incoming.remote_address();
    let conn = incoming.await.context("accepting QUIC connection")?;
    let alpn = conn
        .handshake_data()
        .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .and_then(|data| data.protocol)
        .map(|protocol| String::from_utf8_lossy(&protocol).into_owned())
        .unwrap_or_else(|| "<none>".to_string());

    info!(
        %remote,
        alpn,
        max_datagram_size = ?conn.max_datagram_size(),
        "quic connection established"
    );

    let session = accept_webtransport_connect(conn.clone()).await?;
    info!(
        %remote,
        path = %session.path,
        stream_id = session.stream_id,
        expected_session_id = session.expected_session_id,
        "webtransport session established"
    );

    let keepalive = tokio::spawn(hold_connect_stream(
        session.request_stream,
        session.path.clone(),
    ));
    let datagrams = tokio::spawn(echo_datagrams(conn.clone(), session.expected_session_id));
    let bidi = tokio::spawn(accept_bidi_streams(
        conn.clone(),
        session.expected_session_id,
    ));
    let uni = tokio::spawn(accept_uni_streams(
        conn.clone(),
        session.expected_session_id,
    ));

    conn.closed().await;
    info!(%remote, "quic connection closed");

    keepalive.abort();
    datagrams.abort();
    bidi.abort();
    uni.abort();

    Ok(())
}

struct Session {
    path: String,
    stream_id: u64,
    expected_session_id: u64,
    request_stream: h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
}

async fn accept_webtransport_connect(conn: quinn::Connection) -> Result<Session> {
    let h3_conn = h3_quinn::Connection::new(conn);
    let mut h3_conn = h3::server::builder()
        .send_grease(false)
        .enable_extended_connect(true)
        .enable_webtransport(true)
        .enable_datagram(true)
        .max_webtransport_sessions(64)
        .build(h3_conn)
        .await
        .context("building H3 server connection")?;

    let resolver = time::timeout(Duration::from_secs(20), h3_conn.accept())
        .await
        .context("timed out waiting for CONNECT")?
        .context("H3 accept failed")?
        .ok_or_else(|| anyhow!("connection closed before CONNECT"))?;

    let (request, mut stream) = resolver
        .resolve_request()
        .await
        .context("resolving H3 request")?;
    log_request(&request);

    if request.method() != Method::CONNECT {
        warn!(method = %request.method(), uri = %request.uri(), "non-CONNECT request rejected");
        stream
            .send_response(Response::builder().status(StatusCode::NOT_FOUND).body(())?)
            .await
            .context("sending 404")?;
        stream.finish().await.context("finishing 404")?;
        bail!("first request was not WebTransport CONNECT");
    }

    let protocol = request
        .extensions()
        .get::<Protocol>()
        .map(Protocol::as_str)
        .unwrap_or("<none>");
    if protocol != "webtransport" {
        warn!(protocol, "unexpected CONNECT :protocol");
    }

    let path = request.uri().path().to_string();
    if !path.starts_with("/wt") {
        warn!(%path, "CONNECT path rejected");
        stream
            .send_response(Response::builder().status(StatusCode::NOT_FOUND).body(())?)
            .await
            .context("sending 404")?;
        stream.finish().await.context("finishing 404")?;
        bail!("CONNECT path is outside /wt");
    }

    let response = response_for_path(&path)?;
    log_response(&path, &response);
    stream
        .send_response(response)
        .await
        .context("sending WebTransport CONNECT response")?;

    let stream_id = stream.send_id().into_inner();
    let expected_session_id = stream_id / 4;

    // Drop the H3 connection after the CONNECT response so raw Quinn accepts the
    // subsequent WebTransport streams instead of the generic H3 request path.
    drop(h3_conn);

    Ok(Session {
        path,
        stream_id,
        expected_session_id,
        request_stream: stream,
    })
}

async fn hold_connect_stream(
    mut stream: h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    path: String,
) {
    loop {
        match stream.recv_data().await {
            Ok(Some(mut data)) => {
                let bytes = data.copy_to_bytes(data.remaining());
                info!(%path, bytes = bytes.len(), hex = %hex_preview(&bytes), "connect body bytes");
            }
            Ok(None) => {
                info!(%path, "connect request body ended");
                break;
            }
            Err(error) => {
                warn!(%path, %error, "connect body read failed");
                break;
            }
        }
    }
}

async fn echo_datagrams(conn: quinn::Connection, expected_session_id: u64) {
    loop {
        match conn.read_datagram().await {
            Ok(datagram) => {
                let (prefix, prefix_len) = read_varint(&datagram).unwrap_or((u64::MAX, 0));
                info!(
                    bytes = datagram.len(),
                    prefix,
                    prefix_len,
                    expected_session_id,
                    hex = %hex_preview(&datagram),
                    "datagram received"
                );
                if let Err(error) = conn.send_datagram(datagram.clone()) {
                    warn!(%error, "datagram echo send failed");
                } else {
                    info!(bytes = datagram.len(), "datagram echoed");
                }
            }
            Err(error) => {
                debug!(%error, "datagram loop ended");
                break;
            }
        }
    }
}

async fn accept_bidi_streams(conn: quinn::Connection, expected_session_id: u64) {
    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                info!("bidirectional QUIC stream opened");
                tokio::spawn(async move {
                    if let Err(error) = echo_bidi_stream(send, recv, expected_session_id).await {
                        warn!(%error, "bidirectional stream failed");
                    }
                });
            }
            Err(error) => {
                debug!(%error, "bidirectional accept loop ended");
                break;
            }
        }
    }
}

async fn accept_uni_streams(conn: quinn::Connection, expected_session_id: u64) {
    loop {
        match conn.accept_uni().await {
            Ok(recv) => {
                info!("unidirectional QUIC stream opened");
                let conn = conn.clone();
                tokio::spawn(async move {
                    if let Err(error) = echo_uni_stream(conn, recv, expected_session_id).await {
                        warn!(%error, "unidirectional stream failed");
                    }
                });
            }
            Err(error) => {
                debug!(%error, "unidirectional accept loop ended");
                break;
            }
        }
    }
}

async fn echo_bidi_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    expected_session_id: u64,
) -> Result<()> {
    let mut prefix = PrefixReader::default();
    let mut first_payload = Vec::new();
    while prefix.values.len() < 2 {
        let Some(bytes) = recv
            .read_chunk(4096, true)
            .await
            .context("reading WT bidi prefix")?
        else {
            bail!("bidi stream ended before WT prefix");
        };
        prefix.push(&bytes.bytes, &mut first_payload)?;
    }

    let stream_type = prefix.values[0];
    let session_id = prefix.values[1];
    info!(
        stream_type = format_args!("0x{stream_type:x}"),
        session_id,
        expected_session_id,
        first_payload_bytes = first_payload.len(),
        "webtransport bidi preface"
    );

    if stream_type != WT_BIDI_STREAM_TYPE {
        bail!("unexpected bidi stream preface type 0x{stream_type:x}");
    }

    if !first_payload.is_empty() {
        info!(bytes = first_payload.len(), "bidi payload received");
        send.write_all(&first_payload)
            .await
            .context("echoing first bidi payload")?;
    }

    let mut buf = [0_u8; 16 * 1024];
    loop {
        match recv.read(&mut buf).await.context("reading bidi payload")? {
            Some(0) => {}
            Some(n) => {
                info!(bytes = n, "bidi payload received");
                send.write_all(&buf[..n])
                    .await
                    .context("echoing bidi payload")?;
            }
            None => {
                info!("bidi stream receive side ended");
                send.finish().context("finishing bidi send side")?;
                break;
            }
        }
    }

    Ok(())
}

async fn echo_uni_stream(
    conn: quinn::Connection,
    mut recv: RecvStream,
    expected_session_id: u64,
) -> Result<()> {
    let mut prefix = PrefixReader::default();
    let mut first_payload = Vec::new();
    while prefix.values.len() < 2 {
        let Some(bytes) = recv
            .read_chunk(4096, true)
            .await
            .context("reading WT uni prefix")?
        else {
            bail!("uni stream ended before WT prefix");
        };
        prefix.push(&bytes.bytes, &mut first_payload)?;
    }

    let stream_type = prefix.values[0];
    let session_id = prefix.values[1];
    info!(
        stream_type = format_args!("0x{stream_type:x}"),
        session_id,
        expected_session_id,
        first_payload_bytes = first_payload.len(),
        "webtransport uni preface"
    );

    if stream_type != WT_UNI_STREAM_TYPE {
        bail!("unexpected uni stream preface type 0x{stream_type:x}");
    }

    let mut payload = BytesMut::from(first_payload.as_slice());
    let mut buf = [0_u8; 16 * 1024];
    loop {
        match recv.read(&mut buf).await.context("reading uni payload")? {
            Some(0) => {}
            Some(n) => {
                info!(bytes = n, "uni payload received");
                payload.extend_from_slice(&buf[..n]);
            }
            None => break,
        }
    }

    let mut send = conn.open_uni().await.context("opening server uni echo")?;
    let mut preface = Vec::new();
    write_varint(WT_UNI_STREAM_TYPE, &mut preface);
    write_varint(session_id, &mut preface);
    send.write_all(&preface)
        .await
        .context("writing server uni WT preface")?;
    send.write_all(&payload)
        .await
        .context("writing server uni echo")?;
    send.finish().context("finishing server uni echo")?;
    info!(
        bytes = payload.len(),
        session_id, "uni payload echoed on server uni stream"
    );

    Ok(())
}

fn response_for_path(path: &str) -> Result<Response<()>> {
    let mut builder = Response::builder().status(StatusCode::OK);

    match path {
        "/wt/basic" | "/wt/h3-token" | "/wt" | "/wt/" => {}
        "/wt/protocol" => {
            builder = builder.header("wt-protocol", "\"quicast-wt-v0\"");
        }
        "/wt/capsule" => {
            builder = builder
                .header("wt-protocol", "\"quicast-wt-v0\"")
                .header("capsule-protocol", "?1");
        }
        "/wt/init" => {
            builder = builder
                .header("wt-protocol", "\"quicast-wt-v0\"")
                .header("capsule-protocol", "?1")
                .header("webtransport-init", "u=8388608, bl=8388608, br=8388608");
        }
        "/wt/draft" => {
            builder = builder.header("sec-webtransport-http3-draft", "draft02");
        }
        _ => {
            warn!(%path, "unknown /wt path; using basic response");
        }
    }

    Ok(builder.body(())?)
}

fn log_request(request: &Request<()>) {
    let protocol = request
        .extensions()
        .get::<Protocol>()
        .map(Protocol::as_str)
        .unwrap_or("<none>");
    let mut headers = BTreeMap::new();
    for (name, value) in request.headers() {
        headers.insert(name.as_str().to_string(), header_value(value));
    }

    info!(
        method = %request.method(),
        uri = %request.uri(),
        protocol,
        origin = headers.get("origin").map(String::as_str).unwrap_or("<none>"),
        wt_available_protocols = headers
            .get("wt-available-protocols")
            .map(String::as_str)
            .unwrap_or("<none>"),
        "connect request"
    );
    debug!(?headers, "connect request headers");
}

fn log_response(path: &str, response: &Response<()>) {
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        headers.insert(name.as_str().to_string(), header_value(value));
    }
    info!(%path, status = %response.status(), ?headers, "connect response");
}

fn header_value(value: &http::HeaderValue) -> String {
    value
        .to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|_| format!("{:?}", value.as_bytes()))
}

#[derive(Default)]
struct PrefixReader {
    pending: Vec<u8>,
    values: Vec<u64>,
}

impl PrefixReader {
    fn push(&mut self, chunk: &[u8], payload: &mut Vec<u8>) -> Result<()> {
        self.pending.extend_from_slice(chunk);

        loop {
            if self.values.len() >= 2 {
                payload.extend_from_slice(&self.pending);
                self.pending.clear();
                return Ok(());
            }

            match read_varint(&self.pending) {
                Some((value, len)) => {
                    self.values.push(value);
                    self.pending.drain(..len);
                }
                None => return Ok(()),
            }
        }
    }
}

fn read_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let len = match first >> 6 {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => 8,
    };
    if bytes.len() < len {
        return None;
    }

    let mut value = u64::from(first & 0x3f);
    for byte in &bytes[1..len] {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, len))
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

fn format_error_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn server_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig> {
    let certs = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("building rustls server config")?;
    tls.alpn_protocols = vec![b"h3".to_vec()];

    let mut server_config = ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    let mut transport = TransportConfig::default();
    transport.max_concurrent_bidi_streams(128_u32.into());
    transport.max_concurrent_uni_streams(128_u32.into());
    transport.datagram_receive_buffer_size(Some(4 * 1024 * 1024));
    transport.datagram_send_buffer_size(4 * 1024 * 1024);
    server_config.transport = Arc::new(transport);

    Ok(server_config)
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let sections = pem_sections(path)?;
    let certs = sections
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| CertificateDer::from(der))
        .collect::<Vec<_>>();

    if certs.is_empty() {
        bail!("no CERTIFICATE sections found in {}", path.display());
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let sections = pem_sections(path)?;
    for (label, der) in sections {
        if matches!(
            label.as_str(),
            "PRIVATE KEY" | "RSA PRIVATE KEY" | "EC PRIVATE KEY"
        ) {
            return PrivateKeyDer::try_from(der)
                .map_err(|error| anyhow!("{error}"))
                .with_context(|| format!("parsing private key from {}", path.display()));
        }
    }

    bail!("no private key section found in {}", path.display());
}

fn pem_sections(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut sections = Vec::new();
    let mut current_label: Option<String> = None;
    let mut current_body = String::new();

    for line in text.lines() {
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|s| s.strip_suffix("-----"))
        {
            current_label = Some(label.to_string());
            current_body.clear();
        } else if let Some(label) = line
            .strip_prefix("-----END ")
            .and_then(|s| s.strip_suffix("-----"))
        {
            let begin = current_label.take().ok_or_else(|| {
                anyhow!("END {label} without matching BEGIN in {}", path.display())
            })?;
            if begin != label {
                bail!(
                    "PEM section mismatch in {}: BEGIN {}, END {}",
                    path.display(),
                    begin,
                    label
                );
            }
            let der = base64::engine::general_purpose::STANDARD
                .decode(current_body.as_bytes())
                .with_context(|| format!("base64 decoding {label} in {}", path.display()))?;
            sections.push((label.to_string(), der));
            current_body.clear();
        } else if current_label.is_some() {
            current_body.push_str(line.trim());
        }
    }

    Ok(sections)
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "webtransport_echo=debug,info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[derive(Debug)]
struct Options {
    listen: SocketAddr,
    cert: PathBuf,
    key: PathBuf,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut listen = DEFAULT_LISTEN.parse::<SocketAddr>()?;
        let mut cert = PathBuf::from(DEFAULT_CERT);
        let mut key = PathBuf::from(DEFAULT_KEY);

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--listen" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--listen needs a value"))?;
                    listen = value
                        .parse()
                        .with_context(|| format!("parsing --listen {value}"))?;
                }
                "--cert" => {
                    cert = args
                        .next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("--cert needs a value"))?;
                }
                "--key" => {
                    key = args
                        .next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("--key needs a value"))?;
                }
                "--help" | "-h" => {
                    println!(
                        "Usage: cargo run -- [--listen {DEFAULT_LISTEN}] [--cert {DEFAULT_CERT}] [--key {DEFAULT_KEY}]"
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other}"),
            }
        }

        Ok(Self { listen, cert, key })
    }
}
