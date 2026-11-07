//! Source-qualified imported Robonine datum workflow; not contact or travel qualification.
#[path = "robonine_analytic_support/export_reimport.rs"]
mod export_reimport;
#[path = "robonine_analytic_support/joints.rs"]
mod joint_support;
mod public_assembly_artifacts;
mod robonine_analytic_support;
use async_trait::async_trait;
use joint_support::{fixtures, pose};
use onecad_app::{
    api,
    dto::AssemblyProjectionDto,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    joint_execution,
    state::AppState,
};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::{ComponentDef, JointBinding, JointSide},
        import_source_identity::ImportSourceAddress,
        record::{
            ImportSourceCodec, ImportStepParams, KnownOperation, Operation, OperationRecord,
            IMPORT_HEAL_POLICY_V1,
        },
        variables::Scalar,
        Document,
    },
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use robonine_analytic_support::write_fixture;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use uuid::Uuid;

const FILES: [&str; 4] = [
    "part-010.xbf",
    "part-020.xbf",
    "part-030.xbf",
    "part-040.xbf",
];
const HASHES: [&str; 4] = [
    "f643cfdf9ec480a23d14888bf896cad30e02bd05be079a0baf11f7fc88bd1992",
    "932cc0452078850d9059ed6ad0716dd3658d35b8117c5598c2f0699e2b14b21c",
    "d9880bc3f2d82b2e9cf49919603ada70af7e3f4c7d9d60caeb3f81a26fc28d8e",
    "2e780afa05c59f10fb3a8c4acf1cc0c2289d45312dedc0172bc7fa83ecbeffc0",
];
const FACES: [u32; 4] = [181, 168, 108, 96];
const SIZES: [usize; 4] = [118968, 154964, 58366, 85170];
const MAP: [usize; 6] = [0, 1, 2, 3, 1, 2];
struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}
fn def(i: usize) -> ComponentDefId {
    ComponentDefId(Uuid::from_u128(0xC200 + i as u128))
}
fn occ(i: usize) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(0xC400 + i as u128))
}
fn record(i: usize) -> RecordId {
    RecordId(Uuid::from_u128(0xC300 + i as u128))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn observation() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/robonine/analytic/observations.json"
    ))
    .unwrap()
}
fn params(i: usize, scale: f64) -> ImportStepParams {
    ImportStepParams {
        source_sha256: HASHES[i].into(),
        source_codec: ImportSourceCodec::Xbf,
        source_name: FILES[i].into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(scale),
        brep_format: Some(12),
        provenance_sha256: None,
        extra: Default::default(),
    }
}
fn host(root: &Path) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    }
}
async fn open(path: &Path, area: &Path) -> (AppState, HostContext, String) {
    let host = host(area);
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    let opened = api::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(state.readiness().wait_ready(Duration::from_secs(15)).await);
    (state, host, opened.runtime_session)
}
async fn solve(
    state: &AppState,
    session: &str,
    joints: Vec<Joint>,
    bindings: Vec<JointBinding>,
    drives: Vec<(JointId, f64)>,
    cancelled: bool,
) -> Result<joint_execution::JointSolveReport, onecad_app::error::ApiError> {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    joint_execution::solve_bound_joints_at_generation_cancellable(
        state,
        session,
        projection.persisted_generation,
        joints,
        drives,
        bindings,
        Arc::new(AtomicBool::new(cancelled)),
    )
    .await
}
async fn drive(
    state: &AppState,
    session: &str,
    values: Vec<(JointId, f64)>,
    cancelled: bool,
) -> Result<joint_execution::JointSolveReport, onecad_app::error::ApiError> {
    let p = api::get_assembly_projection(state, session).await.unwrap();
    let bindings = api::get_joint_bindings(state, session, p.persisted_generation)
        .await
        .unwrap();
    solve(state, session, p.joints, bindings, values, cancelled).await
}
fn assert_pose_near(a: RigidTransform, b: RigidTransform) {
    const SOLVE_TOLERANCE: f64 = 1e-9;
    assert!((a.translation - b.translation).norm() < SOLVE_TOLERANCE);
    for axis in [Vec3d::X, Vec3d::Y, Vec3d::Z] {
        assert!((a.apply_vector(axis) - b.apply_vector(axis)).norm() < SOLVE_TOLERANCE);
    }
}
fn retain_export(source: &Path, destination: &Path) {
    use std::io::Write;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .unwrap();
    let bytes = std::fs::read(source).unwrap();
    out.write_all(&bytes).unwrap();
    out.sync_all().unwrap();
    let manifest = serde_json::json!({
        "format": "flattened STEP, millimetres; no assembly joints or hierarchy",
        "stepSha256": digest(&bytes),
        "sourceFiles": FILES, "sourceSha256": HASHES,
        "occurrenceDefinitionIndices": MAP,
        "drives": {"clamp1Mm": -1.0, "clamp2Mm": -2.0, "gearRad": 0.2},
        "expectedSolidCount": 6,
        "scope": "Kinematic fixture, not contact or physical range qualification"
    });
    let oracle_path = destination.with_extension("oracle.json");
    let mut oracle = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(oracle_path)
        .unwrap();
    oracle
        .write_all(&serde_json::to_vec_pretty(&manifest).unwrap())
        .unwrap();
    oracle.sync_all().unwrap();
}
fn stable(p: &AssemblyProjectionDto) -> serde_json::Value {
    serde_json::json!({"root":p.root_definition,"occurrences":p.occurrences,"joints":p.joints,"bindings":p.joint_bindings})
}
async fn child(path: &Path, area: &Path) {
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(area.join("joints-expected.json")).unwrap()).unwrap();
    let (state, host, session) = open(path, area).await;
    for i in 0..4 {
        api::regenerate_component(&state, &session, def(i))
            .await
            .unwrap();
    }
    let current = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(stable(&current), expected);
    assert!(current
        .joint_bindings
        .iter()
        .all(|b| b.verified && !b.needs_repair));
    let report = drive(
        &state,
        &session,
        vec![(JointId(Uuid::from_u128(0xC601)), -3.0)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(report.remaining_dof, 2);
    let changed = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!((pose(&changed, 1).translation.x + 3.0).abs() < 1e-6);
    api::shutdown_host(&state, host, None).await.unwrap();
}
#[tokio::test]
async fn source_qualified_analytic_joints_cold_reopen() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    if let Some(area) = std::env::var_os("ONECAD_ROBONINE_JOINT_CHILD") {
        let area = Path::new(&area);
        child(&area.join("joints.onecad"), area).await;
        return;
    }
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("joints.onecad");
    write_fixture(&path);
    let (state, host, session) = open(&path, area.path()).await;
    for i in 0..4 {
        api::regenerate_component(&state, &session, def(i))
            .await
            .unwrap();
    }
    assert!(
        api::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    let (joints, bindings) = fixtures(&state, &session, &path).await;
    if let Some(capture) = std::env::var_os("ONECAD_JOINT_INPUT_CAPTURE") {
        let projection = api::get_assembly_projection(&state, &session)
            .await
            .unwrap();
        let bodies = (0..6)
            .map(|i| onecad_assembly::solver::SolverBody {
                id: occ(i),
                pose: pose(&projection, i),
                grounded: i == 0,
            })
            .collect();
        let input = onecad_assembly::solver::SolveRequest::new(bodies, joints.clone());
        std::fs::write(capture, serde_json::to_vec_pretty(&input).unwrap()).unwrap();
    }
    let report = solve(&state, &session, joints, bindings, vec![], false)
        .await
        .unwrap();
    assert_eq!(report.remaining_dof, 3);
    let zero = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!(zero
        .joint_bindings
        .iter()
        .all(|b| b.verified && !b.needs_repair));
    let id1 = JointId(Uuid::from_u128(0xC601));
    let id2 = JointId(Uuid::from_u128(0xC602));
    let id3 = JointId(Uuid::from_u128(0xC603));
    let report = drive(&state, &session, vec![(id1, -1.0)], false)
        .await
        .unwrap();
    assert_eq!(report.remaining_dof, 2);
    let moved = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!((pose(&moved, 1).translation.x + 1.0).abs() < 1e-6);
    assert!((pose(&moved, 2).translation.x + 1.0).abs() < 1e-6);
    assert_eq!(pose(&moved, 0), RigidTransform::IDENTITY);
    assert_pose_near(pose(&moved, 4), pose(&zero, 4));
    assert_pose_near(pose(&moved, 5), pose(&zero, 5));
    api::undo(&state, host.clone()).await.unwrap();
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&zero)
    );
    api::redo(&state, host.clone()).await.unwrap();
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&moved)
    );
    if let Some(capture) = std::env::var_os("ONECAD_JOINT_ENDPOINT_CAPTURE") {
        let projection = api::get_assembly_projection(&state, &session)
            .await
            .unwrap();
        let bodies = (0..6)
            .map(|i| onecad_assembly::solver::SolverBody {
                id: occ(i),
                pose: pose(&projection, i),
                grounded: i == 0,
            })
            .collect();
        let mut input = onecad_assembly::solver::SolveRequest::new(bodies, projection.joints);
        input.drive = vec![(id1, -42.0), (id2, -42.0), (id3, std::f64::consts::PI)];
        std::fs::write(capture, serde_json::to_vec_pretty(&input).unwrap()).unwrap();
    }
    let report = drive(
        &state,
        &session,
        vec![(id1, -42.0), (id2, -42.0), (id3, std::f64::consts::PI)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(report.remaining_dof, 0);
    let endpoints = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(pose(&endpoints, 0), RigidTransform::IDENTITY);
    assert!((pose(&endpoints, 1).translation.x + 42.0).abs() < 1e-6);
    assert!((pose(&endpoints, 4).translation.x - 42.0).abs() < 1e-6);
    assert!(drive(&state, &session, vec![(id1, -42.001)], false)
        .await
        .is_err());
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&endpoints)
    );
    assert!(drive(&state, &session, vec![(id2, -1.0)], true)
        .await
        .is_err());
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&endpoints)
    );
    drive(
        &state,
        &session,
        vec![(id1, -1.0), (id2, -2.0), (id3, 0.2)],
        false,
    )
    .await
    .unwrap();
    let saved = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!((pose(&saved, 1).translation.x + 1.0).abs() < 1e-6);
    assert!((pose(&saved, 4).translation.x - 2.0).abs() < 1e-6);
    let export_path = area.path().join("robonine-posed.step");
    assert!(!export_path.exists());
    let target = export_path.to_string_lossy().into_owned();
    let exported = api::export_step_file(&state, host.clone(), Some(target.clone()))
        .await
        .unwrap();
    assert_eq!(exported.as_deref(), Some(target.as_str()));
    assert!(export_path.metadata().unwrap().len() > 0);
    let after_export = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert_eq!(stable(&after_export), stable(&saved));
    export_reimport::verify(&export_path, area.path()).await;
    if let Some(destination) = std::env::var_os("ONECAD_ROBONINE_EXPORT_OUT") {
        retain_export(&export_path, Path::new(&destination));
    }
    std::fs::write(
        area.path().join("joints-expected.json"),
        serde_json::to_vec(&stable(&saved)).unwrap(),
    )
    .unwrap();
    assert!(
        api::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    api::shutdown_host(&state, host, None).await.unwrap();
    public_assembly_artifacts::retain(
        &path,
        "robonine-joints",
        include_bytes!("robonine_analytic_joints.rs"),
        "imported source parts; not native feature-history qualification",
    );
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "source_qualified_analytic_joints_cold_reopen",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ONECAD_ROBONINE_JOINT_CHILD", area.path())
        .status()
        .unwrap();
    assert!(status.success(), "cold process failed: {status}");
    let (state, host, session) = open(&path, area.path()).await;
    for i in 0..4 {
        api::regenerate_component(&state, &session, def(i))
            .await
            .unwrap();
    }
    let before = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    let after = api::apply_assembly_edit_at_generation(
        &state,
        &session,
        before.persisted_generation,
        AssemblyEdit::EditDefinition {
            id: def(1),
            command: Box::new(EditCommand::SetOperationSuppression {
                record: record(1),
                suppressed: true,
                cascade: false,
            }),
        },
    )
    .await
    .unwrap();
    for id in [id1, id2] {
        let binding = after.joint_bindings.iter().find(|b| b.joint == id).unwrap();
        assert!(binding.needs_repair);
    }
    assert_eq!(pose(&before, 1), pose(&after, 1));
    assert_eq!(pose(&before, 4), pose(&after, 4));
    api::shutdown_host(&state, host, None).await.unwrap();
}
