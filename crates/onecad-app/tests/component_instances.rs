//! Component-instance management against the REAL C++ OCCT worker: list,
//! plan/apply version upgrades as one undo step, stale-plan refusal, save and
//! fresh-worker reopen, and reset overrides.
//!
//! Gated on `ONECAD_WORKER_PATH`; `ONECAD_REQUIRE_WORKER=1` makes a missing
//! binary a hard failure.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use onecad_core::document::record::{
    ComponentParamValue, KnownOperation, Operation, OperationRecord, PlaceComponentParams,
};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId};
use onecad_core::io::container::SaveMeta;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};

use onecad_app::api;
use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::{ComponentStatusDto, UpgradeItemDto};
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::library::{
    copy_placement_commands, prepare_component_draft_at, reindex_library_at,
    set_component_params_at, PlaceComponentRequest,
};
use onecad_app::library_instances::{
    apply_component_upgrades_at, component_instances_at, plan_component_upgrades_at,
    reset_component_overrides_at,
};
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

const ID: &str = "me.test.screw";

// ─────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn onecad_app::export::GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        let face_projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
        let step_import: Arc<dyn StepImport> = Arc::new(wm.clone());
        let circuit: Arc<dyn CircuitControl> = Arc::new(wm.clone());
        let readiness: Arc<dyn WorkerReadiness> = Arc::new(wm.clone());
        (
            engine,
            meshes,
            solver,
            exporter,
            elements,
            preview,
            face_projection,
            step_import,
            circuit,
            readiness,
        )
    }))
}

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct NoDialogs;
#[async_trait]
impl DialogPort for NoDialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        panic!("no dialog expected")
    }
}

fn host() -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: None,
            app_config: None,
        },
        events: Arc::new(Events::default()),
        dialogs: Arc::new(NoDialogs),
    }
}

async fn open_blank(state: &AppState) {
    let (engine, meshes, solver) = state.make_backend();
    *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
}

fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-11-01T00:00:00Z".into(),
        modified: "2026-11-01T00:00:00Z".into(),
    }
}

/// Regens the open document to its end and returns the published snapshot.
async fn regen(state: &AppState) -> Arc<ModelSnapshot> {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().expect("open document");
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::Published(s) => s,
        other => panic!("expected Published, got {other:?}"),
    }
}

async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

type Params = BTreeMap<String, ComponentParamValue>;

fn num(v: f64) -> ComponentParamValue {
    ComponentParamValue::Number(v)
}

fn text(v: &str) -> ComponentParamValue {
    ComponentParamValue::Text(v.into())
}

/// v1.0.0 declares `thread` (M3..M8), `length` and `thread_detail`; v1.1.0 narrows
/// `thread` to M3..M6 and drops `thread_detail` as a free parameter.
fn write_version(root: &Path, version: &str) {
    let v11 = version == "1.1.0";
    let (domain, detail) = if v11 {
        (r#"["M3", "M4", "M5", "M6"]"#, "")
    } else {
        (
            r#"["M3", "M4", "M5", "M6", "M8"]"#,
            r#"thread_detail = { role = "free", value = "cosmetic", domain = ["cosmetic", "simplified", "modeled"] }"#,
        )
    };
    let dir = root.join(format!("{ID}@{version}"));
    std::fs::create_dir_all(&dir).unwrap();
    let toml = format!(
        r#"
[identity]
id = "{ID}"
version = "{version}"
revision = "sha256:{rev}"

[metadata]
name = "Test screw"
designation = "ISO 4762 {{thread}}x{{length}}"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
thread = {{ role = "free", value = "M6", domain = {domain} }}
length = {{ role = "free", value = 20, min = 2 }}
{detail}
"#,
        rev = if v11 { "1" } else { "0" }.repeat(64)
    );
    std::fs::write(dir.join("component.toml"), toml).unwrap();
}

fn request(translate: [f64; 3], params: Params) -> PlaceComponentRequest {
    PlaceComponentRequest {
        component_id: ID.into(),
        component_version: "1.0.0".into(),
        translate,
        rotate: None,
        params,
        mate: None,
        configuration: None,
    }
}

async fn place(root: &Path, state: &AppState, at: f64, params: Params) -> String {
    let draft = prepare_component_draft_at(root, state, request([at, 0.0, 0.0], params))
        .await
        .expect("draft");
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Place Component",
        Operation::Known(KnownOperation::PlaceComponent(draft)),
    );
    let id = record.record_id.to_string();
    api::apply_operation(
        state,
        host(),
        vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }],
        None,
    )
    .await
    .expect("commit placement");
    id
}

async fn place_copy(root: &Path, state: &AppState, at: f64) -> String {
    let draft = prepare_component_draft_at(root, state, request([at, 0.0, 0.0], Params::new()))
        .await
        .expect("draft");
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Place Component",
        Operation::Known(KnownOperation::PlaceComponent(draft)),
    );
    let cmds = copy_placement_commands(record, None).expect("copy");
    let EditCommand::AddOperation { record, .. } = &cmds[0] else {
        panic!("one AddOperation");
    };
    let id = record.record_id.to_string();
    api::apply_operation(state, host(), cmds, None)
        .await
        .expect("commit copy");
    id
}

async fn placed(state: &AppState, id: &str) -> PlaceComponentParams {
    let guard = state.runtime.lock().await;
    let raw = guard
        .as_ref()
        .unwrap()
        .operation_params(id.parse::<RecordId>().unwrap())
        .expect("params");
    serde_json::from_value(raw).expect("PlaceComponent")
}

async fn volumes(wm: &WorkerManager, snap: &ModelSnapshot) -> Vec<f64> {
    let mut out = Vec::new();
    for b in &snap.bodies {
        out.push(exact_volume(wm, b.body).await);
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

fn assert_vols(a: &[f64], b: &[f64], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: body count");
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-6, "{what}: {a:?} vs {b:?}");
    }
}

async fn undo_depth(state: &AppState) -> usize {
    state.runtime.lock().await.as_ref().unwrap().undo_depth()
}

struct Doc {
    library: tempfile::TempDir,
    wm: WorkerManager,
    state: AppState,
    a: String,
    b: String,
    copy: String,
}

async fn document() -> Option<Doc> {
    let bin = real_worker().or_else(|| {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        None
    })?;
    let library = tempfile::tempdir().expect("tempdir");
    write_version(library.path(), "1.0.0");
    reindex_library_at(library.path()).expect("index v1.0.0");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let root = library.path();
    let a = place(
        root,
        &state,
        0.0,
        Params::from([("length".into(), num(30.0))]),
    )
    .await;
    let b = place(
        root,
        &state,
        40.0,
        Params::from([("thread_detail".into(), text("simplified"))]),
    )
    .await;
    let copy = place_copy(root, &state, 80.0).await;
    write_version(root, "1.1.0");
    reindex_library_at(root).expect("index v1.1.0");
    Some(Doc {
        library,
        wm,
        state,
        a,
        b,
        copy,
    })
}

#[tokio::test]
async fn lists_plans_and_upgrades_two_linked_instances_as_one_undo_step() {
    let Some(d) = document().await else { return };
    let root = d.library.path();
    let before = regen(&d.state).await;
    assert_eq!(before.bodies.len(), 3);
    let vols_before = volumes(&d.wm, &before).await;

    let list = component_instances_at(Some(root), &d.state).await.unwrap();
    assert_eq!(list.len(), 3);
    assert!(list[0].linked && list[1].linked && !list[2].linked);
    assert_eq!(list[0].overrides, ["length"]);
    assert_eq!(list[0].designation.as_deref(), Some("ISO 4762 M6x30"));
    assert_eq!(list[1].overrides, ["thread_detail"]);
    assert!(matches!(
        list[0].status,
        ComponentStatusDto::UpgradeAvailable { ref version } if version == "1.1.0"
    ));
    assert_eq!(list[2].status, ComponentStatusDto::Detached);

    let ids = vec![d.a.clone(), d.b.clone(), d.copy.clone()];
    let plan = plan_component_upgrades_at(root, &d.state, ids, None)
        .await
        .unwrap();
    assert!(plan[0].applicable && plan[1].applicable && !plan[2].applicable);
    assert_eq!(
        plan[0].kept_overrides,
        Params::from([("length".into(), num(30.0))])
    );
    assert_eq!(plan[1].dropped_overrides[0].key, "thread_detail");

    let (a0, b0) = (placed(&d.state, &d.a).await, placed(&d.state, &d.b).await);
    let depth = undo_depth(&d.state).await;
    apply_component_upgrades_at(root, &d.state, plan)
        .await
        .expect("apply");
    assert_eq!(undo_depth(&d.state).await, depth + 1, "one undo step");
    let (a1, b1) = (placed(&d.state, &d.a).await, placed(&d.state, &d.b).await);
    assert_eq!(a1.component_version, "1.1.0");
    assert_eq!(b1.component_version, "1.1.0");
    assert!(b1.params.is_empty());
    let after = regen(&d.state).await;
    assert_eq!(after.bodies.len(), 3, "copy untouched, nothing lost");
    // A's length=30 is kept and the copy is untouched; B's dropped `simplified`
    // override falls back to the cosmetic default, i.e. the copy's volume.
    assert!(vols_before[0] < vols_before[1], "override had an effect");
    let expected = [vols_before[1], vols_before[1], vols_before[2]];
    assert_vols(&expected, &volumes(&d.wm, &after).await, "upgrade");

    api::undo(&d.state, host()).await.expect("undo");
    assert_eq!(placed(&d.state, &d.a).await, a0);
    assert_eq!(placed(&d.state, &d.b).await, b0);
    api::redo(&d.state, host()).await.expect("redo");
    assert_eq!(placed(&d.state, &d.a).await, a1);
    assert_eq!(placed(&d.state, &d.b).await, b1);

    let list = component_instances_at(Some(root), &d.state).await.unwrap();
    assert_eq!(list[2].status, ComponentStatusDto::Detached);
    assert_eq!(list[0].component_version, "1.1.0");
    d.wm.shutdown().await;
    d.wm.retire();
}

#[tokio::test]
async fn a_stale_plan_is_refused_and_changes_nothing() {
    let Some(d) = document().await else { return };
    let root = d.library.path();
    let plan: Vec<UpgradeItemDto> =
        plan_component_upgrades_at(root, &d.state, vec![d.a.clone(), d.b.clone()], None)
            .await
            .unwrap();
    set_component_params_at(
        root,
        &d.state,
        d.a.clone(),
        Params::from([("length".into(), num(35.0))]),
    )
    .await
    .expect("mutate A after planning");
    let (a, b) = (placed(&d.state, &d.a).await, placed(&d.state, &d.b).await);
    let depth = undo_depth(&d.state).await;
    let err = apply_component_upgrades_at(root, &d.state, plan)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("upgrade plan is stale"), "{err}");
    assert_eq!(placed(&d.state, &d.a).await, a);
    assert_eq!(placed(&d.state, &d.b).await, b);
    assert_eq!(undo_depth(&d.state).await, depth);
    d.wm.shutdown().await;
    d.wm.retire();
}

#[tokio::test]
async fn an_upgrade_survives_save_and_a_fresh_worker_reopen() {
    let Some(d) = document().await else { return };
    let root = d.library.path();
    let plan = plan_component_upgrades_at(root, &d.state, vec![d.a.clone(), d.b.clone()], None)
        .await
        .unwrap();
    apply_component_upgrades_at(root, &d.state, plan)
        .await
        .expect("apply");
    let snap = regen(&d.state).await;
    let vols = volumes(&d.wm, &snap).await;
    let (a1, b1) = (placed(&d.state, &d.a).await, placed(&d.state, &d.b).await);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgraded.onecad");
    d.state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    d.wm.shutdown().await;
    d.wm.retire();

    let wm2 = spawn_worker(real_worker().expect("worker")).await;
    let mut reopened = open_over(&wm2, &path);
    let report = reopened
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap2) = report.outcome else {
        panic!("reopen must publish");
    };
    for (id, expected) in [(&d.a, &a1), (&d.b, &b1)] {
        let raw = reopened
            .operation_params(id.parse::<RecordId>().unwrap())
            .expect("params");
        let got: PlaceComponentParams = serde_json::from_value(raw).unwrap();
        assert_eq!(&got, expected, "record {id} survives reopen");
    }
    assert_vols(&vols, &volumes(&wm2, &snap2).await, "reopen");
    wm2.shutdown().await;
    wm2.retire();
}

#[tokio::test]
async fn reset_overrides_is_one_undo_step_and_effective_equals_defaults() {
    let Some(d) = document().await else { return };
    let root = d.library.path();
    let depth = undo_depth(&d.state).await;
    reset_component_overrides_at(root, &d.state, d.a.clone(), None)
        .await
        .expect("reset");
    assert_eq!(undo_depth(&d.state).await, depth + 1, "one undo step");
    let list = component_instances_at(Some(root), &d.state).await.unwrap();
    assert!(list[0].overrides.is_empty());
    assert_eq!(list[0].effective, list[0].defaults);
    let snap = regen(&d.state).await;
    assert_eq!(snap.bodies.len(), 3);
    api::undo(&d.state, host()).await.expect("undo");
    assert_eq!(
        placed(&d.state, &d.a).await.params,
        Params::from([("length".into(), num(30.0))])
    );
    d.wm.shutdown().await;
    d.wm.retire();
}
