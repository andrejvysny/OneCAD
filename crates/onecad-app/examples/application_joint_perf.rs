//! Headless application joint-publication timing with independently authored logical frames.
mod application_joint_perf_support;
use application_joint_perf_support::{open, pose_report, replay_all, stable, tree};
use onecad_app::{api, dto::AssemblyProjectionDto, joint_execution, state::AppState};
use onecad_assembly::joint::JointId;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

const EXPECTED_SHA: &str = "7c39c64fa89618fcb7cf40dcb2a230b0f3f68acd2d09f4fd7c8af58be006ae9d";
const WARMUPS: usize = 5;
const SAMPLES: usize = 100;

struct Campaign<'a> {
    state: &'a AppState,
    session: &'a str,
    initial: &'a AssemblyProjectionDto,
    slider: JointId,
    output: &'a Path,
    worker_path: PathBuf,
    worker_hash: String,
    worker_hello: Value,
    replay: Vec<Value>,
    warmups: Vec<Value>,
    samples: Vec<Value>,
    refusals: Vec<Value>,
    failures: usize,
}

impl Campaign<'_> {
    fn save(&self, complete: bool, fixture_unchanged: Option<bool>, worker_after: Option<&str>) {
        let result = json!({"fixtureSha256": EXPECTED_SHA, "fixtureUnchanged": fixture_unchanged,
            "workerResolvedPath": self.worker_path, "workerSha256": self.worker_hash, "workerHello": self.worker_hello,
            "workerSha256After": worker_after, "workerUnchanged": worker_after.map(|hash| hash == self.worker_hash), "complete": complete, "replayedDefinitions": self.replay,
            "thresholds": {"linearMm": 1e-6, "angularRad": 1e-9, "linearGoalIsSolverContract": false, "angularGoalIsSolverContract": false},
            "scope": "headless application joint publication; authored unbound logical frames; no GUI, contact, fit, or performance budget",
            "warmups": WARMUPS, "samples": SAMPLES, "validationOrRequestFailures": self.failures,
            "warmupsRaw": self.warmups, "samplesRaw": self.samples, "refusals": self.refusals});
        std::fs::write(self.output, serde_json::to_vec_pretty(&result).unwrap())
            .expect("write evidence");
    }

    async fn projection(&self) -> AssemblyProjectionDto {
        api::get_assembly_projection(self.state, self.session)
            .await
            .expect("assembly projection")
    }

    async fn timed_request(
        &self,
        before: &AssemblyProjectionDto,
        value: f64,
        cancelled: bool,
    ) -> (
        f64,
        Result<joint_execution::JointSolveReport, onecad_app::error::ApiError>,
    ) {
        let joints = before.joints.clone();
        let cancel = Arc::new(AtomicBool::new(cancelled));
        let drives = vec![(self.slider, value)];
        let start = Instant::now();
        let result = joint_execution::solve_joints_at_generation_cancellable(
            self.state,
            self.session,
            before.persisted_generation,
            joints,
            drives,
            cancel,
        )
        .await;
        (start.elapsed().as_secs_f64() * 1000.0, result)
    }

    fn outcome(
        &mut self,
        i: usize,
        value: f64,
        elapsed: f64,
        before: &AssemblyProjectionDto,
        after: &AssemblyProjectionDto,
        result: Result<joint_execution::JointSolveReport, onecad_app::error::ApiError>,
    ) -> Value {
        let poses = pose_report(self.initial, after, value);
        match result {
            Ok(report) => {
                let poses_ok =
                    poses.len() == 20 && poses.iter().all(|row| row["status"] == "passed");
                let limits_ok = after
                    .joints
                    .iter()
                    .find(|joint| joint.id == self.slider)
                    .and_then(|joint| joint.limits)
                    .is_some_and(|limits| limits.min == -2.0 && limits.max == 2.0);
                let advanced = after.persisted_generation > before.persisted_generation;
                let passed = report.remaining_dof == 0 && poses_ok && limits_ok && advanced;
                if !passed {
                    self.failures += 1;
                }
                json!({"index": i, "requestedMm": value, "elapsedMs": elapsed, "status": "published", "validationStatus": if passed { "passed" } else { "failed" }, "remainingDof": report.remaining_dof, "maxResidual": report.max_residual, "poseValidation": poses, "limitsUnchanged": limits_ok, "generationAdvanced": advanced})
            }
            Err(error) => {
                self.failures += 1;
                let cancelled = matches!(&error, onecad_app::error::ApiError::StalePreview(message) if message == "joint solve cancelled");
                json!({"index": i, "requestedMm": value, "elapsedMs": elapsed, "status": if cancelled { "cancelled" } else { "failed" }, "validationStatus": "not_run", "poseValidation": poses, "stateUnchanged": stable(before) == stable(after), "error": error.to_string()})
            }
        }
    }

    async fn sample(&mut self, i: usize) {
        let before = self.projection().await;
        let value = if i.is_multiple_of(2) { 0.5 } else { -0.5 };
        let (elapsed, result) = self.timed_request(&before, value, false).await;
        let after = self.projection().await;
        let row = self.outcome(i, value, elapsed, &before, &after, result);
        if i < WARMUPS {
            self.warmups.push(row);
        } else {
            self.samples.push(row);
        }
        self.save(false, None, None);
    }

    async fn refusal(&mut self, value: f64, cancelled: bool) {
        let before = self.projection().await;
        let (elapsed, result) = self.timed_request(&before, value, cancelled).await;
        let after = self.projection().await;
        let correct = match &result {
            Err(onecad_app::error::ApiError::StalePreview(message)) if cancelled => {
                message == "joint solve cancelled"
            }
            Err(onecad_app::error::ApiError::InvalidCommand(message)) if !cancelled => {
                message.contains("invalid or coupled joint drive")
            }
            _ => false,
        };
        let unchanged = stable(&before) == stable(&after);
        if !correct || !unchanged {
            self.failures += 1;
        }
        self.refusals.push(json!({"requestedMm": value, "preCancelled": cancelled, "elapsedMs": elapsed, "error": result.err().map(|e| e.to_string()), "correctRefusal": correct, "stateUnchanged": unchanged}));
        self.save(false, None, None);
    }
}

fn sha(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(std::fs::read(path).expect("read input"))
    )
}

async fn prepare(state: &AppState, session: &str) -> (AssemblyProjectionDto, JointId, Vec<Value>) {
    let opened = api::get_assembly_projection(state, session).await.unwrap();
    let replay = replay_all(state, session, &opened).await;
    let initial = api::get_assembly_projection(state, session).await.unwrap();
    let (joints, slider) = tree(&initial);
    let report = joint_execution::solve_joints_at_generation_cancellable(
        state,
        session,
        initial.persisted_generation,
        joints,
        vec![],
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .expect("author joints");
    assert_eq!(report.remaining_dof, 1);
    let zero = api::get_assembly_projection(state, session).await.unwrap();
    assert!(
        pose_report(&initial, &zero, 0.0)
            .iter()
            .all(|row| row["status"] == "passed"),
        "zero pose"
    );
    assert_eq!(zero.joints.len(), 19);
    assert!(zero.joint_bindings.is_empty());
    (initial, slider, replay)
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: application_joint_perf FIXTURE.onecad OUTPUT.json"
    );
    let source = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .expect("create output");
    assert_eq!(sha(&source), EXPECTED_SHA);
    let worker = onecad_app::worker::resolve_worker_path().expect("no resolved worker candidate");
    let worker_hash = sha(&worker);
    let area = tempfile::tempdir().expect("isolated host directory");
    let (state, host, session) = open(&source, area.path()).await;
    let hello = state
        .live_worker()
        .expect("worker was not adopted")
        .hello()
        .expect("worker handshake unavailable");
    assert_eq!(
        sha(&worker),
        worker_hash,
        "worker candidate changed during startup"
    );
    let (initial, slider, replay) = prepare(&state, &session).await;
    let mut c = Campaign {
        state: &state,
        session: &session,
        initial: &initial,
        slider,
        output: &output,
        worker_path: worker.clone(),
        worker_hash,
        worker_hello: serde_json::to_value(hello).expect("serialize worker handshake"),
        replay,
        warmups: Vec::with_capacity(WARMUPS),
        samples: Vec::with_capacity(SAMPLES),
        refusals: Vec::new(),
        failures: 0,
    };
    c.save(false, None, None);
    for i in 0..WARMUPS + SAMPLES {
        c.sample(i).await;
    }
    c.refusal(3.0, false).await;
    c.refusal(0.5, true).await;
    api::shutdown_host(&state, host, None).await.unwrap();
    let unchanged = sha(&source) == EXPECTED_SHA;
    let worker_after = std::fs::read(&worker)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    c.save(true, Some(unchanged), worker_after.as_deref());
    assert!(unchanged, "source fixture changed");
    assert_eq!(
        worker_after.as_deref(),
        Some(c.worker_hash.as_str()),
        "resolved worker candidate missing or changed"
    );
    assert_eq!(
        c.failures, 0,
        "API request or published-result validation goal failed; raw evidence retained"
    );
}
