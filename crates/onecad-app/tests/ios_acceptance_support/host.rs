//! A headless application host over the real Mac sidecar (the
//! `native_plate_support` pattern): open a `.onecad` the way the app does,
//! wait for its regeneration and measure the result with kernel queries.
use super::expect::{BodyMeasure, FeatureMeasure, Measurement, SketchMeasure};
use async_trait::async_trait;
use onecad_app::{
    api,
    dto::{DocumentProjection, RegenFinished},
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use onecad_core::{document::body::BodyHealth, ids::BodyId};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

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
        panic!("the oracle passes explicit paths; no dialog is expected")
    }
}

pub struct Host {
    pub state: AppState,
    pub host: HostContext,
    events: Arc<Events>,
}

impl Host {
    pub fn start(root: &Path) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let events = Arc::new(Events::default());
        let host = HostContext {
            paths: HostPaths {
                app_data: Some(root.join("data")),
                app_config: Some(root.join("config")),
            },
            events: events.clone(),
            dialogs: Arc::new(NoDialogs),
        };
        let state = AppState::default();
        state.start(host.clone()).expect("application host starts");
        Self {
            state,
            host,
            events,
        }
    }

    pub async fn projection(&self) -> DocumentProjection {
        api::get_projection(&self.state).await.unwrap()
    }

    /// The regeneration that answers `source`'s revision (never `superseded`).
    pub async fn settle(&self, source: &DocumentProjection) -> RegenFinished {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Some(report) = self.finished(source) {
                    return report;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the real worker finishes regeneration within 60 s")
    }

    fn finished(&self, source: &DocumentProjection) -> Option<RegenFinished> {
        let events = self.events.0.lock().unwrap();
        events.iter().rev().find_map(|event| match event {
            AppEvent::RegenFinished(report)
                if report.runtime_session == source.runtime_session
                    && (report.source_revision >= source.revision
                        || report.revision >= source.revision)
                    && report.outcome != "superseded" =>
            {
                Some(report.clone())
            }
            _ => None,
        })
    }

    pub async fn close(self) {
        api::close_document(&self.state, self.host.clone())
            .await
            .unwrap();
        self.state.retire_all();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.state.retire_all();
    }
}

/// The real sidecar this oracle requires (`ONECAD_REQUIRE_WORKER=1`).
pub fn required_worker() -> PathBuf {
    assert_eq!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref(),
        Ok("1"),
        "the iPad geometry oracle runs only with ONECAD_REQUIRE_WORKER=1"
    );
    let path = PathBuf::from(
        std::env::var_os("ONECAD_WORKER_PATH").expect("ONECAD_WORKER_PATH names the Mac sidecar"),
    );
    assert!(path.is_file(), "real worker missing: {}", path.display());
    path
}

/// Opens `document` in a fresh host under `scratch`, regenerates it on macOS
/// and measures it.
pub async fn measure_file(document: &Path, scratch: &Path) -> Measurement {
    let host = Host::start(scratch);
    api::open_document(
        &host.state,
        host.host.clone(),
        document.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("{} does not open: {e}", document.display()));
    let opened = host.projection().await;
    let report = host.settle(&opened).await;
    let measurement = measure(&host, &report.outcome).await;
    host.close().await;
    measurement
}

pub async fn measure(host: &Host, outcome: &str) -> Measurement {
    let projection = host.projection().await;
    let mut bodies = Vec::new();
    for (id, body) in &projection.bodies {
        bodies.push(measure_body(host, &projection, id, body.health).await);
    }
    // Which kernel measured: the gate checks it is the Mac sidecar's OCCT.
    let hello = host.state.live_worker().and_then(|worker| worker.hello());
    Measurement {
        occt_fingerprint: hello
            .map(|hello| hello.occt.fingerprint)
            .unwrap_or_default(),
        document_id: projection.document_id.clone(),
        regen_outcome: outcome.to_string(),
        features: projection
            .features
            .iter()
            .map(|feature| FeatureMeasure {
                id: feature.id.clone(),
                op_type: feature.op_type.clone(),
                status: label(&feature.status),
            })
            .collect(),
        bodies,
        sketches: sketches(host, &projection).await,
    }
}

fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

async fn measure_body(
    host: &Host,
    projection: &DocumentProjection,
    id: &str,
    health: BodyHealth,
) -> BodyMeasure {
    let wire = onecad_app::worker::wire::body_id_wire(id.parse::<BodyId>().unwrap());
    let topology = api::query_body_topology(&host.state, wire.clone())
        .await
        .unwrap();
    let mass = api::query_mass_properties(&host.state, wire, None)
        .await
        .unwrap();
    let blob = api::get_mesh(
        &host.state,
        id.to_string(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    let (min, max) = mesh_bounds(&blob);
    BodyMeasure {
        id: id.to_string(),
        healthy: health == BodyHealth::Healthy,
        solids: topology.solid_count,
        faces: topology.face_count,
        volume: mass.volume,
        min,
        max,
    }
}

/// Vertex bounds of the checked MESH1 decode. OCW1 has no exact BRep box
/// query; on planar extreme faces the tessellation vertices are the BRep
/// vertices, so these bounds are exact up to their f32 storage.
fn mesh_bounds(blob: &[u8]) -> ([f64; 3], [f64; 3]) {
    let mesh = MeshView::decode(blob, MeshLimits::default()).expect("checked MESH1 decode");
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for index in 0..mesh.header().vertex_count as usize {
        let point = mesh.position(index).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(point[axis]));
            max[axis] = max[axis].max(f64::from(point[axis]));
        }
    }
    (min, max)
}

/// Solve state of every sketch from the Mac solver: entering a sketch solves
/// it (a reopened document's projection carries no solve), leaving keeps it.
async fn sketches(host: &Host, projection: &DocumentProjection) -> Vec<SketchMeasure> {
    let mut measured = Vec::new();
    for id in projection.sketches.keys() {
        let session = api::enter_sketch(&host.state, host.host.clone(), id.clone(), None)
            .await
            .unwrap_or_else(|e| panic!("sketch {id} does not enter: {e}"));
        api::cancel_sketch(&host.state, host.host.clone(), id.clone(), Some(false))
            .await
            .unwrap_or_else(|e| panic!("sketch {id} does not exit: {e}"));
        let count = |rows: &serde_json::Value| rows.as_array().map_or(0, Vec::len);
        let status = if session.conflicting.is_empty() {
            label(&session.status)
        } else {
            "Conflicting".into()
        };
        measured.push(SketchMeasure {
            id: id.clone(),
            status,
            dof: Some(session.dof),
            entities: count(&session.entities),
            constraints: count(&session.constraints),
        });
    }
    measured
}
