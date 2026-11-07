//! A captured package survives independent external changes at each service boundary.
use super::*;
use onecad_core::io::container::ContainerReader;
use onecad_library::{package, resolve::ResolveRequest};

fn change_package(root: &Path, change: impl FnOnce(&mut package::ComponentPackage)) {
    let directory = root.join(SCREW);
    let manifest = directory.join("component.toml");
    let raw = std::fs::read_to_string(&manifest).unwrap();
    let mut package = package::parse(&raw, &manifest).unwrap();
    change(&mut package);
    package::write_package(&directory, &package).unwrap();
    reindex_library_at(root).unwrap();
}

fn saved_package_matches(path: &Path, captured: &PlaceComponentParams, manifest: &str) {
    let container = ContainerReader::open(path).unwrap();
    assert_eq!(container.library_archives().len(), 1);
    let hash = &container.library_archives()[0].sha256;
    let archive = container.read_library_archive(hash).unwrap();
    let expected = ResolveRequest {
        component_id: &captured.component_id,
        component_version: &captured.component_version,
        component_revision: &captured.component_revision,
    };
    let decoded = onecad_library::archive::decode(&archive, hash, &expected, 32 << 20).unwrap();
    assert_eq!(decoded.manifest, manifest);
    let record = container.document().timeline.records().last().unwrap();
    let Operation::Known(KnownOperation::PlaceComponent(saved)) = &record.op else {
        panic!("saved linked placement");
    };
    assert_eq!(saved, captured);
}

async fn verify_cold_geometry(
    bin: PathBuf,
    path: &Path,
    body: BodyId,
    bbox: ([f64; 3], [f64; 3]),
    volume: f64,
) {
    let worker = spawn_worker(bin).await;
    let mut runtime = open_over(&worker, path);
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    let state = app_state_over(&worker);
    *state.runtime.lock().await = Some(runtime);
    assert_bbox_eq(
        committed_bbox(&state, body).await,
        bbox,
        "offline cold package replay",
    );
    assert!((exact_volume(&worker, body).await - volume).abs() < 1e-6);
    worker.shutdown().await;
}

struct CapturedPlacement {
    draft: PlaceComponentParams,
    record: OperationRecord,
    bounds: ([f64; 3], [f64; 3]),
}

async fn capture_before_default_change(state: &AppState, root: &Path) -> CapturedPlacement {
    let draft = prepare_component_draft_at(root, state, request([30., 0., 0.]))
        .await
        .unwrap();
    let record = place_record(draft.clone());
    change_package(root, |package| {
        package.parameters.get_mut("length").unwrap().value = Some(toml::Value::Integer(21));
    });
    let previewed = preview(state, &draft, record.record_id).await;
    assert_eq!(previewed.bodies.len(), 1);
    CapturedPlacement {
        draft,
        record,
        bounds: mesh_bbox(&previewed.bodies[0].mesh),
    }
}

fn replace_generator_and_frame(root: &Path) {
    change_package(root, |package| {
        package.attachments.get_mut("head_seat").unwrap().frame = Some(package::AttachmentFrame {
            origin: [4., 0., 0.],
            z: [0., 0., 1.],
            x: [1., 0., 0.],
        });
        package.source = package::SourceSpec::Generator {
            generator: "iso7089".into(),
            generator_version: 1,
        };
    });
}

async fn commit_captured(
    state: &AppState,
    worker: &WorkerManager,
    plate: BodyId,
    captured: &CapturedPlacement,
) -> (BodyId, f64) {
    api::apply_operation(state, host(), vec![add(captured.record.clone())], None)
        .await
        .unwrap();
    let snapshot = regen(state).await;
    assert_eq!(snapshot.bodies.len(), 2);
    let body = snapshot
        .bodies
        .iter()
        .find(|body| body.body != plate)
        .unwrap()
        .body;
    assert_bbox_eq(
        committed_bbox(state, body).await,
        captured.bounds,
        "captured preview vs commit",
    );
    let row = Iso4762Table::row("M6").unwrap();
    for (dimension, expected) in
        dims(captured.bounds)
            .into_iter()
            .zip([row.d2, row.d2, row.k + 20.])
    {
        assert!((dimension - expected).abs() < 1e-4);
    }
    let volume = exact_volume(worker, body).await;
    assert!(volume.is_finite() && volume > 0.);
    (body, volume)
}

async fn save_snapshot(
    state: &AppState,
    path: &Path,
    draft: &PlaceComponentParams,
    manifest: &str,
) {
    let saved = api::save_document(
        state,
        host(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    saved_package_matches(path, draft, manifest);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn captured_generator_survives_changes_between_preview_commit_and_save() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let manifest =
        std::fs::read_to_string(library.path().join(SCREW).join("component.toml")).unwrap();
    let worker = spawn_worker(bin.clone()).await;
    let state = app_state_over(&worker);
    open_blank(&state).await;
    let (plate, _) = author_bored_plate(&state, 0xD1E100).await;
    let captured = capture_before_default_change(&state, library.path()).await;
    replace_generator_and_frame(library.path());
    let (body, volume) = commit_captured(&state, &worker, plate, &captured).await;
    std::fs::remove_dir_all(library.path()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("captured.onecad");
    save_snapshot(&state, &path, &captured.draft, &manifest).await;
    worker.shutdown().await;
    worker.retire();
    verify_cold_geometry(bin, &path, body, captured.bounds, volume).await;
}
