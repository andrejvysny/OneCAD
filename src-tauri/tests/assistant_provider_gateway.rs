//! The local-provider gateway (ADR-0017) and its route on the OCAK1 bridge.
//!
//! Two lanes:
//!
//! * **Pure validation** — [`validate_provider_base`] against the encoded and
//!   ambiguous host forms it exists to refuse. No sockets, no server.
//! * **Against a real upstream** — a throwaway HTTP/1.1 server bound to
//!   `127.0.0.1:0` inside the test. It is a fixture, not product code: it speaks
//!   just enough HTTP to answer, stream, redirect and notice a client hanging up,
//!   which is what the size, redirect and cancellation gates need to observe.
//!
//! Nothing here needs the compiled sidecar or the geometry worker.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use onecad_assistant_protocol::{
    decode_frame, encode_frame, Cancel, Envelope, Hello, Principal, Req, PROTOCOL_VERSION,
    SIDECAR_TO_HOST_VERBS,
};
use onecad_lib::assistant::provider_gateway::{host_reason, path_reason};
use onecad_lib::assistant::{
    validate_provider_base, AssistantBridge, AssistantSlot, BridgeOptions, GatewayError,
    GatewayLimits, ProviderConfig, ProviderGateway, ProviderRegistry,
};

// ─────────────────────────────────────────────────────────────────────────────
// Validation — no upstream needed
// ─────────────────────────────────────────────────────────────────────────────

/// Every rejection class ADR-0017 names, with the reason each refusal must carry.
///
/// The encoded IPv4 rows are the ones that matter most: a WHATWG URL parser reads
/// `2130706433`, `0177.0.0.1` and `0x7f.1` as `127.0.0.1`, so a gateway that
/// classified the PARSER'S output would accept all three — and would accept them
/// from a settings file a user pasted from somewhere. The gateway classifies the
/// raw host text instead, which is why these are refused rather than normalised.
#[test]
fn the_base_validator_refuses_every_encoded_and_ambiguous_host_form() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "http://2130706433/v1",
            "decimal-integer IPv4",
            host_reason::NOT_A_LITERAL,
        ),
        (
            "http://0177.0.0.1/v1",
            "octal IPv4",
            host_reason::NOT_A_LITERAL,
        ),
        ("http://0x7f.1/v1", "hex IPv4", host_reason::NOT_A_LITERAL),
        (
            "http://0x7f.0.0.1/v1",
            "hex-octet IPv4",
            host_reason::NOT_A_LITERAL,
        ),
        (
            "http://127.1/v1",
            "short-form IPv4",
            host_reason::NOT_A_LITERAL,
        ),
        (
            "http://017700000001/v1",
            "octal-integer IPv4",
            host_reason::NOT_A_LITERAL,
        ),
        (
            "http://[::ffff:127.0.0.1]/v1",
            "IPv4-mapped IPv6",
            host_reason::IPV4_MAPPED,
        ),
        (
            "http://[::ffff:7f00:1]/v1",
            "IPv4-mapped IPv6 in hextet form",
            host_reason::IPV4_MAPPED,
        ),
        (
            "http://[0:0:0:0:0:0:0:1]/v1",
            "expanded IPv6 loopback",
            host_reason::NOT_CANONICAL,
        ),
        (
            "http://[::127.0.0.1]/v1",
            "IPv4-compatible IPv6",
            host_reason::NOT_LOOPBACK,
        ),
        (
            "http://[fe80::1]/v1",
            "link-local IPv6",
            host_reason::NOT_LOOPBACK,
        ),
        (
            "http://192.168.1.1/v1",
            "a private but routable IPv4",
            host_reason::NOT_LOOPBACK,
        ),
        (
            "http://0.0.0.0/v1",
            "the unspecified address",
            host_reason::NOT_LOOPBACK,
        ),
        (
            "http://api.openai.com/v1",
            "a real remote name",
            host_reason::NOT_A_LITERAL,
        ),
        (
            "http://localhost.evil.example/v1",
            "a name whose leftmost label is localhost",
            host_reason::NOT_A_LITERAL,
        ),
    ];
    for (base, why, reason) in cases {
        match validate_provider_base(base) {
            Err(GatewayError::Host { reason: got, .. }) => assert_eq!(
                got, *reason,
                "{base} ({why}) must be refused with {reason:?}, got {got:?}"
            ),
            other => panic!("{base} must be refused: {why}; got {other:?}"),
        }
    }

    // A malformed authority never reaches the host classifier at all — the URL
    // parser refuses it first. Asserted here so the rejection is recorded as
    // deliberate rather than as a row someone later notices is missing.
    assert!(
        matches!(
            validate_provider_base("http://[::1/v1"),
            Err(GatewayError::NotAUrl { .. })
        ),
        "an IPv6 literal with no closing bracket is not a URL"
    );
}

#[test]
fn the_base_validator_refuses_userinfo_a_non_http_scheme_and_a_query_base() {
    // Userinfo: the host of `http://127.0.0.1@evil.com/` is evil.com. A reader
    // who stops at the first dotted quad reads it the other way round, and so
    // does a check that looks at the wrong half of the authority.
    for (base, why) in [
        (
            "http://127.0.0.1@evil.com/",
            "userinfo hiding a remote host",
        ),
        ("http://user:pass@127.0.0.1:8080/", "populated userinfo"),
        (
            "http://@evil.com/",
            "empty userinfo, which a URL parser reports as no userinfo at all",
        ),
    ] {
        assert!(
            matches!(
                validate_provider_base(base),
                Err(GatewayError::Userinfo { .. })
            ),
            "{base} must be refused: {why}"
        );
    }

    for (base, why) in [
        ("file:///etc/passwd", "a scheme with no network semantics"),
        ("ftp://127.0.0.1/v1", "a scheme this gateway does not speak"),
        (
            "ws://127.0.0.1:8080",
            "a websocket base is not an HTTP base",
        ),
    ] {
        assert!(
            matches!(
                validate_provider_base(base),
                Err(GatewayError::Scheme { .. })
            ),
            "{base} must be refused: {why}"
        );
    }

    for (base, why) in [
        (
            "http://127.0.0.1:11434/v1?api_key=x",
            "a query in the base cannot survive having a path appended",
        ),
        (
            "http://127.0.0.1:11434/v1#frag",
            "a fragment in the base is meaningless to a request",
        ),
    ] {
        assert!(
            matches!(
                validate_provider_base(base),
                Err(GatewayError::QueryOrFragment { .. })
            ),
            "{base} must be refused: {why}"
        );
    }
}

/// The accepted set, and the normalisation ADR-0017 asks for: `localhost` becomes
/// a literal HERE, once, so that the per-request comparison is against an address
/// rather than a name a resolver is free to answer differently next time.
#[test]
fn the_base_validator_accepts_loopback_and_normalises_localhost_once() {
    let ollama = validate_provider_base("http://localhost:11434/v1").expect("a loopback base");
    assert_eq!(ollama.host().to_string(), "127.0.0.1");
    assert_eq!(ollama.port(), 11434);
    assert_eq!(ollama.path_prefix(), "/v1");
    assert_eq!(ollama.origin(), "http://127.0.0.1:11434");

    let v6 = validate_provider_base("http://[::1]:8080").expect("the IPv6 loopback");
    assert_eq!(v6.origin(), "http://[::1]:8080");
    assert_eq!(v6.path_prefix(), "");

    assert_eq!(
        validate_provider_base("http://127.9.9.9:8080")
            .expect("all of 127.0.0.0/8 is loopback")
            .host()
            .to_string(),
        "127.9.9.9"
    );
}

#[test]
fn a_non_loopback_base_is_refused_at_registration_not_at_request_time() {
    let err = ProviderGateway::new(
        vec![ProviderConfig {
            id: "cloud".into(),
            base_url: "http://api.example.com/v1".into(),
            model: "gpt".into(),
            api_key: Some("sk-x".into()),
        }],
        GatewayLimits::default(),
    )
    .expect_err("a non-loopback base must never become a registry entry");
    assert!(
        matches!(err, GatewayError::Host { .. }),
        "the registry is where this is caught, so a request never has to be: {err}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The test upstream — a fixture, not product code
// ─────────────────────────────────────────────────────────────────────────────

/// Signals a connected client going away.
#[derive(Debug)]
enum ServerEvent {
    /// The client hung up while the server was still writing a body.
    ClientHungUp,
}

struct TestUpstream {
    port: u16,
    /// Hits on `/v1/redirect-target`, which nothing may reach without following
    /// the 302 that points at it.
    redirect_target_hits: Arc<AtomicUsize>,
    events: mpsc::UnboundedReceiver<ServerEvent>,
}

impl TestUpstream {
    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }
}

/// One parsed HTTP/1.1 request head plus its body.
struct UpstreamRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

async fn start_upstream() -> TestUpstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let task_hits = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let hits = task_hits.clone();
            let events = events_tx.clone();
            tokio::spawn(async move {
                serve_connection(socket, hits, events).await;
            });
        }
    });
    TestUpstream {
        port,
        redirect_target_hits: hits,
        events: events_rx,
    }
}

async fn serve_connection(
    socket: TcpStream,
    redirect_target_hits: Arc<AtomicUsize>,
    events: mpsc::UnboundedSender<ServerEvent>,
) {
    let (mut reader, mut writer) = socket.into_split();
    let Some(request) = read_request(&mut reader).await else {
        return;
    };
    // Every response closes the connection, so a body with no length is
    // terminated by EOF and the fixture never needs chunked encoding.
    let path = request.target.split('?').next().unwrap_or("").to_string();
    match path.as_str() {
        "/v1/echo" => {
            let body = json!({
                "method": request.method,
                "target": request.target,
                "headers": request.headers,
                "bodyLen": request.body.len(),
                "body": String::from_utf8_lossy(&request.body),
            })
            .to_string();
            let _ = writer
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
        "/v1/stream" => {
            let _ = writer
                .write_all(
                    b"HTTP/1.1 200 OK\r\nconnection: close\r\n\
                      content-type: text/event-stream\r\n\r\n",
                )
                .await;
            for index in 0..4u32 {
                if writer
                    .write_all(format!("data: {index}\n\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = writer.flush().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        // Advertises more than the cap before a byte of body exists.
        "/v1/advertises-too-much" => {
            let body = vec![b'x'; 4096];
            let _ = writer
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
            let _ = writer.write_all(&body).await;
        }
        // Advertises nothing and then overruns the cap mid-body.
        "/v1/overruns" => {
            let _ = writer
                .write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n")
                .await;
            for _ in 0..8 {
                if writer.write_all(&[b'y'; 512]).await.is_err() {
                    return;
                }
                let _ = writer.flush().await;
            }
        }
        "/v1/redirect" => {
            let _ = writer
                .write_all(
                    b"HTTP/1.1 302 Found\r\nconnection: close\r\n\
                      location: /v1/redirect-target\r\ncontent-length: 0\r\n\r\n",
                )
                .await;
        }
        "/v1/redirect-target" => {
            redirect_target_hits.fetch_add(1, Ordering::SeqCst);
            let _ = writer
                .write_all(
                    b"HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: 7\r\n\r\nFOLLOWED",
                )
                .await;
        }
        // Streams until the client goes away. The point of this route is the
        // NOTICING: a cancelled bridge request must close this connection, not
        // leave it generating.
        "/v1/forever" => {
            let _ = writer
                .write_all(
                    b"HTTP/1.1 200 OK\r\nconnection: close\r\n\
                      content-type: text/event-stream\r\n\r\n",
                )
                .await;
            let hung_up = events.clone();
            // The read half sees the client's FIN as EOF; the write half sees it
            // as an error a moment later. Whichever notices first reports it.
            tokio::spawn(async move {
                let mut scratch = [0u8; 256];
                loop {
                    match reader.read(&mut scratch).await {
                        Ok(0) | Err(_) => {
                            let _ = hung_up.send(ServerEvent::ClientHungUp);
                            return;
                        }
                        Ok(_) => {}
                    }
                }
            });
            loop {
                if writer.write_all(b"data: tick\n\n").await.is_err() {
                    let _ = events.send(ServerEvent::ClientHungUp);
                    return;
                }
                let _ = writer.flush().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        _ => {
            let _ = writer
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nconnection: close\r\ncontent-length: 0\r\n\r\n",
                )
                .await;
        }
    }
    let _ = writer.flush().await;
    let _ = writer.shutdown().await;
}

async fn read_request<R: AsyncReadExt + Unpin>(reader: &mut R) -> Option<UpstreamRequest> {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(index) = find_double_crlf(&buf) {
            break index;
        }
        let mut scratch = [0u8; 1024];
        let read = reader.read(&mut scratch).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&scratch[..read]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let target = request_line.next()?.to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let body_len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < body_len {
        let mut scratch = [0u8; 1024];
        let read = reader.read(&mut scratch).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&scratch[..read]);
    }
    body.truncate(body_len);
    Some(UpstreamRequest {
        method,
        target,
        headers,
        body,
    })
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn gateway_for(upstream: &TestUpstream, limits: GatewayLimits) -> Arc<ProviderGateway> {
    Arc::new(
        ProviderGateway::new(
            vec![ProviderConfig {
                id: "local".into(),
                base_url: upstream.base_url(),
                model: "test-model".into(),
                api_key: None,
            }],
            limits,
        )
        .expect("a loopback provider"),
    )
}

async fn drain(
    stream: &mut onecad_lib::assistant::ProviderStream,
) -> Result<Vec<u8>, GatewayError> {
    let mut body = Vec::new();
    while let Some(chunk) = stream.next_chunk().await? {
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

// ─────────────────────────────────────────────────────────────────────────────
// Gateway behaviour against a real upstream
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_registered_provider_is_reached_by_id_and_the_path_is_appended_to_its_base() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());

    let mut stream = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "POST",
            "path": "/echo?stream=true",
            "headers": {"content-type": "application/json"},
            "bodyBase64": "eyJoaSI6MX0=",
        }))
        .await
        .expect("the registered provider answers");
    assert_eq!(stream.head().status, 200);
    let body: Value = serde_json::from_slice(&drain(&mut stream).await.expect("body")).unwrap();

    // The base's `/v1` prefix plus the operation path, built HERE — the sidecar
    // sent neither a host nor a scheme.
    assert_eq!(body["target"], "/v1/echo?stream=true");
    assert_eq!(body["method"], "POST");
    assert_eq!(body["body"], "{\"hi\":1}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_provider_id_is_refused_at_request_time() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());

    let err = gateway
        .fetch(json!({
            "providerId": "not-registered",
            "method": "GET",
            "path": "/echo",
        }))
        .await
        .expect_err("an id outside the registry names no base at all");
    assert_eq!(err.code(), "unknown_provider", "{err}");
    assert!(
        err.to_string().contains("local"),
        "the refusal lists what IS registered so the mismatch is visible: {err}"
    );
}

/// §5: the sidecar names an id, never a URL. A payload that tries to carry one
/// anyway must be refused, not have the extra field ignored while the id is
/// honoured — an ignored override is indistinguishable from an accepted one from
/// the caller's side.
#[tokio::test(flavor = "multi_thread")]
async fn a_payload_carrying_a_url_or_base_override_is_refused() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());

    for field in ["url", "baseUrl", "base", "origin", "host", "endpoint"] {
        let err = gateway
            .fetch(json!({
                "providerId": "local",
                "method": "GET",
                "path": "/echo",
                field: "http://169.254.169.254/latest/meta-data/",
            }))
            .await
            .expect_err("a payload may not carry an endpoint of its own");
        assert_eq!(err.code(), "bad_request", "{field}: {err}");
        assert!(
            err.to_string().contains(field),
            "the refusal must name the field it refused ({field}): {err}"
        );
    }

    // And a path that spells out a URL is refused by the path check.
    let err = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "GET",
            "path": "//169.254.169.254/latest/meta-data/",
        }))
        .await
        .expect_err("a protocol-relative path is an authority in disguise");
    match err {
        GatewayError::BadPath { reason, .. } => {
            assert_eq!(reason, path_reason::PROTOCOL_RELATIVE)
        }
        other => panic!("expected a path refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_request_body_is_refused() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(
        &upstream,
        GatewayLimits {
            max_request_body_bytes: 128,
            ..GatewayLimits::default()
        },
    );

    let encoded = base64_encode(&[b'x'; 129]);
    let err = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "POST",
            "path": "/echo",
            "bodyBase64": encoded,
        }))
        .await
        .expect_err("an oversized request body must be refused");
    assert_eq!(err.code(), "request_too_large", "{err}");

    // One byte under the cap still goes through, so the cap is a cap and not a
    // blanket refusal.
    let ok = base64_encode(&[b'x'; 128]);
    let mut stream = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "POST",
            "path": "/echo",
            "bodyBase64": ok,
        }))
        .await
        .expect("a body at the cap is accepted");
    let body: Value = serde_json::from_slice(&drain(&mut stream).await.unwrap()).unwrap();
    assert_eq!(body["bodyLen"], 128);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_response_is_refused_whether_or_not_it_is_advertised() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(
        &upstream,
        GatewayLimits {
            max_response_bytes: 1024,
            ..GatewayLimits::default()
        },
    );

    // Advertised: refused on the head, before a body byte is read.
    let err = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "GET",
            "path": "/advertises-too-much",
        }))
        .await
        .expect_err("a content-length over the cap is refused before the body");
    assert_eq!(err.code(), "response_too_large", "{err}");

    // Unadvertised: refused mid-body, once the running total passes the cap.
    let mut stream = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "GET",
            "path": "/overruns",
        }))
        .await
        .expect("a response with no advertised length starts normally");
    let err = drain(&mut stream)
        .await
        .expect_err("a body that overruns the cap must fail rather than grow");
    assert_eq!(err.code(), "response_too_large", "{err}");
}

/// A redirect is a second hop chosen by whatever answered the first, and nothing
/// validated it. The gateway hands the 302 back instead of acting on it.
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_response_is_not_followed() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());

    let mut stream = gateway
        .fetch(json!({
            "providerId": "local",
            "method": "GET",
            "path": "/redirect",
        }))
        .await
        .expect("the 302 itself is a normal response");
    assert_eq!(
        stream.head().status,
        302,
        "the redirect is reported, not resolved"
    );
    assert_eq!(
        stream.head().headers.get("location").map(String::as_str),
        Some("/v1/redirect-target"),
        "the caller can see where it was being sent"
    );
    let body = drain(&mut stream).await.expect("an empty body");
    assert!(
        !String::from_utf8_lossy(&body).contains("FOLLOWED"),
        "the redirect target's body must never appear here"
    );
    assert_eq!(
        upstream.redirect_target_hits.load(Ordering::SeqCst),
        0,
        "the redirect target must never be requested"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The bridge route
// ─────────────────────────────────────────────────────────────────────────────

struct ScriptedSidecar {
    io: tokio::io::DuplexStream,
    buf: BytesMut,
}

impl ScriptedSidecar {
    async fn send(&mut self, envelope: &Envelope) {
        let bytes = encode_frame(&envelope.to_json_vec().unwrap(), &[]).unwrap();
        self.io.write_all(&bytes).await.unwrap();
        self.io.flush().await.unwrap();
    }

    /// The next frame the host sent: its envelope and its binary tail.
    async fn recv(&mut self) -> Option<(Envelope, Vec<u8>)> {
        loop {
            if let Some((frame, consumed)) = decode_frame(&self.buf).unwrap() {
                let _ = self.buf.split_to(consumed);
                return Some((Envelope::from_json_slice(&frame.json).unwrap(), frame.bin));
            }
            if self.io.read_buf(&mut self.buf).await.unwrap() == 0 {
                return None;
            }
        }
    }
}

async fn connect_bridge(
    gateway: Option<Arc<ProviderGateway>>,
) -> (AssistantBridge, ScriptedSidecar) {
    let registry = Arc::new(ProviderRegistry::new());
    registry.install(gateway);
    connect_bridge_with(registry).await
}

/// The same handshake, against a holder the caller keeps — so a test can install,
/// swap or clear a provider on a bridge that is already connected.
async fn connect_bridge_with(
    registry: Arc<ProviderRegistry>,
) -> (AssistantBridge, ScriptedSidecar) {
    let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
    let mut peer = ScriptedSidecar {
        io: peer_io,
        buf: BytesMut::new(),
    };
    peer.send(&Envelope::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        host_version: "0.0.0-scripted".into(),
        agentkit_contract_version: "scripted".into(),
        pid: 1,
        session_nonce: "scripted".into(),
    }))
    .await;
    let (rx, tx) = tokio::io::split(host_io);
    let bridge = AssistantBridge::connect(
        rx,
        tx,
        BridgeOptions {
            provider_registry: registry,
            ..BridgeOptions::default()
        },
    )
    .await
    .expect("handshake");
    assert!(matches!(peer.recv().await, Some((Envelope::Accept(_), _))));
    (bridge, peer)
}

fn provider_req(id: u64, payload: Value) -> Envelope {
    Envelope::Req(Req {
        id,
        // The claim is irrelevant — the bridge stamps `host` before the table is
        // consulted. It is written as a lie here so that stays visible.
        principal: Principal::Ui,
        verb: "provider.fetch".into(),
        payload,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bridge_streams_a_provider_response_as_chunks_and_one_end() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());
    let (_bridge, mut peer) = connect_bridge(Some(gateway)).await;

    peer.send(&provider_req(
        1,
        json!({"providerId": "local", "method": "GET", "path": "/stream"}),
    ))
    .await;

    match peer.recv().await {
        Some((Envelope::Res(res), _)) => {
            assert_eq!(res.id, 1);
            assert!(res.ok, "{:?}", res.error);
            let head = res.payload.expect("a head payload");
            assert_eq!(head["status"], 200);
            assert_eq!(head["statusText"], "OK");
            assert_eq!(head["headers"]["content-type"], "text/event-stream");
        }
        other => panic!("expected the response head, got {other:?}"),
    }

    let mut body = Vec::new();
    let mut expected_seq = 0u64;
    loop {
        match peer.recv().await {
            Some((Envelope::Chunk(chunk), bin)) => {
                assert_eq!(chunk.id, 1);
                assert_eq!(chunk.seq, expected_seq, "§2: seq increases by one");
                expected_seq += 1;
                body.extend_from_slice(&bin);
            }
            Some((Envelope::End(end), _)) => {
                assert_eq!(end.id, 1);
                assert!(end.ok, "{:?}", end.error);
                break;
            }
            other => panic!("expected a chunk or an end, got {other:?}"),
        }
    }
    assert!(
        expected_seq > 0,
        "the body arrived as chunks, not as a head"
    );
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "data: 0\n\ndata: 1\n\ndata: 2\n\ndata: 3\n\n"
    );
}

/// A gateway failure comes back as a refusing `res`, so the sidecar's
/// `gatewayFetch` sees a reply rather than a hang.
#[tokio::test(flavor = "multi_thread")]
async fn the_bridge_reports_a_gateway_refusal_as_a_failed_res() {
    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());
    let (_bridge, mut peer) = connect_bridge(Some(gateway)).await;

    peer.send(&provider_req(
        1,
        json!({"providerId": "somewhere-else", "method": "GET", "path": "/echo"}),
    ))
    .await;
    match peer.recv().await {
        Some((Envelope::Res(res), _)) => {
            assert!(!res.ok);
            assert_eq!(res.error.expect("an error body").code, "unknown_provider");
        }
        other => panic!("expected a refusing res, got {other:?}"),
    }
}

/// THE cancellation gate. A `cancel` for an in-flight `provider.fetch` must reach
/// the task holding the upstream response and drop it, closing that connection —
/// not merely stop forwarding while the provider keeps generating.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_mid_stream_aborts_the_upstream_call_rather_than_orphaning_it() {
    let mut upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());
    let (_bridge, mut peer) = connect_bridge(Some(gateway)).await;

    peer.send(&provider_req(
        1,
        json!({"providerId": "local", "method": "GET", "path": "/forever"}),
    ))
    .await;
    match peer.recv().await {
        Some((Envelope::Res(res), _)) => assert!(res.ok, "{:?}", res.error),
        other => panic!("expected the response head, got {other:?}"),
    }
    // Wait until the body is actually flowing, so the cancel lands mid-stream.
    match peer.recv().await {
        Some((Envelope::Chunk(chunk), bin)) => {
            assert_eq!(chunk.id, 1);
            assert!(!bin.is_empty());
        }
        other => panic!("expected a body chunk, got {other:?}"),
    }

    peer.send(&Envelope::Cancel(Cancel { id: 1 })).await;

    // The upstream notices. Without the abort this server streams until the test
    // process exits.
    let hung_up = tokio::time::timeout(Duration::from_secs(10), upstream.events.recv()).await;
    assert!(
        matches!(hung_up, Ok(Some(ServerEvent::ClientHungUp))),
        "a cancelled provider.fetch must close the upstream connection, not leave it \
         generating: {hung_up:?}"
    );
}

/// Requirement: configuration is trusted-path only.
///
/// There is no verb by which anything arriving over the bridge can add, modify or
/// select a provider outside the registry — the sidecar→host table has exactly
/// one entry, and that entry NAMES a provider rather than defining one. This test
/// is the assertion that the table has not grown one.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_on_the_bridge_can_add_or_select_a_provider_outside_the_registry() {
    // 1. The verb table itself: one row, host-only.
    let rows: Vec<(&str, usize)> = SIDECAR_TO_HOST_VERBS
        .entries()
        .iter()
        .map(|entry| (entry.verb, entry.principals.len()))
        .collect();
    assert_eq!(
        rows,
        vec![("provider.fetch", 1)],
        "a second sidecar→host verb is a work-package-sized authorization decision, \
         not a new table row"
    );

    let upstream = start_upstream().await;
    let gateway = gateway_for(&upstream, GatewayLimits::default());
    let (_bridge, mut peer) = connect_bridge(Some(gateway)).await;

    // 2. A verb that would configure one does not exist and is not invented.
    for verb in [
        "provider.register",
        "provider.configure",
        "provider.list",
        "settings.write",
    ] {
        peer.send(&Envelope::Req(Req {
            id: 1,
            principal: Principal::Host,
            verb: verb.into(),
            payload: json!({"id": "evil", "baseUrl": "http://evil.example/v1"}),
        }))
        .await;
        match peer.recv().await {
            Some((Envelope::Res(res), _)) => {
                assert!(!res.ok, "{verb} must not be served");
                assert_eq!(
                    res.error.expect("an error body").code,
                    "unknown_verb",
                    "{verb}"
                );
            }
            other => panic!("expected a refusing res for {verb}, got {other:?}"),
        }
    }

    // 3. The registry is unchanged and still holds exactly what Rust put in it.
    assert_eq!(
        gateway_for(&upstream, GatewayLimits::default()).provider_ids(),
        vec!["local"]
    );
}

/// A bridge with no gateway says so rather than pretending: `unimplemented` is a
/// refusal the sidecar can see, and it is what a build with no configured local
/// model should answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_with_no_gateway_answers_provider_fetch_unimplemented() {
    let (_bridge, mut peer) = connect_bridge(None).await;
    peer.send(&provider_req(
        1,
        json!({"providerId": "local", "method": "GET", "path": "/echo"}),
    ))
    .await;
    match peer.recv().await {
        Some((Envelope::Res(res), _)) => {
            assert!(!res.ok);
            assert_eq!(res.error.expect("an error body").code, "unimplemented");
        }
        other => panic!("expected a refusing res, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The trusted settings path (ADR-0017): AssistantSlot → registry → live bridge
// ─────────────────────────────────────────────────────────────────────────────

/// THE wiring gate. Production used to answer every `provider.fetch` with
/// `unimplemented` because nothing populated the registry; this is the path that
/// populates it, and it has to reach a bridge that is ALREADY connected.
///
/// One connection, four states: nothing configured, a provider installed, the
/// endpoint changed to a different one, and the provider cleared. The sidecar is
/// never restarted in between — a user editing their endpoint must not have to
/// restart the app to be heard.
#[tokio::test(flavor = "multi_thread")]
async fn configuring_a_provider_arms_swaps_and_clears_an_already_connected_bridge() {
    let first = start_upstream().await;
    let second = start_upstream().await;
    let slot = AssistantSlot::new();
    let (_bridge, mut peer) = connect_bridge_with(slot.provider_registry()).await;

    // 1. Nothing configured: the route exists but has no provider to serve.
    peer.send(&provider_req(1, echo_payload())).await;
    assert_eq!(refusal_code(&mut peer).await, "unimplemented");

    // 2. The trusted command installs one. No restart, no new bridge.
    slot.configure_provider(Some(ProviderConfig {
        id: "local".into(),
        base_url: first.base_url(),
        model: "test-model".into(),
        api_key: None,
    }))
    .expect("a loopback provider is installable");

    let body = ok_body(&mut peer, 3, echo_payload()).await;
    assert_eq!(body["target"], "/v1/echo");
    assert_eq!(
        body["headers"]["host"],
        format!("127.0.0.1:{}", first.port),
        "the request reached the provider that was just installed"
    );

    // 3. The user edits their endpoint: the next request goes to the new one.
    slot.configure_provider(Some(ProviderConfig {
        id: "local".into(),
        base_url: second.base_url(),
        model: "test-model".into(),
        api_key: None,
    }))
    .expect("a second loopback provider is installable");

    let body = ok_body(&mut peer, 5, echo_payload()).await;
    assert_eq!(
        body["headers"]["host"],
        format!("127.0.0.1:{}", second.port),
        "the registry is read per request, so the swap took effect on the next one"
    );

    // 4. The user turns the assistant off: the route has nothing to serve again.
    slot.configure_provider(None).expect("clearing never fails");
    peer.send(&provider_req(7, echo_payload())).await;
    assert_eq!(refusal_code(&mut peer).await, "unimplemented");
}

/// A rejected endpoint must be REPORTED, and must not disarm the one that works.
///
/// Silently dropping the refused entry would leave the user with a provider that
/// answers "unknown provider" on every later call — a message that names the
/// wrong fault and hides the typo that caused it.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_reconfiguration_reports_the_reason_and_leaves_the_working_provider_installed() {
    let upstream = start_upstream().await;
    let slot = AssistantSlot::new();
    let (_bridge, mut peer) = connect_bridge_with(slot.provider_registry()).await;

    slot.configure_provider(Some(ProviderConfig {
        id: "local".into(),
        base_url: upstream.base_url(),
        model: "test-model".into(),
        api_key: None,
    }))
    .expect("a loopback provider is installable");

    for (base, why) in [
        ("http://api.example.com/v1", "a non-loopback host"),
        (
            "http://127.0.0.1@evil.com/v1",
            "userinfo hiding the real host",
        ),
        ("not a url", "text that is not a URL at all"),
    ] {
        let err = slot
            .configure_provider(Some(ProviderConfig {
                id: "local".into(),
                base_url: base.into(),
                model: "test-model".into(),
                api_key: None,
            }))
            .expect_err(why);
        assert!(
            !err.to_string().is_empty(),
            "{base}: every refusal names its reason"
        );
    }

    // The provider that was working still is, on the same bridge.
    let body = ok_body(&mut peer, 1, echo_payload()).await;
    assert_eq!(
        body["headers"]["host"],
        format!("127.0.0.1:{}", upstream.port),
        "a refused reconfigure must not disarm the provider that answers"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Local helpers
// ─────────────────────────────────────────────────────────────────────────────
/// The `/echo` operation, the cheapest round trip the fixture serves.
fn echo_payload() -> Value {
    json!({"providerId": "local", "method": "GET", "path": "/echo"})
}

/// Reads one refusing `res` and returns its code.
async fn refusal_code(peer: &mut ScriptedSidecar) -> String {
    match peer.recv().await {
        Some((Envelope::Res(res), _)) => {
            assert!(!res.ok, "expected a refusal, got {:?}", res.payload);
            res.error.expect("an error body").code
        }
        other => panic!("expected a refusing res, got {other:?}"),
    }
}

/// Sends one `provider.fetch` and reads its head, body chunks and `end` back as
/// the JSON the fixture echoed.
async fn ok_body(peer: &mut ScriptedSidecar, id: u64, payload: Value) -> Value {
    peer.send(&provider_req(id, payload)).await;
    match peer.recv().await {
        Some((Envelope::Res(res), _)) => {
            assert!(res.ok, "{:?}", res.error);
            assert_eq!(res.payload.expect("a head payload")["status"], 200);
        }
        other => panic!("expected the response head, got {other:?}"),
    }
    let mut body = Vec::new();
    loop {
        match peer.recv().await {
            Some((Envelope::Chunk(chunk), bin)) => {
                assert_eq!(chunk.id, id);
                body.extend_from_slice(&bin);
            }
            Some((Envelope::End(end), _)) => {
                assert!(end.ok, "{:?}", end.error);
                break;
            }
            other => panic!("expected a chunk or an end, got {other:?}"),
        }
    }
    serde_json::from_slice(&body).expect("the fixture answers JSON")
}

/// Standard base64, matching what `createGatewayFetch` writes into `bodyBase64`.
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
