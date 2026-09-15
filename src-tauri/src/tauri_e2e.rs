use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::state::AppState;
use crate::worker::resolve_worker_path;

const EMBEDDED_MANIFEST: &str =
    include_str!(concat!(env!("OUT_DIR"), "/onecad-worker-manifest.json"));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompositionStatus {
    runtime: &'static str,
    worker_path: String,
    binary_sha256: String,
    manifest: serde_json::Value,
    hello: onecad_protocol::messages::HelloResult,
    snapshot_id: u64,
}

#[tauri::command]
pub(crate) async fn composition_status(
    state: State<'_, AppState>,
) -> Result<CompositionStatus, String> {
    let path = resolve_worker_path().ok_or("bundled worker did not resolve")?;
    let manifest = serde_json::from_str(EMBEDDED_MANIFEST)
        .map_err(|error| format!("embedded worker manifest is invalid: {error}"))?;
    let hello = state
        .composition_worker_hello()
        .ok_or("worker handshake is not ready")?;
    let snapshot_id = {
        let guard = state.runtime.lock().await;
        let runtime = guard.as_ref().ok_or("no open document")?;
        let snapshots = runtime.subscribe_snapshots();
        let head_id = snapshots.borrow().as_ref().map_or(0, |head| head.id.0);
        head_id
    };

    Ok(CompositionStatus {
        runtime: "tauri",
        worker_path: path.to_string_lossy().into_owned(),
        binary_sha256: sha256(&path)?,
        manifest,
        hello,
        snapshot_id,
    })
}

/// Identity handshake for the `tauri-agent` MCP harness.
///
/// The agent drives a real desktop app with native input, so before it sends anything it
/// must prove the process serving WebDriver is the app from ITS checkout. `cargo_manifest_dir`
/// is the load-bearing field: `env!` bakes it in at COMPILE time, so it proves which source
/// tree produced this binary — something no runtime path inspection can establish.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentIdentity {
    runtime: &'static str,
    agent_testing: bool,
    bundle_id: String,
    pid: u32,
    executable: String,
    cargo_manifest_dir: &'static str,
    session_nonce: String,
}

/// One nonce per process lifetime: two readings that differ mean the app restarted.
static SESSION_NONCE: OnceLock<String> = OnceLock::new();

/// Deliberately cheap — no worker hashing and no `state.runtime` lock, unlike
/// [`composition_status`]. The agent calls this on every connect and reconnect, including
/// while a regen holds the runtime, so it must never be able to block.
#[tauri::command]
pub(crate) fn agent_identity(app: tauri::AppHandle) -> Result<AgentIdentity, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve current executable: {error}"))?
        .to_string_lossy()
        .into_owned();
    let session_nonce = SESSION_NONCE
        .get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
        .clone();

    Ok(AgentIdentity {
        runtime: "tauri",
        agent_testing: true,
        bundle_id: app.config().identifier.clone(),
        pid: std::process::id(),
        executable,
        cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
        session_nonce,
    })
}

/// Backend-side idle state for the `tauri-agent` harness.
///
/// DOM quietness is not "the CAD transaction finished". A click on Extrude runs frontend →
/// Rust → the OCCT worker → mesh generation → a Three.js upload, and an orbit changes camera
/// matrices and repaints WebGL with no DOM mutation at all. The harness therefore settles on
/// observable backend and renderer state, and this is the backend half of it.
///
/// Deliberately cheap: no worker hashing, and it never waits on the document runtime. The
/// runtime mutex is shared by every in-flight async command (see the hazard note above
/// `apply_operation`), so a probe that blocked on it could stall the very regen it is waiting
/// for. When it is busy, `snapshot_id` comes back `null` — an unknown answer the caller can
/// retry, never a lie and never a deadlock.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentStatus {
    snapshot_id: Option<u64>,
    /// `None` in a release build, where the render-expectation ledger is not compiled in.
    pending_render_expectations: Option<usize>,
    pending_bodies: Option<Vec<String>>,
}

#[tauri::command]
pub(crate) async fn agent_status(state: State<'_, AppState>) -> Result<AgentStatus, String> {
    // `try_lock` on purpose — see the doc comment. A contended runtime means "busy", which is
    // itself the answer the idle probe needs, so it must not be turned into a wait.
    let snapshot_id = match state.runtime.try_lock() {
        Ok(guard) => guard.as_ref().map(|runtime| {
            let snapshots = runtime.subscribe_snapshots();
            let head = snapshots.borrow();
            head.as_ref().map_or(0, |snapshot| snapshot.id.0)
        }),
        Err(_) => None,
    };

    #[cfg(debug_assertions)]
    let (pending_render_expectations, pending_bodies) = {
        let (count, bodies) = crate::api::pending_render_expectations();
        (Some(count), Some(bodies))
    };
    #[cfg(not(debug_assertions))]
    let (pending_render_expectations, pending_bodies) = (None, None);

    Ok(AgentStatus {
        snapshot_id,
        pending_render_expectations,
        pending_bodies,
    })
}

fn sha256(path: &std::path::Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open resolved worker {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| format!("read resolved worker {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
