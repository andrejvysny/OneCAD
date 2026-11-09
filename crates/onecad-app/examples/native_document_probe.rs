//! Cold, headless native-document replay diagnostic; no imported substitute geometry.
//!
//! Usage: `native_document_probe INPUT.onecad OUTPUT.json [--observe-late]`.
//! The optional flag observes a timed-out replay for up to 30 more seconds. Its
//! result is diagnostic only and cannot change the original timeout or exit code.
use async_trait::async_trait;
use onecad_app::{
    api,
    dto::{DocumentProjection, FeatureStatus, RegenFinished},
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const WORKER_SHA: &str = "4272493274b789539a0e731afe0d45084e344ccf646f46bec35af6561a3fcfb0";
const BUDGET: Duration = Duration::from_secs(30);

struct EventLog {
    rows: VecDeque<Value>,
    terminals: VecDeque<RegenFinished>,
    dropped: usize,
    terminals_dropped: usize,
}
struct Events {
    started: Instant,
    log: Mutex<EventLog>,
}
impl Default for Events {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            log: Mutex::new(EventLog {
                rows: VecDeque::new(),
                terminals: VecDeque::new(),
                dropped: 0,
                terminals_dropped: 0,
            }),
        }
    }
}
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        let (kind, detail) = match &event {
            AppEvent::ProjectionUpdated(p) => (
                "projectionUpdated",
                json!({"session":p.runtime_session,"revision":p.revision,"featureCount":p.features.len(),"bodyCount":p.bodies.len()}),
            ),
            AppEvent::DocumentChanged(e) => ("documentChanged", json!(format!("{e:?}"))),
            AppEvent::RegenStarted(e) => ("regenStarted", json!(e)),
            AppEvent::RegenFinished(e) => ("regenFinished", json!(e)),
            AppEvent::NeedsRepair(e) => ("needsRepair", json!(format!("{e:?}"))),
            AppEvent::SketchSolved(e) => ("sketchSolved", json!(format!("{e:?}"))),
            AppEvent::WorkerStatus(e) => ("workerStatus", json!(format!("{e:?}"))),
            AppEvent::Autosave(e) => ("autosave", json!(format!("{e:?}"))),
        };
        let serialized = detail.to_string();
        let detail = if serialized.len() > 8192 {
            json!({"truncated":true,"bytes":serialized.len()})
        } else {
            detail
        };
        let mut log = self.log.lock().map_err(|e| e.to_string())?;
        if let AppEvent::RegenFinished(report) = event {
            if log.terminals.len() == 64 {
                log.terminals.pop_front();
                log.terminals_dropped += 1;
            }
            log.terminals.push_back(report);
        }
        if log.rows.len() == 1024 {
            log.rows.pop_front();
            log.dropped += 1;
        }
        log.rows.push_back(json!({"elapsedMs":self.started.elapsed().as_secs_f64()*1000.0,"kind":kind,"detail":detail}));
        Ok(())
    }
}
struct NoDialogs;
#[async_trait]
impl DialogPort for NoDialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}
fn hash(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|e| e.to_string())
}
fn host(root: &Path, events: Arc<Events>) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events,
        dialogs: Arc::new(NoDialogs),
    }
}
fn completion(events: &Events, projection: &DocumentProjection) -> Option<RegenFinished> {
    events
        .log
        .lock()
        .ok()?
        .terminals
        .iter()
        .rev()
        .find(|report| {
            report.runtime_session == projection.runtime_session
                && report.source_revision == projection.revision
                && report.outcome != "superseded"
        })
        .cloned()
}
async fn terminal(
    events: &Events,
    projection: &DocumentProjection,
    deadline: tokio::time::Instant,
) -> Result<RegenFinished, String> {
    tokio::time::timeout_at(deadline, async {
        loop {
            if let Some(report) = completion(events, projection) {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "cold deadline expired awaiting terminal".into())
}
async fn masses(state: &AppState, p: &DocumentProjection) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for id in p.bodies.keys() {
        let body = id
            .parse()
            .map_err(|e| format!("invalid body ID {id}: {e}"))?;
        let mass =
            api::query_mass_properties(state, onecad_app::worker::wire::body_id_wire(body), None)
                .await
                .map_err(|e| format!("mass for {id}: {e}"))?;
        result.push(json!({"bodyId": id, "mass": mass}));
    }
    Ok(result)
}
async fn latest_projection(state: &AppState) -> Result<DocumentProjection, String> {
    tokio::time::timeout(Duration::from_secs(5), api::get_projection(state))
        .await
        .map_err(|_| "latest projection timeout".to_string())?
        .map_err(|e| e.to_string())
}
async fn after_terminal(
    state: &AppState,
    initial: &DocumentProjection,
    report: RegenFinished,
    elapsed_ms: f64,
) -> Value {
    let latest = match latest_projection(state).await {
        Ok(p) => p,
        Err(e) => {
            return json!({"status":"projection_error", "error":e,"terminal":report,"elapsedMs":elapsed_ms})
        }
    };
    let valid = latest.runtime_session == initial.runtime_session
        && latest.revision == initial.revision
        && latest
            .features
            .iter()
            .all(|f| f.status == FeatureStatus::Ok);
    let published = report.outcome == "published"
        && report.failed_steps.is_empty()
        && report.repair_steps.is_empty();
    if !published || !valid {
        return json!({"status":"terminal_or_projection_failure", "terminal":report, "projectionValid":valid,
            "featureCount":latest.features.len(), "bodyCount":latest.bodies.len(),"elapsedMs":elapsed_ms});
    }
    match tokio::time::timeout(Duration::from_secs(5), masses(state, &latest)).await {
        Ok(Ok(mass)) => {
            json!({"status":"published", "terminal":report, "featureCount":latest.features.len(),"bodyCount":latest.bodies.len(),"mass":mass,"elapsedMs":elapsed_ms})
        }
        Ok(Err(e)) => {
            json!({"status":"mass_failure","error":e,"terminal":report,"elapsedMs":elapsed_ms})
        }
        Err(_) => json!({"status":"mass_timeout","terminal":report,"elapsedMs":elapsed_ms}),
    }
}
async fn run(
    state: &AppState,
    host: &HostContext,
    events: &Events,
    input: &Path,
    observe_late: bool,
) -> (Value, Option<Value>) {
    let start = Instant::now();
    let deadline = tokio::time::Instant::now() + BUDGET;
    let open = tokio::time::timeout_at(
        deadline,
        api::open_document(
            state,
            host.clone(),
            input.to_string_lossy().into_owned(),
            None,
        ),
    )
    .await;
    match open {
        Err(_) => {
            return (
                json!({"status":"open_timeout","elapsedMs":start.elapsed().as_secs_f64()*1000.0}),
                None,
            )
        }
        Ok(Err(e)) => {
            return (
                json!({"status":"open_error","error":e.to_string(),"elapsedMs":start.elapsed().as_secs_f64()*1000.0}),
                None,
            )
        }
        Ok(Ok(_)) => {}
    }
    let initial = match tokio::time::timeout_at(deadline, api::get_projection(state)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return (
                json!({"status":"projection_error","error":e.to_string()}),
                None,
            )
        }
        Err(_) => return (json!({"status":"projection_timeout"}), None),
    };
    match terminal(events, &initial, deadline).await {
        Ok(report) => (
            after_terminal(
                state,
                &initial,
                report,
                start.elapsed().as_secs_f64() * 1000.0,
            )
            .await,
            None,
        ),
        Err(error) => {
            let result = json!({"status":"terminal_timeout","error":error,"elapsedMs":start.elapsed().as_secs_f64()*1000.0,
                "initialSession":initial.runtime_session,"sourceRevision":initial.revision,
                "initialFeatureCount":initial.features.len(),"initialBodyCount":initial.bodies.len()});
            let late = if observe_late {
                Some(observe_terminal(state, events, &initial, deadline + BUDGET, start).await)
            } else {
                None
            };
            (result, late)
        }
    }
}
async fn observe_terminal(
    state: &AppState,
    events: &Events,
    initial: &DocumentProjection,
    deadline: tokio::time::Instant,
    start: Instant,
) -> Value {
    match terminal(events, initial, deadline).await {
        Ok(report) => {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            json!({"scope":"diagnostic only; not acceptance", "terminalElapsedMs":elapsed_ms,
                "result":after_terminal(state, initial, report, elapsed_ms).await})
        }
        Err(error) => json!({"scope":"diagnostic only; not acceptance", "status":"late_timeout",
            "error":error,"elapsedMs":start.elapsed().as_secs_f64()*1000.0}),
    }
}

async fn execute(
    input: &Path,
    output: &Path,
    file: &mut std::fs::File,
    observe_late: bool,
) -> Result<bool, String> {
    if std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1") {
        return Err("ONECAD_REQUIRE_WORKER=1 required".into());
    }
    let requested_worker = std::env::var_os("ONECAD_WORKER_PATH").map(PathBuf::from);
    let worker_path = onecad_app::worker::resolve_worker_path()
        .ok_or("production worker resolver found no worker")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let worker_hash = hash(&worker_path)?;
    if worker_hash != WORKER_SHA {
        return Err("worker SHA-256 mismatch".into());
    }
    let before = hash(input)?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    for name in ["tmp", "data/library", "logs"] {
        std::fs::create_dir_all(temp.path().join(name)).map_err(|e| e.to_string())?;
    }
    std::env::set_var("TMPDIR", temp.path().join("tmp"));
    std::env::set_var("ONECAD_LIBRARY_ROOT", temp.path().join("data/library"));
    let log_dir = output
        .parent()
        .ok_or("output parent required")?
        .join("logs");
    if log_dir.exists() {
        return Err("log directory must be fresh".into());
    }
    std::fs::create_dir(&log_dir).map_err(|e| e.to_string())?;
    std::env::set_var("ONECAD_LOG_DIR", &log_dir);
    std::env::set_var("RUST_LOG", "info,onecad_app::document_runtime=debug,onecad_lib::document_runtime=debug,onecad_protocol::frames=debug,worker=debug");
    let log_guard = onecad_app::logging::init().ok_or("logging initialization failed")?;
    let events = Arc::new(Events::default());
    let host = host(temp.path(), events.clone());
    let state = AppState::default();
    let (result, late_observation) = match state.start(host.clone()) {
        Ok(()) => run(&state, &host, &events, input, observe_late).await,
        Err(e) => (
            json!({"status":"startup_error", "error":e.to_string()}),
            None,
        ),
    };
    let worker = state.live_worker();
    let close =
        tokio::time::timeout(Duration::from_secs(10), api::close_document(&state, host)).await;
    state.retire_all();
    let torn_down = match worker {
        Some(w) => Some(w.wait_torn_down(Duration::from_secs(10)).await),
        None => None,
    };
    let after = hash(input)?;
    let worker_hash_after = hash(&worker_path)?;
    let success = result["status"] == "published"
        && before == after
        && worker_hash == worker_hash_after
        && torn_down == Some(true)
        && matches!(close, Ok(Ok(())));
    let event_log = events.log.lock().map_err(|e| e.to_string())?;
    let event_rows: Vec<_> = event_log.rows.iter().cloned().collect();
    let events_dropped = event_log.dropped;
    let terminals_dropped = event_log.terminals_dropped;
    drop(event_log);
    drop(log_guard);
    let evidence = json!({"events":event_rows, "eventsDropped":events_dropped, "terminalsDropped":terminals_dropped, "logDirectory":log_dir, "input":input, "inputSha256Before":before, "inputSha256After":after,
        "requestedWorkerPath":requested_worker, "workerPath":worker_path, "workerSha256":worker_hash,
        "workerSha256After":worker_hash_after, "terminalBudgetSeconds":30,
        "logLimits":"Event retention is bounded; temporary event serialization and runtime trace bytes are not size-capped.",
        "result":result, "lateObservation":late_observation, "close":format!("{close:?}"), "workerTornDown":torn_down, "success":success});
    serde_json::to_writer_pretty(&mut *file, &evidence).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(success)
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if !(args.len() == 3 || (args.len() == 4 && args[3] == "--observe-late")) {
        eprintln!("usage: native_document_probe INPUT.onecad OUTPUT.json [--observe-late]");
        std::process::exit(2);
    }
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(&args[2]))
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("cannot exclusively create output: {error}");
            std::process::exit(1);
        }
    };
    match execute(
        Path::new(&args[1]),
        Path::new(&args[2]),
        &mut output,
        args.len() == 4,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            let _ = serde_json::to_writer_pretty(
                &mut output,
                &json!({"status":"setup_error","error":error}),
            );
            let _ = output.flush();
            eprintln!("probe error: {error}");
            std::process::exit(1);
        }
    }
}
