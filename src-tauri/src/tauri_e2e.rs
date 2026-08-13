use std::fs::File;
use std::io::Read;

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
