//! Opt-in controlled Release replay measurements; the default smoke remains one-shot.
#[path = "evidence.rs"]
mod evidence;
#[path = "statistics.rs"]
mod statistics;

use super::*;
use evidence::Result;
use onecad_app::document_runtime::SaveCaches;
use onecad_core::{
    document::component::root_definition_id,
    ids::DocumentId,
    io::container::{ContainerReader, SaveMeta},
};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

const BATCHES: usize = 3;
const WARMUPS: usize = 5;
const WARM_SAMPLES: usize = 100;
const COLD_SAMPLES: usize = 20;
const CASES: [(&str, usize, usize); 3] = [
    ("edit-step-3", 1, 3),
    ("edit-step-21", 10, 21),
    ("edit-step-39", 19, 39),
];

struct Campaign {
    out: PathBuf,
    raw: File,
    records: Vec<Value>,
    metadata: Value,
    worker: PathBuf,
    fixture: Option<String>,
}

pub(super) async fn run(out: &Path) -> Result {
    if cfg!(debug_assertions) {
        return Err("ONECAD_PERF_OUT requires cargo test --release".into());
    }
    if std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1") {
        return Err("measurement requires ONECAD_REQUIRE_WORKER=1".into());
    }
    let mut campaign = Campaign::new(out)?;
    let result = campaign.execute().await;
    campaign.finish(&result)?;
    result
}

impl Campaign {
    fn new(out: &Path) -> Result<Self> {
        let requested = std::path::absolute(out)?;
        let out = fs::canonicalize(requested.parent().ok_or("output parent required")?)?
            .join(requested.file_name().ok_or("output name required")?);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        if out.exists() || out.starts_with(root) {
            return Err("measurement output must be new and outside repository".into());
        }
        fs::create_dir(&out)?;
        let worker = real_worker().ok_or("real worker required")?;
        let config = json!({"batches":BATCHES,"warmupsPerCasePerBatch":WARMUPS,"warmSamplesPerCasePerBatch":WARM_SAMPLES,
            "coldSamplesPerBatch":COLD_SAMPLES,"cases":CASES,"cold":"fresh worker and runtime; OS caches not cleared; startup excluded from replay wall time",
            "warm":"same worker/runtime for all warm samples in a batch; alternating pocket depths 9 and 10 mm",
            "fixture":"60x60x30 block and nineteen 6x6 blind pockets; forty feature history",
            "measured":"replay wall time and production planner/worker/mesh integer-ms clocks; boundary RSS",
            "unmeasured":["input","preview admission","bound-origin resolution","scene capture/upload","GPU completion","presentation","peak memory","GPU memory"]});
        let metadata = json!({"schemaVersion":1,"status":"incomplete","configSha256":evidence::hash(&serde_json::to_vec(&config)?),
            "config":config,"source":evidence::source()?,"conditionsStart":evidence::conditions(),
            "workerPath":worker,"workerSha256":evidence::hash(&fs::read(&worker)?),
            "harnessSha256":evidence::hash(include_bytes!("../perf_baseline.rs")),"profile":"release",
            "testExecutable":std::env::current_exe()?,"testExecutableSha256":evidence::hash(&fs::read(std::env::current_exe()?)?),
            "commandLine":std::env::args_os().map(|arg|arg.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "selectedBuildEnvironment":evidence::build_environment(),
            "hostKind":"backend test-host process; not native desktop UI"});
        fs::write(
            out.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;
        let raw = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out.join("samples.jsonl"))?;
        Ok(Self {
            out,
            raw,
            records: Vec::new(),
            metadata,
            worker,
            fixture: None,
        })
    }

    fn append(&mut self, record: Value) -> Result {
        serde_json::to_writer(&mut self.raw, &record)?;
        writeln!(&mut self.raw)?;
        self.raw.sync_data()?;
        self.records.push(record);
        Ok(())
    }

    async fn execute(&mut self) -> Result {
        for batch in 0..BATCHES {
            self.append(json!({"record":"batch-conditions","batch":batch,"conditions":evidence::conditions()}))?;
            self.warm(batch).await?;
            for index in 0..COLD_SAMPLES {
                self.cold(batch, index).await?;
            }
        }
        Ok(())
    }

    async fn fresh(
        &mut self,
        batch: usize,
        name: &str,
    ) -> Result<(WorkerManager, DocumentRuntime, Value)> {
        self.append(json!({"record":"worker-start","batch":batch,"case":name}))?;
        let start = Instant::now();
        let wm = WorkerManager::spawn(SupervisorConfig::production(self.worker.clone()));
        if !wm.wait_ready(Duration::from_secs(10)).await {
            wm.retire();
            wm.wait_torn_down(Duration::from_secs(10)).await;
            return Err("measurement worker did not become ready".into());
        }
        let mut rt = runtime_over(&wm);
        let features = build_part(&mut rt);
        let startup = json!({"startupMs":start.elapsed().as_secs_f64()*1000.0,"features":features,
            "runtimeSession":rt.runtime_session(),"workerEpoch":wm.epoch().0,"workerHello":format!("{:?}",wm.hello())});
        if let Err(error) = self.capture_fixture(&mut rt, &format!("{batch}-{name}")) {
            Self::retire(&wm).await?;
            return Err(error);
        }
        Ok((wm, rt, startup))
    }

    fn capture_fixture(&mut self, rt: &mut DocumentRuntime, name: &str) -> Result {
        let path = self.out.join(format!("fixture-{name}.onecad"));
        let meta = SaveMeta {
            app_version: "perf40-measurement".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        };
        let payload = rt.build_save_payload(meta, SaveCaches::none());
        DocumentRuntime::write_payload(&path, &payload)?;
        let loaded = ContainerReader::open(&path)?;
        let mut doc = loaded.document().clone();
        doc.id = DocumentId(Uuid::nil());
        doc.root_definition_id = root_definition_id(doc.id);
        let normalized = evidence::hash(&serde_json::to_vec(&doc)?);
        if self
            .fixture
            .as_ref()
            .is_some_and(|expected| *expected != normalized)
        {
            return Err("generated fixture changed between matched samples".into());
        }
        self.fixture = Some(normalized.clone());
        self.append(json!({"record":"fixture","path":path.file_name().unwrap().to_string_lossy(),
            "sha256":evidence::hash(&fs::read(&path)?),"normalizedFixtureSha256":normalized,
            "normalization":"document and derived root-definition IDs only; captured file preserves original IDs"}))
    }

    async fn retire(wm: &WorkerManager) -> Result {
        wm.retire();
        if !wm.wait_torn_down(Duration::from_secs(10)).await {
            return Err("measurement worker teardown incomplete".into());
        }
        Ok(())
    }

    async fn warm(&mut self, batch: usize) -> Result {
        let (wm, mut rt, startup) = self.fresh(batch, "warm").await?;
        let result = self.warm_samples(batch, &wm, &mut rt, &startup).await;
        let teardown = Self::retire(&wm).await;
        self.append(json!({"record":"worker-stop","batch":batch,"runtimeSession":rt.runtime_session(),"tornDown":teardown.is_ok(),"failure":teardown.as_ref().err().map(ToString::to_string)}))?;
        result.and(teardown)
    }

    async fn warm_samples(
        &mut self,
        batch: usize,
        wm: &WorkerManager,
        rt: &mut DocumentRuntime,
        startup: &Value,
    ) -> Result {
        self.sample(batch, "bootstrap", 0, "cold-replay", 0, wm, rt, startup)
            .await?;
        for index in 0..WARMUPS + WARM_SAMPLES {
            let phase = if index < WARMUPS { "warmup" } else { "warm" };
            for (case, pocket, from) in CASES {
                edit_pocket_depth(rt, pocket, if index % 2 == 0 { 9.0 } else { 10.0 });
                self.sample(batch, phase, index, case, from, wm, rt, startup)
                    .await?;
            }
        }
        Ok(())
    }

    async fn cold(&mut self, batch: usize, index: usize) -> Result {
        let (wm, mut rt, startup) = self.fresh(batch, &format!("cold-{index}")).await?;
        let result = self
            .sample(
                batch,
                "cold",
                index,
                "cold-replay",
                0,
                &wm,
                &mut rt,
                &startup,
            )
            .await;
        let teardown = Self::retire(&wm).await;
        self.append(json!({"record":"worker-stop","batch":batch,"runtimeSession":rt.runtime_session(),"tornDown":teardown.is_ok(),"failure":teardown.as_ref().err().map(ToString::to_string)}))?;
        result.and(teardown)
    }

    #[allow(clippy::too_many_arguments)]
    async fn sample(
        &mut self,
        batch: usize,
        phase: &str,
        index: usize,
        case: &str,
        from: usize,
        wm: &WorkerManager,
        rt: &mut DocumentRuntime,
        startup: &Value,
    ) -> Result {
        let epoch = wm.epoch().0;
        let rss_before = evidence::rss(&self.worker);
        self.append(
            json!({"record":"started","batch":batch,"phase":phase,"sample":index,"case":case,
            "runtimeSession":rt.runtime_session(),"workerEpoch":epoch}),
        )?;
        let token = CancelToken::new();
        let clock = Instant::now();
        let report = rt.run_regen(RegenRequest::ToEnd { from }, token).await;
        let wall = clock.elapsed().as_secs_f64() * 1000.0;
        let rss_after = evidence::rss(&self.worker);
        let mut record = json!({"record":"result","batch":batch,"phase":phase,"sample":index,"case":case,
            "runtimeSession":rt.runtime_session(),"workerEpochBefore":epoch,"workerEpochAfter":wm.epoch().0,
            "wallMs":wall,"plannerMs":report.timings.planner_ms,"workerMs":report.timings.worker_ms,"meshMs":report.timings.mesh_ms,
            "outcome":report.outcome_str(),"failedSteps":format!("{:?}",report.failed_steps),"failure":report.failure_message(),
            "diagnostics":format!("{:?}",report.diagnostics),"needsRepair":format!("{:?}",report.needs_repair),
            "rssBefore":rss_before,"rssAfter":rss_after,"startup":startup,"normalizedFixtureSha256":self.fixture});
        let check = validate(
            &report,
            wall,
            startup["workerEpoch"].as_u64().unwrap(),
            wm.epoch().0,
        );
        let geometry = if check.is_ok() {
            checked_triangles(rt).await
        } else {
            Ok(0)
        };
        record["triangles"] = json!(geometry.as_ref().ok());
        record["accepted"] = json!(check.is_ok() && geometry.is_ok());
        record["validationError"] = json!(check
            .as_ref()
            .err()
            .or(geometry.as_ref().err())
            .map(ToString::to_string));
        self.append(record)?;
        check?;
        geometry?;
        Ok(())
    }

    fn finish(&mut self, result: &Result) -> Result {
        let source_end = evidence::source()?;
        let worker_end = evidence::hash(&fs::read(&self.worker)?);
        let executable_end = evidence::hash(&fs::read(std::env::current_exe()?)?);
        let source_stable = source_end == self.metadata["source"];
        let worker_stable = self.metadata["workerSha256"] == worker_end;
        let executable_stable = self.metadata["testExecutableSha256"] == executable_end;
        let stable = source_stable && worker_stable && executable_stable;
        self.metadata["testExecutableSha256End"] = json!(executable_end);
        self.metadata["workerSha256End"] = json!(worker_end);
        self.metadata["conditionsEnd"] = evidence::conditions();
        self.metadata["sourceEnd"] = source_end;
        self.metadata["normalizedFixtureSha256"] = json!(self.fixture);
        self.metadata["status"] = json!(if result.is_ok() && stable {
            "complete"
        } else {
            "incomplete"
        });
        self.metadata["failure"] = json!(result.as_ref().err().map(ToString::to_string));
        self.metadata["sourceUnchanged"] = json!(source_stable);
        self.metadata["workerUnchanged"] = json!(worker_stable);
        self.metadata["testExecutableUnchanged"] = json!(executable_stable);
        fs::write(
            self.out.join("metadata.json"),
            serde_json::to_vec_pretty(&self.metadata)?,
        )?;
        fs::write(
            self.out.join("summary.json"),
            serde_json::to_vec_pretty(&json!({"status":self.metadata["status"],
            "classification":"measured replay phase distributions and boundary RSS; no UI/GPU/presentation or optimization claim",
            "groups":statistics::summarize(&self.records),"rawRecordCount":self.records.len(),
            "rawSamplesSha256":evidence::hash(&fs::read(self.out.join("samples.jsonl"))?)}))?,
        )?;
        if !stable {
            return Err(
                "source, worker, or test executable changed during performance campaign; measurements remain incomplete".into(),
            );
        }
        Ok(())
    }
}

fn validate(report: &RegenReport, wall: f64, epoch: u64, after: u64) -> Result {
    if !report.published()
        || !report.failed_steps.is_empty()
        || !report.needs_repair.is_empty()
        || epoch != after
    {
        return Err("regen failed, cancelled, superseded, or restarted its worker".into());
    }
    let split = report.timings.planner_ms + report.timings.worker_ms + report.timings.mesh_ms;
    if !wall.is_finite() || split as f64 > wall + 2.0 || report.timings.worker_ms == 0 {
        return Err("replay phase clocks are inconsistent with wall time".into());
    }
    Ok(())
}

async fn checked_triangles(rt: &mut DocumentRuntime) -> Result<u64> {
    let mut total = 0;
    for body in rt.head_body_ids() {
        let blob = rt
            .get_mesh(body, Lod::Fine, None)
            .await
            .ok_or("published body has no display mesh")?;
        total += u64::from(validate_mesh_blob(&blob)?.triangle_count);
    }
    if total == 0 {
        return Err("published geometry contains no triangles".into());
    }
    Ok(total)
}
