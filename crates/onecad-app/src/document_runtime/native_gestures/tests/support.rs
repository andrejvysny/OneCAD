use super::*;
use crate::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use crate::worker::{NativeGestureWorker, PendingBackend};
use async_trait::async_trait;
use std::sync::{atomic::AtomicBool, Mutex as StdMutex};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Backend(pub Arc<BackendState>);
pub struct BackendState {
    pub cache: StdMutex<Option<Sketch>>,
    pub gestures: StdMutex<HashMap<u64, (EntityId, [f64; 2])>>,
    pub targets: StdMutex<Vec<[f64; 2]>>,
    pub calls: StdMutex<Vec<&'static str>>,
    pub end_positions: StdMutex<Option<BTreeMap<String, [f64; 2]>>>,
    pub update_positions: StdMutex<std::collections::VecDeque<BTreeMap<String, [f64; 2]>>>,
    pub cancel_ok: AtomicBool,
    pub gate: StdMutex<Option<&'static str>>,
    pub entered: Semaphore,
    pub release: Semaphore,
    pub retired: AtomicBool,
    pub quarantine_ok: AtomicBool,
    revision: AtomicU64,
}

impl Backend {
    pub fn new() -> Self {
        Self(Arc::new(BackendState {
            cache: StdMutex::new(None),
            gestures: StdMutex::new(HashMap::new()),
            targets: StdMutex::new(Vec::new()),
            calls: StdMutex::new(Vec::new()),
            end_positions: StdMutex::new(None),
            update_positions: StdMutex::new(Default::default()),
            cancel_ok: AtomicBool::new(true),
            gate: StdMutex::new(None),
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            retired: AtomicBool::new(false),
            quarantine_ok: AtomicBool::new(true),
            revision: AtomicU64::new(0),
        }))
    }

    pub async fn step(&self, verb: &'static str) {
        self.0.calls.lock().unwrap().push(verb);
        let gated = *self.0.gate.lock().unwrap() == Some(verb);
        if gated {
            self.0.entered.add_permits(1);
            self.0.release.acquire().await.unwrap().forget();
        }
    }

    pub async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.0.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }

    pub fn resume(&self) {
        *self.0.gate.lock().unwrap() = None;
        self.0.release.add_permits(1);
    }

    fn solve(&self, sketch: &Sketch, positions: BTreeMap<String, [f64; 2]>) -> SketchUpsertDto {
        SketchUpsertDto {
            sketch_id: sketch.id.to_string(),
            sketch_revision: self.0.revision.fetch_add(1, Ordering::Relaxed) + 1,
            dof: 2,
            status: SketchSolveStatus::UnderConstrained,
            truth: Default::default(),
            conflicting: vec![],
            solved_positions: positions,
            solved_curves: Default::default(),
            entity_states: Default::default(),
        }
    }
}

#[async_trait]
impl SolverEngine for Backend {
    fn bind_native_gesture(&self) -> Result<Arc<dyn NativeGestureWorker>, EngineError> {
        Ok(Arc::new(self.clone()))
    }
    async fn sketch_upsert(&self, sketch: &Sketch) -> Result<SketchUpsertDto, EngineError> {
        self.step("upsert").await;
        *self.0.cache.lock().unwrap() = Some(sketch.clone());
        Ok(self.solve(sketch, Default::default()))
    }
    async fn begin_gesture(
        &self,
        _sketch: &str,
        _revision: u64,
        id: u64,
        point: EntityId,
        _target: wire::GestureTarget,
        _policy: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        // Scratch exists before the delayed reply, reproducing cancellation during pending Begin.
        self.0
            .gestures
            .lock()
            .unwrap()
            .insert(id, (point, [0.0, 0.0]));
        self.step("begin").await;
        Ok(BeginGestureDto {
            gesture_id: id,
            ready: true,
            point_offset: None,
            positions: Default::default(),
            curves: Default::default(),
            entity_states: Default::default(),
        })
    }
    async fn solve_drag(
        &self,
        id: u64,
        seq: u64,
        point: EntityId,
        target: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        self.step("update").await;
        self.0.targets.lock().unwrap().push(target);
        self.0.gestures.lock().unwrap().get_mut(&id).unwrap().1 = target;
        Ok(DragSolveDto {
            gesture_id: id,
            seq,
            status: "success".into(),
            dof: 2,
            conflicting: vec![],
            positions: self
                .0
                .update_positions
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| [(point.to_string(), target)].into()),
            curves: Default::default(),
            solve_micros: 1,
            superseded: false,
            diagnostics: None,
        })
    }
    async fn end_gesture(
        &self,
        _sketch: &str,
        id: u64,
        target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        let (point, previous) = self.0.gestures.lock().unwrap().remove(&id).unwrap();
        let target = target.unwrap_or(previous);
        let sketch = self.0.cache.lock().unwrap().clone().unwrap();
        let positions = self
            .0
            .end_positions
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| [(point.to_string(), target)].into());
        let mut ahead = sketch.clone();
        ahead.apply_solved_positions(&typed_positions(&positions));
        *self.0.cache.lock().unwrap() = Some(ahead);
        // Worker End has already written its store before Rust can fence this reply.
        self.step("end").await;
        Ok(self.solve(&sketch, positions))
    }
    async fn cancel_gesture(&self, id: u64) -> Result<(), EngineError> {
        self.step("cancel").await;
        if !self.0.cancel_ok.load(Ordering::SeqCst) {
            return Err(op_failed("Scripted cancel failure"));
        }
        self.0.gestures.lock().unwrap().remove(&id);
        Ok(())
    }
    async fn sketch_regions(&self, _sketch: &str) -> Result<FinishSketchDto, EngineError> {
        Err(op_failed("unused"))
    }
}

#[async_trait]
impl NativeGestureWorker for Backend {
    async fn gesture_worker_head(&self) -> Result<onecad_core::regen::WorkerHead, EngineError> {
        Ok(onecad_core::regen::WorkerHead {
            document_revision: DocumentRevision(0),
            worker_epoch: WorkerEpoch(1),
            snapshot_id: SnapshotId(0),
            history_prefix_hash: onecad_core::regen::HistoryPrefixHash::empty(),
            has_scratch: false,
            has_restored_base: false,
            inflight: None,
        })
    }

    fn is_current(&self) -> bool {
        !self.0.retired.load(Ordering::SeqCst)
    }
    async fn quarantine(&self) -> bool {
        self.0.calls.lock().unwrap().push("quarantine");
        self.0.quarantine_ok.load(Ordering::SeqCst)
    }
}

struct Ports;
impl EventSink for Ports {
    fn emit(&self, _event: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Ports {
    async fn pick(&self, _request: FileDialog) -> Vec<String> {
        vec![]
    }
}
pub fn host() -> HostContext {
    HostContext {
        paths: HostPaths::default(),
        events: Arc::new(Ports),
        dialogs: Arc::new(Ports),
    }
}

pub async fn setup() -> (Arc<AppState>, Backend, NativeGestureScope, EntityId) {
    let backend = Backend::new();
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(PendingBackend),
        Arc::new(PendingBackend),
        Arc::new(backend.clone()),
    );
    let id = SketchId(Uuid::new_v4());
    let point = EntityId(Uuid::new_v4());
    let mut sketch = Sketch::on_world_plane(id, "Native gesture", WorldPlane::XY);
    sketch
        .add_entity(onecad_core::sketch::SketchEntity::point(
            point,
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime.enter_sketch(id).await.unwrap();
    runtime.dirty = false;
    let scope = runtime
        .native_gesture_scope(
            id,
            NativeGestureSource {
                capture: 4,
                generation: 8,
            },
        )
        .unwrap();
    let state = Arc::new(AppState::default());
    *state.runtime.lock().await = Some(runtime);
    (state, backend, scope, point)
}

pub async fn settled(state: &AppState) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.native_gestures.lock().unwrap().is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

pub async fn point_at(state: &AppState, sketch: SketchId, point: EntityId) -> [f64; 2] {
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    let sketch = runtime.sketch_snapshot(sketch, "test").unwrap();
    match sketch.get_entity(point).unwrap() {
        onecad_core::sketch::SketchEntity::Point { at, .. } => [at.x, at.y],
        _ => panic!("point expected"),
    }
}
