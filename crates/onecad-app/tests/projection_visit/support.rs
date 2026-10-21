//! Real-worker harness for the in-visit projection services: a dimensioned box
//! or an elliptic cylinder, a cap pick with its world anchor, and an empty XY
//! sketch to project into.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::SketchHistoryScope;
use onecad_app::export::GeometryExporter;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::body_id_wire;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, HistoryDirection, SketchEditOp};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId};
use onecad_core::math::Vec2;
use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, SketchPlane, WorldPlane,
};
use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob};

pub const EXTRUDE: u128 = 0xD01;
pub const BASE: u128 = 0xD10;
pub const PROJ: u128 = 0xD11;
/// The base rect's horizontal dimension — edited to widen the box.
pub const WIDTH_DIM: u128 = 0x2000 + 0x40 + 10;

pub struct Fixture {
    pub worker: WorkerManager,
    pub state: AppState,
    pub host: HostContext,
    pub base: SketchId,
    pub proj: SketchId,
    pub snapshot: SnapshotId,
    /// The top cap pick, with the world anchor the §7.5 ladder needs to tell it
    /// from the bottom cap after an upstream edit.
    pub source: serde_json::Value,
}

impl Fixture {
    pub fn source(&self) -> Vec<onecad_app::api::ProjectionSourceInput> {
        vec![serde_json::from_value(self.source.clone()).unwrap()]
    }

    pub fn sketch(&self, rt: &DocumentRuntime) -> Sketch {
        rt.sketch_snapshot(self.proj, "test").unwrap()
    }

    pub async fn snapshot_sketch(&self) -> Sketch {
        let guard = self.state.runtime.lock().await;
        self.sketch(guard.as_ref().unwrap())
    }

    pub async fn enter(&self) {
        onecad_app::api::enter_sketch_with_completion(
            &self.state,
            self.host.clone(),
            self.proj.to_string(),
            Some(false),
        )
        .await
        .expect("enterSketch")
        .result
        .expect("entered");
    }

    pub async fn finish(&self) -> usize {
        onecad_app::api::finish_sketch_with_completion(
            &self.state,
            self.host.clone(),
            self.proj.to_string(),
        )
        .await
        .expect("finishSketch")
        .result
        .expect("regions")
        .regions
        .len()
    }

    /// One in-visit sketch history move through the native service.
    pub async fn history(&self, direction: HistoryDirection) -> bool {
        let scope = {
            let guard = self.state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            SketchHistoryScope {
                sketch_id: self.proj,
                edit_session: rt.get_sketch(self.proj).unwrap().edit_session.unwrap(),
                runtime_session: rt.runtime_session(),
                revision: rt.revision().0,
                snapshot_id: rt.head_snapshot_id().unwrap_or(SnapshotId(0)),
            }
        };
        onecad_app::api::sketch_history_with_completion(
            &self.state,
            self.host.clone(),
            scope,
            direction,
        )
        .await
        .expect("sketch history")
        .result
        .expect("history move")
        .moved
    }

    /// Widens the box to 44 mm and republishes the head.
    pub async fn widen(&self) -> SnapshotId {
        let mut guard = self.state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(EditCommand::SketchEdit {
            sketch: self.base,
            ops: vec![SketchEditOp::SetDimension {
                constraint: ConstraintId(Uuid::from_u128(WIDTH_DIM)),
                value: Scalar::new(44.0),
            }],
        })
        .expect("widen the base rect");
        rt.finish_sketch(self.base)
            .await
            .expect("finishSketch(base)");
        published(rt).await
    }
}

/// A 40 × 20 × 25 box, or an elliptic cylinder when `ellipse` is set.
pub async fn fixture(ellipse: bool) -> Option<Fixture> {
    let bin = real_worker()?;
    let worker = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let state = app_state_over(&worker);
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    let base = SketchId(Uuid::from_u128(BASE));
    let proj = SketchId(Uuid::from_u128(PROJ));
    let profile = if ellipse {
        ellipse_sketch(base)
    } else {
        rect_sketch(base, 40.0, 20.0)
    };
    rt.apply(EditCommand::AddSketch { sketch: profile })
        .expect("AddSketch(profile)");
    rt.finish_sketch(base).await.expect("finishSketch(profile)");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(base),
        at_cursor: true,
    })
    .expect("AddOperation");
    let snapshot = published(&mut rt).await;
    let body = BodyId(Uuid::from_u128(EXTRUDE));
    let mesh = rt
        .get_mesh(body, onecad_core::regen::Lod::Coarse, None)
        .await
        .expect("body mesh");
    let top = top_face_pick(&mesh);
    let cap = ElementQuery::query_element_by_topo_key(&worker, snapshot, body, &top)
        .await
        .expect("QueryElement(top cap)")
        .expect("the picked cap resolves");
    let mut target = Sketch::new(
        proj,
        "Projected",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    target.plane = SketchPlane::xy();
    rt.apply(EditCommand::AddSketch { sketch: target })
        .expect("AddSketch(target)");
    *state.runtime.lock().await = Some(rt);
    Some(Fixture {
        worker,
        state,
        host: host(),
        base,
        proj,
        snapshot,
        source: serde_json::json!({
            "bodyId": body_id_wire(body),
            "topoKey": top,
            "anchor": { "worldPoint": cap.center },
        }),
    })
}

pub async fn published(rt: &mut DocumentRuntime) -> SnapshotId {
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "regen must publish, got {:?}",
        report.outcome
    );
    SnapshotId(report.snapshot_id)
}

pub fn point_uvs(sketch: &Sketch) -> Vec<(EntityId, [f64; 2])> {
    sketch
        .entities()
        .iter()
        .filter_map(|e| match e {
            SketchEntity::Point { id, at, .. } => Some((*id, [at.x, at.y])),
            _ => None,
        })
        .collect()
}

pub fn max_u(sketch: &Sketch) -> f64 {
    point_uvs(sketch)
        .iter()
        .map(|(_, p)| p[0])
        .fold(f64::MIN, f64::max)
}

pub fn fixed_pins(sketch: &Sketch) -> usize {
    sketch
        .constraints()
        .iter()
        .filter(|c| matches!(c, Constraint::Fixed { .. }))
        .count()
}

fn real_worker() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "ONECAD_WORKER_PATH names no worker binary");
        return Some(path);
    }
    let path = resolve_worker_path();
    assert!(
        path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    path
}

fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        let projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
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
            projection,
            step_import,
            circuit,
            readiness,
        )
    }))
}

struct Ports;
impl EventSink for Ports {
    fn emit(&self, _event: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Ports {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        panic!("projection services must not open dialogs")
    }
}

fn host() -> HostContext {
    HostContext {
        paths: HostPaths::default(),
        events: Arc::new(Ports),
        dialogs: Arc::new(Ports),
    }
}

fn extrude_record(sketch: SketchId) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(25.0),
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

/// A closed ellipse: its prism's caps are bounded by `GeomAbs_Ellipse` edges,
/// which `ProjectToSketchPlane` must refuse by name.
fn ellipse_sketch(sid: SketchId) -> Sketch {
    let (center, curve) = (EntityId(Uuid::from_u128(0x3000)), Uuid::from_u128(0x3001));
    let mut sk = Sketch::on_world_plane(sid, "Ellipse", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        center,
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::ellipse(EntityId(curve), center, 12.0, 6.0, 0.3, false).unwrap())
        .unwrap();
    sk
}

/// A fully constrained `w × h` rectangle whose `c(10)` is the width dimension.
fn rect_sketch(sid: SketchId, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(0x2000 + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(0x2000 + 0x40 + n));
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    // Side `s` runs from point 2s to point 2s+1 along line 0x10+s.
    for side in 0..4usize {
        let ends = [corners[side], corners[(side + 1) % 4]];
        for (k, (x, y)) in ends.into_iter().enumerate() {
            let at = Vec2::new_unchecked(x, y);
            let id = e((2 * side + k) as u128);
            sk.add_entity(SketchEntity::point(id, at, false, false))
                .unwrap();
        }
        let s = side as u128;
        sk.add_entity(SketchEntity::line(
            e(0x10 + s),
            e(2 * s),
            e(2 * s + 1),
            false,
        ))
        .unwrap();
    }
    let coincident = |n: u128, a: u128, b: u128| Constraint::Coincident {
        id: c(n),
        point1: e(a),
        point2: e(b),
        point1_position: CurvePosition::Arbitrary,
        point2_position: CurvePosition::Arbitrary,
    };
    let constraints = [
        coincident(1, 1, 2),
        coincident(2, 3, 4),
        coincident(3, 5, 6),
        coincident(4, 7, 0),
        Constraint::Horizontal {
            id: c(5),
            line: e(0x10),
        },
        Constraint::Horizontal {
            id: c(6),
            line: e(0x12),
        },
        Constraint::Vertical {
            id: c(7),
            line: e(0x11),
        },
        Constraint::Vertical {
            id: c(8),
            line: e(0x13),
        },
        Constraint::Fixed {
            id: c(9),
            point: e(0),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(0.0, 0.0),
        },
        Constraint::HorizontalDistance {
            id: c(10),
            point1: e(0),
            point1_position: CurvePosition::Arbitrary,
            point2: e(1),
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(w),
        },
        Constraint::VerticalDistance {
            id: c(11),
            point1: e(2),
            point1_position: CurvePosition::Arbitrary,
            point2: e(3),
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(h),
        },
    ];
    for constraint in constraints {
        sk.add_constraint(constraint).unwrap();
    }
    sk
}

/// The `TopoKey` of the face with the greatest mean vertex Z (the far cap).
fn top_face_pick(blob: &[u8]) -> String {
    let view = validate_mesh_blob(blob).expect("MESH1 validates");
    let section = |ty: u32| view.section(ty).expect("MESH1 section").offset as usize;
    let (ranges, indices, positions) = (section(4), section(3), section(1));
    let (offs, chars) = (section(5), section(6));
    let z = |vertex: usize| f32_le(blob, positions + vertex * 12 + 8) as f64;
    let mut best: Option<(usize, f64)> = None;
    for face in 0..view.face_count as usize {
        let first = u32_le(blob, ranges + face * 8) as usize;
        let count = u32_le(blob, ranges + face * 8 + 4) as usize;
        let mut sum = 0.0;
        for tri in first..first + count {
            for k in 0..3 {
                sum += z(u32_le(blob, indices + tri * 12 + k * 4) as usize);
            }
        }
        let mean = sum / (3 * count.max(1)) as f64;
        if count > 0 && best.is_none_or(|(_, top)| mean > top) {
            best = Some((face, mean));
        }
    }
    let face = best.expect("at least one face").0;
    let lo = u32_le(blob, offs + face * 4) as usize;
    let hi = u32_le(blob, offs + (face + 1) * 4) as usize;
    String::from_utf8_lossy(&blob[chars + lo..chars + hi]).into_owned()
}
