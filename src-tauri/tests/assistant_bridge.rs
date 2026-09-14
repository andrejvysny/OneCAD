//! The assistant bridge against the REAL compiled sidecar, plus the refusal
//! paths a real sidecar cannot be made to exercise.
//!
//! Two lanes live here on purpose:
//!
//! * **Real-child cases** — handshake, a bounded stream round trip, cancellation,
//!   crash-then-reconnect. These need
//!   `src-tauri/binaries/onecad-assistant-host-<triple>`, built by
//!   `bash scripts/build-assistant-host.sh`. Absent, they SKIP — unless
//!   `ONECAD_REQUIRE_ASSISTANT_HOST=1`, which turns a missing binary into a hard
//!   failure so CI can never be vacuously green (the same discipline as
//!   `ONECAD_REQUIRE_WORKER=1`).
//! * **Scripted-peer cases** — an incompatible `hello` version, and a sidecar
//!   `req` that claims `principal: "ui"`. The real host always sends a valid
//!   `hello` and never forges a principal, so the only honest way to assert the
//!   refusals is a peer this test writes byte for byte. They always run.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use onecad_assistant_protocol::{
    decode_frame, encode_frame, Envelope, Hello, Ping, Principal, Req, PROTOCOL_VERSION,
};
use onecad_lib::assistant::{
    resolve_assistant_host_path, AssistantBridge, AssistantHost, AssistantState, BridgeError,
    BridgeOptions, HostConfig, StreamEvent,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors the house `real_worker()` pattern)
// ─────────────────────────────────────────────────────────────────────────────

fn real_host() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_ASSISTANT_HOST_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_ASSISTANT_HOST_PATH={p:?} is set but no assistant host binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_assistant_host_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_ASSISTANT_HOST").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_ASSISTANT_HOST=1 but no assistant host binary resolved \
         (build it with scripts/build-assistant-host.sh; CI must hard-fail here)"
    );
    None
}

/// A config with fast supervision timings and a scratch data directory.
///
/// `--app-data-dir` is a tempdir per test: the child opens a SQLite store under
/// it, and sharing one across tests would make them fight over the same file.
fn config(binary: PathBuf, data: &tempfile::TempDir) -> HostConfig {
    HostConfig {
        handshake_timeout: Duration::from_secs(20),
        ping_interval: Duration::from_millis(200),
        ping_timeout: Duration::from_secs(2),
        backoff: vec![Duration::from_millis(50)],
        healthy_threshold: Duration::from_millis(100),
        ..HostConfig::production(binary, data.path().to_path_buf())
    }
}

async fn ready_host(binary: PathBuf, data: &tempfile::TempDir) -> AssistantHost {
    let host = AssistantHost::spawn(
        config(binary, data),
        std::sync::Arc::new(onecad_lib::assistant::ProviderRegistry::new()),
    );
    assert!(
        host.wait_ready(Duration::from_secs(30)).await,
        "the real assistant host must spawn and complete the OCAK1 handshake (state {}, error {:?})",
        host.state().as_str(),
        host.last_error()
    );
    host
}

/// The `getVersion` REST operation — the cheapest real AgentKit call there is.
fn get_version_payload() -> Value {
    json!({
        "method": "GET",
        "path": "/v1/version",
        "headers": {"accept": "application/json"},
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Real-child cases
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn real_host_completes_the_handshake() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let host = ready_host(binary, &data).await;

    let hello = host.hello().expect("hello after ready");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    assert!(hello.pid > 0, "the child reports its pid");
    assert!(!hello.host_version.is_empty());
    assert!(!hello.session_nonce.is_empty(), "each launch gets a nonce");
    assert!(!hello.agentkit_contract_version.is_empty());

    host.retire();
    assert!(
        host.wait_torn_down(Duration::from_secs(10)).await,
        "retirement must actually reap the child"
    );
    assert_eq!(host.state(), AssistantState::Retired);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_agentkit_fetch_streams_its_body_and_ends() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let host = ready_host(binary, &data).await;
    let bridge = host.bridge().expect("a live bridge");

    let mut stream = bridge
        .request_stream("agentkit.fetch", Principal::Ui, get_version_payload())
        .await
        .expect("agentkit.fetch");
    assert_eq!(stream.head["status"], 200, "head: {}", stream.head);

    let mut body = Vec::new();
    loop {
        match stream.next().await {
            Some(StreamEvent::Chunk(bytes)) => body.extend_from_slice(&bytes),
            Some(StreamEvent::End(error)) => {
                assert!(error.is_none(), "the stream must end cleanly: {error:?}");
                break;
            }
            None => panic!("the stream ended without a terminator"),
        }
    }
    let text = String::from_utf8(body).expect("a UTF-8 JSON body");
    let parsed: Value = serde_json::from_str(&text).expect("JSON");
    assert!(
        parsed.get("restApiVersion").is_some(),
        "AgentKit's getVersion payload: {text}"
    );

    host.retire();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// Dropping a stream sends `cancel` (§2) and leaves the connection usable: a
/// cancellation is a request ending, never a bridge ending.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_stream_leaves_the_bridge_healthy() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let host = ready_host(binary, &data).await;
    let bridge = host.bridge().expect("a live bridge");

    let stream = bridge
        .request_stream("agentkit.fetch", Principal::Ui, get_version_payload())
        .await
        .expect("agentkit.fetch");
    drop(stream); // → `cancel` on the wire

    // The next call on the same connection still works, and the child is alive.
    let mut again = bridge
        .request_stream("agentkit.fetch", Principal::Ui, get_version_payload())
        .await
        .expect("a cancelled stream must not poison the connection");
    assert_eq!(again.head["status"], 200);
    while let Some(event) = again.next().await {
        if matches!(event, StreamEvent::End(_)) {
            break;
        }
    }
    assert!(!bridge.is_closed());
    bridge.ping().await.expect("the child still answers pings");

    host.retire();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// Kill the child out from under the supervisor: it must reconnect on its own,
/// with a fresh process, and serve requests again.
#[tokio::test(flavor = "multi_thread")]
async fn a_killed_child_is_restarted_and_serves_again() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let host = ready_host(binary, &data).await;
    let first_pid = host.hello().expect("hello").pid;

    // SIGKILL is the crash this supervisor exists for: no `shutdown`, no clean
    // exit, just a dead pipe.
    #[cfg(unix)]
    unsafe {
        libc::kill(first_pid as i32, libc::SIGKILL);
    }

    // The supervisor notices, backs off, and comes back with a NEW process.
    let mut reconnected = false;
    for _ in 0..600 {
        if host.state() == AssistantState::Ready {
            if let Some(hello) = host.hello() {
                if hello.pid != first_pid {
                    reconnected = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        reconnected,
        "the supervisor must respawn the killed child (state {}, error {:?})",
        host.state().as_str(),
        host.last_error()
    );

    let bridge = host.bridge().expect("a live bridge after the restart");
    let mut stream = bridge
        .request_stream("agentkit.fetch", Principal::Ui, get_version_payload())
        .await
        .expect("the restarted child serves requests");
    assert_eq!(stream.head["status"], 200);
    while let Some(event) = stream.next().await {
        if matches!(event, StreamEvent::End(_)) {
            break;
        }
    }

    host.retire();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scripted-peer cases (no binary required)
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

    async fn recv(&mut self) -> Option<Envelope> {
        loop {
            if let Some((frame, consumed)) = decode_frame(&self.buf).unwrap() {
                let _ = self.buf.split_to(consumed);
                return Some(Envelope::from_json_slice(&frame.json).unwrap());
            }
            if self.io.read_buf(&mut self.buf).await.unwrap() == 0 {
                return None;
            }
        }
    }
}

fn scripted() -> (tokio::io::DuplexStream, ScriptedSidecar) {
    let (host_io, peer_io) = tokio::io::duplex(64 * 1024);
    (
        host_io,
        ScriptedSidecar {
            io: peer_io,
            buf: BytesMut::new(),
        },
    )
}

fn hello(version: u32) -> Envelope {
    Envelope::Hello(Hello {
        protocol_version: version,
        host_version: "0.0.0-scripted".into(),
        agentkit_contract_version: "scripted".into(),
        pid: 1,
        session_nonce: "scripted".into(),
    })
}

#[tokio::test]
async fn an_incompatible_protocol_version_is_rejected_before_anything_else() {
    let (host_io, mut peer) = scripted();
    peer.send(&hello(PROTOCOL_VERSION + 1)).await;
    let (rx, tx) = tokio::io::split(host_io);

    let err = AssistantBridge::connect(rx, tx, BridgeOptions::default())
        .await
        .expect_err("an incompatible version must never be accepted");
    assert!(matches!(err, BridgeError::Handshake(_)), "{err}");

    match peer.recv().await {
        Some(Envelope::Reject(reject)) => assert!(
            reject.reason.contains("unsupported protocol version"),
            "{reject:?}"
        ),
        other => panic!("expected reject, got {other:?}"),
    }
    assert!(
        peer.recv().await.is_none(),
        "a rejected peer gets no further frames"
    );
}

/// THE authority test. A sidecar `req` that claims `principal: "ui"` is stamped
/// `host` and then refused, because `agentkit.fetch` is not a verb the sidecar
/// may call in this direction. A claim in a payload cannot create authority.
#[tokio::test]
async fn a_sidecar_request_claiming_the_ui_principal_is_refused() {
    let (host_io, mut peer) = scripted();
    peer.send(&hello(PROTOCOL_VERSION)).await;
    let (rx, tx) = tokio::io::split(host_io);
    let bridge = Arc::new(
        AssistantBridge::connect(rx, tx, BridgeOptions::default())
            .await
            .expect("handshake"),
    );
    assert!(matches!(peer.recv().await, Some(Envelope::Accept(_))));

    peer.send(&Envelope::Req(Req {
        id: 1, // odd — a legal sidecar id
        principal: Principal::Ui,
        verb: "agentkit.fetch".into(),
        payload: json!({"method": "GET", "path": "/v1/version"}),
    }))
    .await;

    match peer.recv().await {
        Some(Envelope::Res(res)) => {
            assert_eq!(res.id, 1);
            assert!(
                !res.ok,
                "a ui-only verb must never be served to the sidecar"
            );
            assert_eq!(res.error.expect("an error body").code, "unknown_verb");
        }
        other => panic!("expected a refusing res, got {other:?}"),
    }

    // The refusal is not a teardown: the bridge keeps serving.
    peer.send(&Envelope::Ping(Ping { id: 3 })).await;
    assert!(matches!(peer.recv().await, Some(Envelope::Pong(_))));
    assert!(!bridge.is_closed());
}
