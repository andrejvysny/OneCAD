//! `mesh-fixture` — writes MESH1 fixtures for the native renderer from the REAL
//! OneCAD stack: the app crate's `WorkerManager` + `DocumentRuntime` driving the
//! C++ OCCT worker over OCW1 (same harness shape as `crates/onecad-app/tests/step_import.rs`
//! and `crates/onecad-app/tests/fillet_blend_class.rs`). Blobs are written exactly as
//! `DocumentRuntime::get_mesh` returns them (Rust forwards worker bytes verbatim).
//!
//! ```text
//! cargo run -p onecad-mesh-fixture -- [--worker <path>] [--out <dir>] [--step <file>]
//! ```

mod manifest;
mod part;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use onecad_core::document::record::OperationRecord;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ElementId, SnapshotId, TopoKey};
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, Outcome, RegenRequest};
use onecad_protocol::mesh::{f32_le, u32_le};

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::imports::prepare_import;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::body_id_wire;
use onecad_app::worker::{ElementQuery, MeshProvider, SolverEngine, WorkerManager};

use manifest::BodyMeta;

/// The app's committed-geometry tier (`document_runtime.rs` `DISPLAY_LOD`).
const DISPLAY_LOD: Lod = Lod::Fine;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Args {
    worker: PathBuf,
    out: PathBuf,
    step: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let root = repo_root();
    let mut a = Args {
        worker: root.join("worker/build/onecad-worker"),
        out: root.join("fixtures/render"),
        step: root.join("STEP/Nema17 Stepper Motor 17Hs4023.step"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let val = it.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--worker" => a.worker = PathBuf::from(val),
            "--out" => a.out = PathBuf::from(val),
            "--step" => a.step = PathBuf::from(val),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(a)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mesh-fixture: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    if let Err(e) = run(&args).await {
        eprintln!("mesh-fixture: FAILED: {e}");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}

async fn run(args: &Args) -> Result<(), String> {
    if !args.worker.is_file() {
        return Err(format!("no worker binary at {:?}", args.worker));
    }
    let wm = WorkerManager::spawn(SupervisorConfig::production(args.worker.clone()));
    if !wm.wait_ready(Duration::from_secs(20)).await {
        return Err("worker never became ready".into());
    }
    let nema = nema17(&wm, args).await;
    let block = fillet_block(&wm, args).await;
    wm.shutdown().await;
    nema?;
    block
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

/// Regen from step 0; requires a publish with no failed step and no repair.
async fn regen(rt: &mut DocumentRuntime, what: &str) -> Result<RegenReport, String> {
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    if !matches!(report.outcome, Outcome::Published(_)) {
        return Err(format!("{what}: regen not published: {:?}", report.outcome));
    }
    if !report.failed_steps.is_empty() || !report.needs_repair.is_empty() {
        return Err(format!(
            "{what}: failed={:?} needsRepair={:?}",
            report.failed_steps, report.needs_repair
        ));
    }
    Ok(report)
}

fn add(rt: &mut DocumentRuntime, record: OperationRecord) -> Result<(), String> {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .map(|_| ())
    .map_err(|e| format!("AddOperation: {e:?}"))
}

async fn mesh(rt: &mut DocumentRuntime, body: BodyId) -> Result<Arc<Vec<u8>>, String> {
    rt.get_mesh(body, DISPLAY_LOD, None)
        .await
        .ok_or(format!("no mesh for {}", body_id_wire(body)))
}

/// Writes every head body's MESH1 + the manifest under `<out>/<name>/`.
async fn write_fixture(rt: &mut DocumentRuntime, dir: &Path, header: Value) -> Result<(), String> {
    manifest::reset_dir(dir)?;
    let projected = rt.projection().bodies;
    let mut bodies = Vec::new();
    for body in rt.head_body_ids() {
        let blob = mesh(rt, body).await?;
        let dto = projected.get(&body.to_string());
        let meta = BodyMeta {
            body_id: body.to_string(),
            wire_id: body_id_wire(body),
            name: dto.map(|d| d.name.clone()).unwrap_or_default(),
            color: dto.and_then(|d| d.color),
        };
        let entry = manifest::write_body(dir, &meta, &blob)?;
        eprintln!(
            "  {} {:?}: {} B, V={} T={} F={} E={}",
            entry["file"],
            meta.name,
            entry["byteLen"],
            entry["vertexCount"],
            entry["triangleCount"],
            entry["faceCount"],
            entry["edgeCount"]
        );
        bodies.push(entry);
    }
    let mut doc = header;
    doc["meshFormat"] = json!("MESH1");
    doc["mesh1Version"] = json!(1);
    doc["lod"] = json!("fine");
    doc["upAxis"] = json!("+Z");
    doc["units"] = json!("mm");
    doc["bodyCount"] = json!(bodies.len());
    doc["sceneBbox"] = manifest::scene_bbox(&bodies);
    doc["bodies"] = Value::Array(bodies);
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), text + "\n").map_err(|e| e.to_string())
}

async fn nema17(wm: &WorkerManager, args: &Args) -> Result<(), String> {
    eprintln!("nema17: importing {:?}", args.step);
    let mut rt = runtime_over(wm);
    let prepared = prepare_import(wm, &args.step)
        .await
        .map_err(|e| format!("prepare_import: {e:?}"))?;
    rt.add_import_record(&prepared, false)
        .map_err(|e| format!("add_import_record: {e:?}"))?;
    regen(&mut rt, "nema17 import").await?;
    let header = json!({
        "fixture": "nema17",
        "source": "STEP/Nema17 Stepper Motor 17Hs4023.step",
        "pipeline": "prepare_import -> DocumentRuntime::add_import_record -> run_regen(ToEnd{from:0}) -> get_mesh(body, Lod::Fine)",
        "stepSolidCount": prepared.solid_count,
        "stepProductNames": prepared.product_names,
        "importDiagnostics": prepared.diagnostics.iter().map(|(c, m)| json!({"code": c, "message": m})).collect::<Vec<_>>(),
    });
    write_fixture(&mut rt, &args.out.join("nema17"), header).await
}

/// The edge with the greatest world-Z extent (a vertical box edge); first wins ties.
fn vertical_edge_key(blob: &[u8]) -> Result<String, String> {
    let view = manifest::validate(blob)?;
    let (er, ep) = (
        view.section(7).ok_or("no EDGE_RANGES")?,
        view.section(8).ok_or("no EDGE_POSITIONS")?,
    );
    let (offs, chars) = (
        view.section(9).ok_or("no EDGE_ID_OFFS")?,
        view.section(10).ok_or("no EDGE_ID_CHARS")?,
    );
    let mut best: Option<(usize, f32)> = None;
    for i in 0..view.edge_count as usize {
        let first = u32_le(blob, er.offset as usize + 8 * i) as usize;
        let count = u32_le(blob, er.offset as usize + 8 * i + 4) as usize;
        let zs = (first..first + count).map(|p| f32_le(blob, ep.offset as usize + 12 * p + 8));
        let span = zs.clone().fold(f32::MIN, f32::max) - zs.fold(f32::MAX, f32::min);
        if count > 0 && best.is_none_or(|(_, s)| span > s) {
            best = Some((i, span));
        }
    }
    let i = best.ok_or("no edges")?.0;
    let lo = u32_le(blob, offs.offset as usize + 4 * i) as usize;
    let hi = u32_le(blob, offs.offset as usize + 4 * i + 4) as usize;
    let base = chars.offset as usize;
    String::from_utf8(blob[base + lo..base + hi].to_vec()).map_err(|e| e.to_string())
}

async fn promote_edge(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
    snap: SnapshotId,
    key: &str,
) -> Result<(ElementId, Vec3), String> {
    let body = part::body();
    let info = ElementQuery::query_element_by_topo_key(wm, snap, body, key)
        .await
        .map_err(|e| format!("QueryElement {key}: {e:?}"))?
        .ok_or(format!("edge {key} not found"))?;
    let world = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
    let promoted = rt
        .promote_selection(
            snap,
            body,
            vec![(TopoKey::new(key), Some(part::anchor(world)))],
        )
        .await
        .map_err(|e| format!("promote {key}: {e:?}"))?;
    Ok((ElementId::new(promoted[0].element_id.clone()), world))
}

async fn fillet_block(wm: &WorkerManager, args: &Args) -> Result<(), String> {
    eprintln!("fillet-block: sketch 40x30 -> extrude 10 -> fillet r=3");
    let mut rt = runtime_over(wm);
    let (sketch, sid) = part::sketch_record();
    add(&mut rt, sketch)?;
    add(&mut rt, part::extrude_record(sid))?;
    let report = regen(&mut rt, "extrude").await?;
    let snap = SnapshotId(report.snapshot_id);
    let box_mesh = mesh(&mut rt, part::body()).await?;
    let key = vertical_edge_key(&box_mesh)?;
    let (edge, world) = promote_edge(&mut rt, wm, snap, &key).await?;
    add(&mut rt, part::fillet_record(&edge, world))?;
    regen(&mut rt, "fillet").await?;
    let header = json!({
        "fixture": "fillet-block",
        "source": "parametric: Sketch(XY, constrained rect 40x30 at origin) -> Extrude(blind 10, NewBody) -> Fillet(r=3, one vertical edge)",
        "pipeline": "DocumentRuntime AddOperation x3 + promote_selection -> run_regen(ToEnd{from:0}) -> get_mesh(body, Lod::Fine)",
        "filletEdge": {"topoKeyOnExtrudeSnapshot": key, "elementId": edge.as_str(), "anchor": [world.x, world.y, world.z]},
        "params": {"width": part::WIDTH, "height": part::HEIGHT, "depth": part::DEPTH, "filletRadius": part::FILLET_R},
    });
    write_fixture(&mut rt, &args.out.join("fillet-block"), header).await
}
