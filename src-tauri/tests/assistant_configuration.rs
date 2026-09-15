//! The authoritative provider configuration, against the REAL compiled sidecar
//! (`docs/assistant/wire-protocol.md` §5, "Host → sidecar, configuration").
//!
//! The defect this lane exists for: Rust registered a provider in its own
//! gateway and told the sidecar nothing, so AgentKit — which resolves a turn's
//! provider from ITS OWN store — reached `no_provider` on the first message of
//! every fresh install. Rust is the configuration authority; the sidecar's
//! provider row is a projection of it, installed over the bridge, and re-sent on
//! every reconnect because a new child starts with an acknowledgement of
//! nothing.
//!
//! These need `src-tauri/binaries/onecad-assistant-host-<triple>`, built by
//! `bash scripts/build-assistant-host.sh`. Absent, they SKIP — unless
//! `ONECAD_REQUIRE_ASSISTANT_HOST=1`, which turns a missing binary into a hard
//! failure so CI can never be vacuously green.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use onecad_assistant_protocol::Principal;
use onecad_lib::assistant::supervisor::{ProjectedCapabilities, ProjectedProvider};
use onecad_lib::assistant::{
    resolve_assistant_host_path, AssistantHost, AssistantSlot, AssistantState, HostConfig,
    ProviderConfig, StreamEvent,
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

/// The production surface: a slot, configured the way `lib.rs` configures it.
fn slot(binary: PathBuf, data: &tempfile::TempDir) -> AssistantSlot {
    let slot = AssistantSlot::new();
    slot.configure(config(binary, data));
    slot
}

/// The provider the settings command would install — nothing needs to be
/// LISTENING at the base for this lane: registration is syntactic (ADR-0017's
/// canonical-loopback validation), and no model call is made here.
fn provider_config(model: &str) -> ProviderConfig {
    ProviderConfig {
        id: "local".into(),
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: model.into(),
        api_key: None,
    }
}

fn projected(model: &str) -> ProjectedProvider {
    ProjectedProvider {
        id: "local".into(),
        model: model.into(),
        capabilities: ProjectedCapabilities {
            streaming: true,
            tool_calling: true,
            model_list: false,
        },
    }
}

async fn ready(host: &AssistantHost) {
    assert!(
        host.wait_ready(Duration::from_secs(30)).await,
        "the real assistant host must spawn and complete the OCAK1 handshake (state {}, error {:?})",
        host.state().as_str(),
        host.last_error()
    );
}

/// One AgentKit REST call over the bridge, collected whole.
async fn rest(host: &AssistantHost, path: &str) -> Value {
    let bridge = host.bridge().expect("a live bridge");
    let mut stream = bridge
        .request_stream(
            "agentkit.fetch",
            Principal::Ui,
            json!({"method": "GET", "path": path, "headers": {"accept": "application/json"}}),
        )
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
    serde_json::from_slice(&body).expect("a JSON body")
}

/// The provider ids the SIDECAR holds — the projection, read back from the
/// child's own store rather than from anything Rust remembers about it.
async fn sidecar_providers(host: &AssistantHost) -> Vec<(String, String, bool)> {
    let value = rest(host, "/v1/providers").await;
    value
        .as_array()
        .expect("a provider array")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().unwrap_or_default().to_string(),
                entry["defaultModel"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                entry["enabled"].as_bool().unwrap_or_default(),
            )
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Cases
// ─────────────────────────────────────────────────────────────────────────────

/// The execution-ready barrier: a host is not published as `Ready` until the
/// projection it had at connect time was installed and acknowledged. `Ready` is
/// what every bridge caller waits on, so a turn cannot be submitted to a child
/// that does not yet know which provider to run it against.
#[tokio::test(flavor = "multi_thread")]
async fn the_projection_is_acknowledged_before_the_host_is_ready() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let slot = slot(binary, &data);

    slot.configure_provider(Some(provider_config("qwen3:8b")))
        .expect("a canonical loopback base is accepted");
    let generation = slot.project_provider(Some(projected("qwen3:8b")));
    assert_eq!(generation, 1, "the first edit of an app run mints 1");

    let host = slot.start().expect("a configured slot starts a host");
    ready(&host).await;

    assert_eq!(
        host.acknowledged_generation(),
        Some(generation),
        "Ready must not be published before the child acknowledged the projection"
    );
    assert_eq!(
        sidecar_providers(&host).await,
        vec![("local".to_string(), "qwen3:8b".to_string(), true)],
        "the child's own store holds the projection, which is what makes a turn possible"
    );

    slot.stop();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// A host nobody configured a provider for acknowledges nothing — and still
/// answers administrative requests. That is the waiting-for-configuration state:
/// visible, and not a failure.
#[tokio::test(flavor = "multi_thread")]
async fn an_unconfigured_host_acknowledges_nothing_and_still_serves_administrative_calls() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let slot = slot(binary, &data);

    let host = slot.start().expect("start");
    ready(&host).await;

    assert_eq!(host.acknowledged_generation(), None);
    assert!(
        rest(&host, "/v1/version")
            .await
            .get("restApiVersion")
            .is_some(),
        "an unconfigured host still answers administrative requests"
    );
    assert!(
        sidecar_providers(&host).await.is_empty(),
        "nothing is seeded: the sidecar knows only what the host told it"
    );

    slot.stop();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// A settings edit while the child is up reaches it without a restart.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_reaches_a_running_child_and_the_generation_advances() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let slot = slot(binary, &data);

    slot.configure_provider(Some(provider_config("qwen3:8b")))
        .expect("accepted");
    let first = slot.project_provider(Some(projected("qwen3:8b")));
    let host = slot.start().expect("start");
    ready(&host).await;
    assert_eq!(host.acknowledged_generation(), Some(first));

    slot.configure_provider(Some(provider_config("llama3.2:3b")))
        .expect("accepted");
    let second = slot.project_provider(Some(projected("llama3.2:3b")));
    assert!(second > first, "generations are monotonic");
    assert!(
        host.wait_acknowledged(second, Duration::from_secs(10))
            .await,
        "an edit must reach a RUNNING child (acknowledged {:?})",
        host.acknowledged_generation()
    );
    assert_eq!(
        sidecar_providers(&host).await,
        vec![("local".to_string(), "llama3.2:3b".to_string(), true)]
    );

    slot.stop();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// A new child has acknowledged nothing, so the projection is sent again. The
/// store file survives a restart, but a row in it is not an acknowledgement.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_projection_is_reinstalled_after_a_crash() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let slot = slot(binary, &data);

    slot.configure_provider(Some(provider_config("qwen3:8b")))
        .expect("accepted");
    let generation = slot.project_provider(Some(projected("qwen3:8b")));
    let host = slot.start().expect("start");
    ready(&host).await;
    let first_pid = host.hello().expect("hello").pid;
    assert_eq!(host.acknowledged_generation(), Some(generation));

    // SIGKILL: no `shutdown`, no clean exit, just a dead pipe.
    unsafe {
        libc::kill(first_pid as i32, libc::SIGKILL);
    }

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

    assert_eq!(
        host.acknowledged_generation(),
        Some(generation),
        "the new child must have acknowledged the projection before Ready"
    );
    assert_eq!(
        sidecar_providers(&host).await,
        vec![("local".to_string(), "qwen3:8b".to_string(), true)],
        "the restarted child holds the projection it was re-sent"
    );

    slot.stop();
    assert!(host.wait_torn_down(Duration::from_secs(10)).await);
}

/// Retiring and starting again mints no generation and loses none: the
/// projection outlives any one host, because the configuration is the app's.
#[tokio::test(flavor = "multi_thread")]
async fn a_replacement_host_installs_the_same_projection() {
    let Some(binary) = real_host() else {
        eprintln!("SKIP: no assistant host binary");
        return;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let slot = slot(binary, &data);

    slot.configure_provider(Some(provider_config("qwen3:8b")))
        .expect("accepted");
    let generation = slot.project_provider(Some(projected("qwen3:8b")));

    let first = slot.start().expect("start");
    ready(&first).await;
    slot.stop();
    assert!(first.wait_torn_down(Duration::from_secs(10)).await);

    let second = slot.start().expect("a stopped slot starts a new host");
    ready(&second).await;
    assert_eq!(second.acknowledged_generation(), Some(generation));
    assert_eq!(
        slot.projection().generation(),
        generation,
        "no mint on restart"
    );

    slot.stop();
    assert!(second.wait_torn_down(Duration::from_secs(10)).await);
}
