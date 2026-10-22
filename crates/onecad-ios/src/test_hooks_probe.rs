//! Acceptance probes that read or persist the active document through the same
//! `onecad-app` services the editor's document actor calls (`test-hooks`
//! only). Every artifact written here embeds the run id and the kernel
//! identity, or carries them in a `<file>.provenance.json` sidecar, so the
//! simulator gate can reject stale or fake-backend evidence.
use onecad_app::{api, dto::DocumentProjection, worker::WorkerBackend};
use onecad_core::{document::body::BodyHealth, ids::BodyId};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use onecad_ui::host::DocumentAccess;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The kernel this process runs: the identity manifest the binary verifies
/// every in-process run against (`embedded`), the one of the bundle actually
/// linked (`linked`), and the live document worker's hello.
pub fn kernel_identity(access: Option<&DocumentAccess>) -> Value {
    let parse = |text: Option<&str>| text.and_then(|text| serde_json::from_str::<Value>(text).ok());
    let embedded = parse(onecad_app::worker::embedded_identity_json());
    let linked = parse(onecad_app::worker::linked_identity_json());
    let backend = access.map(|access| {
        if matches!(
            access.state.services().backend(),
            WorkerBackend::Embedded(_)
        ) {
            "in-process"
        } else {
            "other"
        }
    });
    let worker = access.and_then(|access| access.state.live_worker());
    json!({
        "backend": backend,
        "embedded": embedded,
        "linked": linked,
        "hello": worker.as_ref().and_then(|worker| worker.hello()),
        "epoch": worker.as_ref().map(|worker| worker.epoch().0),
        "workerState": worker.as_ref().map(|worker| format!("{:?}", worker.state())),
        "journalWriteFailures": worker.as_ref().map(|worker| worker.journal_write_failures()),
        "relaunchRequired": onecad_app::worker::in_process_relaunch_required(),
    })
}

/// `Documents/acceptance/<run id>/`, created on demand.
pub fn run_directory(access: &DocumentAccess, run_id: &str) -> Result<PathBuf, String> {
    let directory = access.documents.join("acceptance").join(run_id);
    std::fs::create_dir_all(&directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    Ok(directory)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Writes `<file>.provenance.json` next to `file`: run id, kernel identity and
/// the file's SHA-256 (a `.onecad` or STEP file cannot carry them inside).
pub fn write_provenance(
    file: &Path,
    kind: &str,
    run_id: &str,
    access: &DocumentAccess,
) -> Result<Value, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let record = json!({
        "runId": run_id,
        "kind": kind,
        "artifact": name,
        "sha256": sha256(&bytes),
        "bytes": bytes.len(),
        "kernelIdentity": kernel_identity(Some(access)),
    });
    let sidecar = file.with_file_name(format!("{name}.provenance.json"));
    write_json(&sidecar, &record)?;
    Ok(record)
}

pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Path, dirty flag, revision, history, bodies (exact mass properties and
/// tessellation bounds) and sketch constraint counts of the active document.
pub async fn document_summary(access: &DocumentAccess, tabs: usize) -> Result<Value, String> {
    let projection = api::get_projection(&access.state)
        .await
        .map_err(|e| e.to_string())?;
    let (document, sketches) = runtime_facts(access, &projection).await?;
    let mut bodies = Vec::new();
    for (id, body) in &projection.bodies {
        bodies.push(body_facts(access, &projection, id, body.health).await?);
    }
    Ok(json!({
        "tabCount": tabs,
        "document": document,
        "features": projection.features.iter().map(|feature| json!({
            "id": feature.id,
            "opType": feature.op_type,
            "label": feature.label,
            "valueText": feature.value_text,
            "primaryValue": feature.primary_value,
            "primaryValueKind": feature.primary_value_kind,
            "status": feature.status,
            "suppressed": feature.suppressed,
        })).collect::<Vec<_>>(),
        "bodies": bodies,
        "sketches": sketches,
    }))
}

async fn runtime_facts(
    access: &DocumentAccess,
    projection: &DocumentProjection,
) -> Result<(Value, Vec<Value>), String> {
    let guard = access.state.runtime.lock().await;
    let runtime = guard.as_ref().ok_or("no document is open")?;
    let document = json!({
        "path": runtime.path(),
        "title": projection.title,
        "documentId": projection.document_id,
        "dirty": runtime.is_dirty(),
        "revision": runtime.revision().0,
        "appliedOps": projection.applied_ops,
        "totalOps": projection.total_ops,
        "undoDepth": projection.undo_depth,
        "redoDepth": projection.redo_depth,
        "geometrySource": projection.geometry_source,
    });
    let mut sketches = Vec::new();
    for dto in projection.sketches.values() {
        let id = dto
            .id
            .parse()
            .map_err(|e| format!("sketch id {}: {e}", dto.id))?;
        let sketch = runtime
            .sketch_snapshot(id, "acceptance summary")
            .map_err(|e| e.to_string())?;
        sketches.push(json!({
            "id": dto.id,
            "name": dto.name,
            "status": dto.status,
            "dof": dto.dof,
            "entities": sketch.entities().len(),
            "constraints": sketch.constraints().len(),
        }));
    }
    Ok((document, sketches))
}

async fn body_facts(
    access: &DocumentAccess,
    projection: &DocumentProjection,
    id: &str,
    health: BodyHealth,
) -> Result<Value, String> {
    let body: BodyId = id.parse().map_err(|e| format!("body id {id}: {e}"))?;
    let wire = onecad_app::worker::wire::body_id_wire(body);
    let topology = api::query_body_topology(&access.state, wire.clone())
        .await
        .map_err(|e| e.to_string())?;
    let mass = api::query_mass_properties(&access.state, wire, None)
        .await
        .map_err(|e| e.to_string())?;
    let session = Some(projection.runtime_session.clone());
    let blob = api::get_mesh(&access.state, id.to_string(), "fine".into(), None, session)
        .await
        .map_err(|e| e.to_string())?;
    let (min, max) = mesh_bounds(&blob)?;
    Ok(json!({
        "id": id,
        "healthy": health == BodyHealth::Healthy,
        "solids": topology.solid_count,
        "faces": topology.face_count,
        "volume": mass.volume,
        "surfaceArea": mass.surface_area,
        "centroid": mass.centroid,
        "min": min,
        "max": max,
    }))
}

/// Vertex bounds of the checked MESH1 decode (OCW1 has no exact BRep box
/// query; on planar extreme faces these are the BRep vertices, in f32).
fn mesh_bounds(blob: &[u8]) -> Result<([f64; 3], [f64; 3]), String> {
    let mesh = MeshView::decode(blob, MeshLimits::default()).map_err(|e| format!("{e:?}"))?;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        let point = mesh.position(index).ok_or("mesh vertex out of range")?;
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(point[axis]));
            max[axis] = max[axis].max(f64::from(point[axis]));
        }
    }
    if min[0] > max[0] {
        return Err("body mesh has no vertices".into());
    }
    Ok((min, max))
}

/// Exports every body as STEP to `<run dir>/<name>.step` (`api::export_step_file`).
pub async fn export_step_artifact(
    access: &DocumentAccess,
    run_id: &str,
    name: &str,
) -> Result<Value, String> {
    let target = run_directory(access, run_id)?.join(format!("{name}.step"));
    let path = Some(target.to_string_lossy().into_owned());
    api::export_step_file(&access.state, access.host.clone(), path)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("STEP export was cancelled")?;
    let provenance = write_provenance(&target, "step", run_id, access)?;
    Ok(json!({ "path": target, "provenance": provenance }))
}

/// Writes `<run dir>/<name>.json`: run id, kernel identity and the summary.
pub async fn write_semantic(
    access: &DocumentAccess,
    run_id: &str,
    name: &str,
    tabs: usize,
) -> Result<Value, String> {
    let summary = document_summary(access, tabs).await?;
    let target = run_directory(access, run_id)?.join(format!("{name}.json"));
    let record = json!({
        "runId": run_id,
        "kind": "semantic",
        "name": name,
        "kernelIdentity": kernel_identity(Some(access)),
        "summary": summary,
    });
    write_json(&target, &record)?;
    Ok(json!({ "path": target }))
}
