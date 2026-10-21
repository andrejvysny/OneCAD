//! The in-editor "Insert STEP…" lane (`api::insert_step`) against the REAL worker,
//! through the production host: dialog, readiness, probe, authoring and scheduler.
//!
//! * `insert_step_keeps_history_and_survives_undo_and_reopen` — a sketch + extrude
//!   document receives the checked-in `colored_boxes.step` (millimetre units): the
//!   prior records, ids and body are untouched, the imported bodies land at the
//!   source placement, undo removes exactly the insertion, and save + reopen on a
//!   fresh host and worker keeps it.
//! * `insert_step_refuses_a_document_replaced_before_commit` — the document is
//!   replaced while the insert is between its identity capture and its commit; the
//!   insert refuses and the replacement receives nothing.
//!
//! `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary instead of skipping.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::api;
use onecad_app::dto::{DocumentProjection, FeatureStatus, RegenFinished};
use onecad_app::error::ApiError;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::state::AppState;
use onecad_app::worker::resolve_worker_path;
use onecad_app::worker::wire::sketch_wire;
use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Host harness
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);

impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

/// Answers each STEP dialog with the queued path. With `hold` set the pick parks
/// until `release`, so a test can act while the insert has captured its document.
#[derive(Default)]
struct Dialogs {
    picks: Mutex<Vec<String>>,
    hold: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl DialogPort for Dialogs {
    async fn pick(&self, dialog: FileDialog) -> Vec<String> {
        assert!(!dialog.save, "insert STEP opens a file");
        let picks = std::mem::take(&mut *self.picks.lock().unwrap());
        if self.hold.load(Ordering::Acquire) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        picks
    }
}

struct Harness {
    state: AppState,
    host: HostContext,
    events: Arc<Events>,
    dialogs: Arc<Dialogs>,
}

impl Harness {
    async fn new(root: &Path) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let events = Arc::new(Events::default());
        let dialogs = Arc::new(Dialogs::default());
        let host = HostContext {
            paths: HostPaths {
                app_data: Some(root.join("data")),
                app_config: Some(root.join("config")),
            },
            events: events.clone(),
            dialogs: dialogs.clone(),
        };
        let state = AppState::default();
        state.start(host.clone()).expect("host starts");
        Self {
            state,
            host,
            events,
            dialogs,
        }
    }

    async fn ready(&self) {
        assert!(
            self.state
                .readiness()
                .wait_ready(Duration::from_secs(15))
                .await,
            "real worker must handshake"
        );
    }

    async fn projection(&self) -> DocumentProjection {
        api::get_projection(&self.state).await.unwrap()
    }

    fn pick(&self, path: &Path) {
        *self.dialogs.picks.lock().unwrap() = vec![path.to_string_lossy().into_owned()];
    }

    async fn apply(&self, record: OperationRecord) -> DocumentProjection {
        api::apply_edit_command(
            &self.state,
            self.host.clone(),
            EditCommand::AddOperation {
                record,
                at_cursor: true,
            },
        )
        .await
        .expect("AddOperation")
    }

    /// Waits for the scheduler's terminal for `source` and requires a clean head.
    async fn settled(&self, source: &DocumentProjection) -> DocumentProjection {
        let report = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(report) = self.completion(source) {
                    return report;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("real-worker scheduler must complete the requested revision");
        assert!(
            matches!(report.outcome.as_str(), "published" | "noop"),
            "{report:?}"
        );
        assert!(report.failed_steps.is_empty(), "{report:?}");
        let projection = self.projection().await;
        assert!(
            projection
                .features
                .iter()
                .all(|feature| feature.status == FeatureStatus::Ok),
            "{projection:?}"
        );
        projection
    }

    fn completion(&self, source: &DocumentProjection) -> Option<RegenFinished> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                AppEvent::RegenFinished(report)
                    if report.runtime_session == source.runtime_session
                        && report.source_revision >= source.revision
                        && report.outcome != "superseded" =>
                {
                    Some(report.clone())
                }
                _ => None,
            })
    }

    async fn bbox(&self, body: &str) -> [[f64; 3]; 2] {
        let session = self.projection().await.runtime_session;
        let blob = api::get_mesh(
            &self.state,
            body.to_owned(),
            "fine".into(),
            None,
            Some(session),
        )
        .await
        .expect("body mesh");
        let view = validate_mesh_blob(&blob).expect("valid MESH1 blob");
        [view.bbox_min, view.bbox_max].map(|corner| corner.map(f64::from))
    }

    async fn close(self) {
        let worker = self.state.live_worker().expect("real worker adopted");
        api::close_document(&self.state, self.host.clone())
            .await
            .unwrap();
        self.state.retire_all();
        assert!(worker.wait_torn_down(Duration::from_secs(10)).await);
    }
}

fn worker_available() -> bool {
    if resolve_worker_path().is_some() {
        return true;
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    false
}

fn fixture(root: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colored_boxes.step");
    let copy = root.join("inserted.step");
    std::fs::copy(source, &copy).unwrap();
    copy
}

// ─────────────────────────────────────────────────────────────────────────────
// Existing history: a 40×20 sketch rectangle extruded 25 mm
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH: u128 = 0x5100;
const EXTRUDE: u128 = 0x5101;

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: onecad_core::math::Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: onecad_core::math::Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A fully-constrained `w`×`h` rectangle at the sketch origin.
fn rect_sketch(sid: SketchId, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(0x5200 + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(0x5240 + n));
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    for (side, start) in corners.iter().enumerate() {
        let end = corners[(side + 1) % 4];
        let side = side as u128;
        for (n, (x, y)) in [(2 * side, *start), (2 * side + 1, end)] {
            sk.add_entity(SketchEntity::point(
                e(n),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
        }
        sk.add_entity(SketchEntity::line(
            e(0x10 + side),
            e(2 * side),
            e(2 * side + 1),
            false,
        ))
        .unwrap();
    }
    for side in 0..4u128 {
        sk.add_constraint(Constraint::Coincident {
            id: c(1 + side),
            point1: e(2 * side + 1),
            point2: e((2 * side + 2) % 8),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    for (id, line) in [(5, 0x10), (6, 0x12)] {
        sk.add_constraint(Constraint::Horizontal {
            id: c(id),
            line: e(line),
        })
        .unwrap();
    }
    for (id, line) in [(7, 0x11), (8, 0x13)] {
        sk.add_constraint(Constraint::Vertical {
            id: c(id),
            line: e(line),
        })
        .unwrap();
    }
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: e(0),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: e(0),
        point1_position: CurvePosition::Arbitrary,
        point2: e(1),
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: e(2),
        point1_position: CurvePosition::Arbitrary,
        point2: e(3),
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(SKETCH)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(sketch: SketchId, distance: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(EXTRUDE)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

async fn sketch_and_extrude(h: &Harness) -> DocumentProjection {
    let sid = SketchId(Uuid::from_u128(0x51));
    let sketched = h.apply(sketch_record(&rect_sketch(sid, 40.0, 20.0))).await;
    h.settled(&sketched).await;
    let extruded = h.apply(extrude_record(sid, 25.0)).await;
    let prior = h.settled(&extruded).await;
    assert_eq!(prior.bodies.len(), 1, "sketch + extrude makes one body");
    prior
}

fn history(projection: &DocumentProjection) -> Vec<(String, String)> {
    projection
        .features
        .iter()
        .map(|feature| (feature.id.clone(), feature.op_type.clone()))
        .collect()
}

fn near(actual: [[f64; 3]; 2], expected: [[f64; 3]; 2]) {
    for (a, e) in actual.iter().flatten().zip(expected.iter().flatten()) {
        assert!((a - e).abs() < 1e-3, "bbox {actual:?} != {expected:?}");
    }
}

/// The imported bodies' boxes, sorted: the source file places a 20×30×40 box at the
/// origin and a 10 mm cube at x = 100 (millimetres, no unit scaling).
const IMPORTED: [[[f64; 3]; 2]; 2] = [
    [[0.0, 0.0, 0.0], [20.0, 30.0, 40.0]],
    [[100.0, 0.0, 0.0], [110.0, 10.0, 10.0]],
];
/// The canonical XY sketch frame maps sketch +X to world +Y and +Y to world −X.
const PRIOR: [[f64; 3]; 2] = [[-20.0, 0.0, 0.0], [0.0, 40.0, 25.0]];

/// Asserts the inserted document: prior history first, one import record last, the
/// prior body unchanged and the imported bodies at the source placement.
async fn assert_inserted(h: &Harness, prior: &DocumentProjection, current: &DocumentProjection) {
    let before = history(prior);
    let after = history(current);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert_eq!(after[..before.len()], before[..], "prior history preserved");
    assert_eq!(after[before.len()].1, "ImportStep");
    assert_eq!(current.bodies.len(), 3, "1 existing + 2 imported bodies");
    let mut imported = Vec::new();
    for body in current.bodies.keys() {
        let bbox = h.bbox(body).await;
        if prior.bodies.contains_key(body) {
            near(bbox, PRIOR);
        } else {
            imported.push(bbox);
        }
    }
    assert_eq!(imported.len(), 2, "prior body ids survive the insert");
    imported.sort_by(|a, b| a[0][0].total_cmp(&b[0][0]));
    for (actual, expected) in imported.into_iter().zip(IMPORTED) {
        near(actual, expected);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_step_keeps_history_and_survives_undo_and_reopen() {
    if !worker_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("inserted.onecad");
    let h = Harness::new(&root.path().join("first")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    h.ready().await;
    let prior = sketch_and_extrude(&h).await;
    near(h.bbox(prior.bodies.keys().next().unwrap()).await, PRIOR);

    h.pick(&fixture(root.path()));
    let inserted = api::insert_step(&h.state, h.host.clone())
        .await
        .unwrap()
        .expect("a picked file inserts");
    assert_eq!(inserted.runtime_session, prior.runtime_session);
    assert_eq!(inserted.document_id, prior.document_id);
    let current = h.settled(&inserted).await;
    assert_inserted(&h, &prior, &current).await;

    let undone = api::undo(&h.state, h.host.clone()).await.unwrap();
    let undone = h.settled(&undone).await;
    assert_eq!(history(&undone), history(&prior), "undo removes the insert");
    assert_eq!(
        undone.bodies.keys().collect::<Vec<_>>(),
        prior.bodies.keys().collect::<Vec<_>>()
    );
    let redone = api::redo(&h.state, h.host.clone()).await.unwrap();
    let redone = h.settled(&redone).await;
    assert_eq!(history(&redone), history(&current));

    api::save_document(
        &h.state,
        h.host.clone(),
        Some(saved.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    h.close().await;

    let fresh = Harness::new(&root.path().join("second")).await;
    api::open_document(
        &fresh.state,
        fresh.host.clone(),
        saved.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    fresh.ready().await;
    let reopened = fresh.settled(&fresh.projection().await).await;
    assert_eq!(reopened.document_id, prior.document_id);
    assert_eq!(history(&reopened), history(&current));
    assert_inserted(&fresh, &prior, &reopened).await;
    fresh.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_step_refuses_a_document_replaced_before_commit() {
    if !worker_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let h = Harness::new(root.path()).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    h.ready().await;
    let original = h.projection().await;
    h.pick(&fixture(root.path()));
    h.dialogs.hold.store(true, Ordering::Release);
    let (inserted, ()) = tokio::join!(api::insert_step(&h.state, h.host.clone()), async {
        h.dialogs.entered.notified().await;
        api::new_document(&h.state, h.host.clone()).await.unwrap();
        h.dialogs.release.notify_one();
    });
    match inserted {
        Err(ApiError::InvalidCommand(message)) => {
            assert!(message.contains("document changed"), "{message}")
        }
        other => panic!("a replaced document must refuse the insert, got {other:?}"),
    }
    let replacement = h.projection().await;
    assert_ne!(replacement.runtime_session, original.runtime_session);
    assert!(
        replacement.features.is_empty(),
        "the replacement receives nothing: {:?}",
        history(&replacement)
    );
    assert_eq!(replacement.undo_depth, 0);
    h.close().await;
}
