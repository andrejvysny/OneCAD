//! Synthetic ONECAD-307 reconstruction; the affected user document was not supplied.
//! Real-worker semantic and BRep evidence is separate from native visual acceptance.

#[path = "sketch_finish_stability/fixture.rs"]
mod fixture;
#[allow(dead_code)]
mod native_plate_support;
#[path = "sketch_finish_stability/solver.rs"]
mod solver;

use onecad_app::{
    document_runtime::DocumentRuntime,
    dto::{FinishSketchDto, SketchRegionDto},
    worker::{ElementQuery, SupervisorConfig, WorkerManager},
};
use onecad_core::{
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, SketchId},
    io::container::SaveMeta,
    regen::{CancelToken, Outcome, RegenRequest},
    sketch::{Sketch, SketchPlane},
};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

#[test]
fn reconstructed_finish_preserves_authority_history_regions_and_fresh_reopen() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root =
            std::path::PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_phase(&root, &phase));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for phase in ["create", "reopen"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "reconstructed_finish_preserves_authority_history_regions_and_fresh_reopen",
        );
    }
}

async fn run_phase(root: &Path, phase: &str) {
    for (name, plane) in fixture::frames() {
        let worker = WorkerManager::spawn(SupervisorConfig::production(
            native_plate_support::required_worker(),
        ));
        assert!(worker.wait_ready(Duration::from_secs(15)).await);
        match phase {
            "create" => create(root, name, plane, &worker).await,
            "reopen" => reopen(root, name, &worker).await,
            _ => panic!("unknown Finish phase: {phase}"),
        }
        worker.shutdown().await;
    }
}

fn runtime(worker: &WorkerManager, solver: Arc<solver::RecordedSolver>) -> DocumentRuntime {
    DocumentRuntime::new_blank(Arc::new(worker.clone()), Arc::new(worker.clone()), solver)
}

fn authority(runtime: &DocumentRuntime) -> Value {
    let sketch = runtime
        .sketch_snapshot(fixture::id(), "Finish evidence")
        .unwrap();
    json!({
        "entities": sketch.entities(), "constraints": sketch.constraints(),
        "attachment": sketch.attachment, "authoredPlane": sketch.plane,
        "effectivePlane": sketch.effective_plane(), "projections": sketch.projections,
    })
}

fn timeline(runtime: &DocumentRuntime) -> Value {
    let sketch = runtime
        .sketch_snapshot(fixture::id(), "timeline evidence")
        .unwrap();
    let (_, entities, constraints) = onecad_app::worker::wire::sketch_wire(&sketch);
    let records: Vec<_> = runtime
        .projection()
        .features
        .iter()
        .filter(|feature| feature.op_type == "Sketch")
        .map(|feature| {
            let params = runtime
                .operation_params(feature.id.parse().unwrap())
                .unwrap();
            assert_eq!(
                params["entities"], entities,
                "record must serialize accepted geometry"
            );
            assert_eq!(
                params["constraints"], constraints,
                "record must serialize accepted constraints"
            );
            let frame = serde_json::to_value(sketch.plane).unwrap();
            for component in ["origin", "xAxis", "yAxis", "normal"] {
                assert_eq!(params["plane"][component], frame[component]);
            }
            json!({
                "id": feature.id,
                "params": params,
            })
        })
        .collect();
    assert_eq!(
        records.len(),
        1,
        "Finish must own exactly one sketch record"
    );
    json!(records)
}

fn evidence(runtime: &DocumentRuntime, regions: &FinishSketchDto) -> Value {
    json!({
        "authority": authority(runtime), "timeline": timeline(runtime),
        "documentRevision": runtime.revision().0,
        "undoDepth": runtime.undo_depth(), "regions": regions,
        "truth": runtime.get_sketch(fixture::id()).unwrap().truth,
    })
}

async fn regions(runtime: &mut DocumentRuntime) -> FinishSketchDto {
    runtime
        .prepare_sketch_regions(fixture::id())
        .unwrap()
        .drive()
        .await
        .unwrap()
}

fn assert_regions(regions: &FinishSketchDto, radius: f64) {
    assert!(regions.diagnostics.is_empty(), "{:?}", regions.diagnostics);
    assert_eq!(regions.regions.len(), 4, "pentagon, annulus and two disks");
    assert_eq!(
        regions
            .regions
            .iter()
            .filter(|r| !r.holes.is_empty())
            .count(),
        1
    );
    for region in &regions.regions {
        let triangles = region
            .preview_triangles
            .as_ref()
            .expect("accepted bounded fill");
        assert_eq!(triangles.holes_subtracted as usize, region.holes.len());
        assert!(triangles.positions.iter().all(|x| x.is_finite()));
        assert!(triangles.positions.as_chunks::<2>().0.iter().all(|p| {
            p[0] >= -1e-7 && p[0] <= 104.0 + 1e-7 && p[1] >= -1e-7 && p[1] <= 25.0 + 1e-7
        }));
        let area: f64 = triangles
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                let p = |index: u32| &triangles.positions[index as usize * 2..][..2];
                let (a, b, c) = (p(triangle[0]), p(triangle[1]), p(triangle[2]));
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
            })
            .sum();
        assert!(
            (area - fixture::expected(region, radius).0).abs() < 1.0,
            "region {} sampled area {area}, analytic {}; outer={:?}",
            region.region_id,
            fixture::expected(region, radius).0,
            region.outer_loop
        );
    }
}

async fn create(root: &Path, name: &str, plane: SketchPlane, worker: &WorkerManager) {
    let solver = Arc::new(solver::RecordedSolver::new(worker));
    let mut runtime = runtime(worker, solver.clone());
    let (before, finished) = first_finish(&mut runtime, &solver, name, plane).await;
    let initial = evidence(&runtime, &finished);
    if name == "xy" {
        if let Some(path) = std::env::var_os("ONECAD_FINISH_PROFILE_FIXTURE") {
            runtime.save(Path::new(&path), SaveMeta::default()).unwrap();
        }
    }
    no_op_visits(&mut runtime, &initial).await;
    logical_edit_history(&mut runtime, &before).await;
    let finished = regions(&mut runtime).await;
    for (index, region) in finished.regions.iter().enumerate() {
        runtime
            .apply(EditCommand::AddOperation {
                record: fixture::extrude(region, finished.region_identity_version, index),
                at_cursor: true,
            })
            .unwrap();
    }
    assert_solids(&mut runtime, worker, &finished).await;
    save(root, name, &mut runtime, &finished);
}

async fn first_finish(
    runtime: &mut DocumentRuntime,
    solver: &solver::RecordedSolver,
    name: &str,
    plane: SketchPlane,
) -> (Value, FinishSketchDto) {
    runtime
        .apply(EditCommand::AddSketch {
            sketch: fixture::blank(plane),
        })
        .unwrap();
    runtime.enter_sketch(fixture::id()).await.unwrap();
    let solved = runtime
        .sketch_upsert(fixture::id(), fixture::operations(&fixture::profile(plane)))
        .await
        .unwrap();
    assert!(solved.conflicting.is_empty());
    let before = authority(runtime);
    let before_revision = runtime.revision().0;
    let before_regions = regions(runtime).await;
    let (finished, _) = runtime
        .finish_sketch_with_outcome(fixture::id())
        .await
        .unwrap();
    let final_solve = solver.last();
    assert!(final_solve.solved_positions.is_empty());
    assert!(final_solve.solved_curves.is_empty());
    assert!(final_solve.conflicting.is_empty());
    eprintln!(
        "{name}: final solve revision={}, point corrections={:?}, curve corrections={:?}",
        final_solve.sketch_revision, final_solve.solved_positions, final_solve.solved_curves
    );
    assert_eq!(
        authority(runtime),
        before,
        "final solve must not move solved authority"
    );
    assert_eq!(finished.regions, before_regions.regions);
    assert_regions(&finished, 5.0);
    eprintln!("{name}: Finish document revision {before_revision} -> {}; authority delta=0; initial solve corrections points={} curves={}; truth={:?}", runtime.revision().0, solved.solved_positions.len(), solved.solved_curves.len(), runtime.get_sketch(fixture::id()).unwrap().truth);
    (before, finished)
}

fn save(root: &Path, name: &str, runtime: &mut DocumentRuntime, finished: &FinishSketchDto) {
    let saved = evidence(runtime, finished);
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&saved).unwrap(),
    )
    .unwrap();
    runtime
        .save(&root.join(format!("{name}.onecad")), SaveMeta::default())
        .unwrap();
    if name == "xy" {
        if let Some(path) = std::env::var_os("ONECAD_FINISH_STABILITY_FIXTURE") {
            std::fs::copy(root.join("xy.onecad"), path).unwrap();
        }
    }
}

async fn no_op_visits(runtime: &mut DocumentRuntime, initial: &Value) {
    for _ in 0..3 {
        runtime.enter_sketch(fixture::id()).await.unwrap();
        let (finished, outcome) = runtime
            .finish_sketch_with_outcome(fixture::id())
            .await
            .unwrap();
        assert!(
            outcome.is_none(),
            "unchanged Finish must not mutate history"
        );
        assert_eq!(evidence(runtime, &finished), *initial);
        assert!(runtime
            .get_sketch(fixture::id())
            .unwrap()
            .edit_session
            .is_none());
        let (repeated, outcome) = runtime
            .finish_sketch_with_outcome(fixture::id())
            .await
            .unwrap();
        assert!(
            outcome.is_none(),
            "duplicate confirmation must not mutate history"
        );
        assert_eq!(evidence(runtime, &repeated), *initial);
    }
}

async fn logical_edit_history(runtime: &mut DocumentRuntime, original: &Value) {
    runtime.enter_sketch(fixture::id()).await.unwrap();
    runtime
        .sketch_upsert(
            fixture::id(),
            vec![SketchEditOp::SetDimension {
                constraint: fixture::radius_id(),
                value: onecad_core::document::variables::Scalar::with_expr(6.0, "6mm"),
            }],
        )
        .await
        .unwrap();
    let finished = runtime.finish_sketch(fixture::id()).await.unwrap();
    assert_regions(&finished, 6.0);
    let edited = authority(runtime);
    assert_ne!(
        &edited, original,
        "accepted radius edit must change authority"
    );
    runtime.undo().expect("sketch visit Undo");
    assert_eq!(&authority(runtime), original);
    assert_regions(&regions(runtime).await, 5.0);
    assert!(runtime.redo().unwrap().is_some());
    assert_eq!(authority(runtime), edited);
    assert_regions(&regions(runtime).await, 6.0);
}

async fn assert_solids(
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
    regions: &FinishSketchDto,
) {
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    assert_eq!(runtime.projection().bodies.len(), 4);
    for (index, region) in regions.regions.iter().enumerate() {
        let body = BodyId(Uuid::from_u128(0x307a00 + index as u128));
        let topology = worker
            .query_body_topology(body, body.to_string())
            .await
            .unwrap();
        let mass = worker
            .query_mass_properties(body, body.to_string())
            .await
            .unwrap();
        let (area, faces) = fixture::expected(region, 6.0);
        assert_eq!((topology.solid_count, topology.face_count), (1, faces));
        assert!((mass.volume - area * 6.0).abs() < 1e-5, "{mass:?}");
    }
}

async fn reopen(root: &Path, name: &str, worker: &WorkerManager) {
    let mut runtime = DocumentRuntime::open(
        &root.join(format!("{name}.onecad")),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    )
    .unwrap();
    let saved: Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap();
    assert_semantic(&authority(&runtime), &saved["authority"], "authority");
    assert_semantic(&timeline(&runtime), &saved["timeline"], "timeline");
    let regions = regions(&mut runtime).await;
    assert_semantic(
        &serde_json::to_value(&regions).unwrap(),
        &saved["regions"],
        "regions",
    );
    assert_regions(&regions, 6.0);
    assert_solids(&mut runtime, worker, &regions).await;
    assert_semantic(&authority(&runtime), &saved["authority"], "authority");
    assert_semantic(&timeline(&runtime), &saved["timeline"], "timeline");
    eprintln!("{name}: fresh process reopen preserves entities, constraints, support, record, four regions; exact volumes and hole topology qualified");
}

fn assert_semantic(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            // Evidence JSON parsing may round the last bit; 1e-9 mm remains far below mesh tolerance.
            assert!(
                (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9,
                "{path}: {a} differs from {b}"
            );
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: array length changed");
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                assert_semantic(a, b, &format!("{path}[{index}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path}"
            );
            for (key, value) in a {
                assert_semantic(value, &b[key], &format!("{path}.{key}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
