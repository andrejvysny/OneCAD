//! (b) Crash journal. In-process, a worker crash (SIGSEGV inside OCCT) kills the
//! HOST app, so the existing in-memory poison circuit (crates/onecad-app/src/worker/
//! manager.rs `record_crash` / `plan_op_keys`) dies with it and a reopen would
//! crash-loop. Keys use the manager's scheme: `prefixHash | opId | fingerprint`.
//!
//! The cursor is written WORKER-side: the ABI 3 journal callback runs on the
//! worker's kernel thread before each op starts, so the record is on disk before
//! the faulting code runs. The event carries the op's own prefix hash (read
//! from the job's `prefixHashes`) and the hello fingerprint, so the record holds
//! the finished crash-circuit key and the host arms nothing per plan. A host-side cursor (advanced on received `planStep`
//! frames) lags: frames still in the socket buffer or not yet processed by the
//! host die with the process. The drill runs both and compares them.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onecad_protocol::client::WorkerEvent;
use onecad_protocol::messages::Lane;
use serde_json::{json, Value};

use crate::inproc::{InProcessWorker, JournalEventView, LogSink, LOG_INFO};
use crate::scenario::{extrude_op, model_round, open_session, plan_args, sketch_op};

const JOB_BEGIN: i32 = 0;
const PLAN_OP_BEGIN: i32 = 1;
const JOB_END: i32 = 2;

/// Stand-in for the Rust planner's prefix hash (Rust is the hash authority in
/// the real app): 64 hex chars over ops[0..=i].
fn prefix_hashes(ops: &[Value]) -> Vec<String> {
    (0..ops.len())
        .map(|i| {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in serde_json::to_vec(&ops[..=i]).unwrap() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x100000001b3);
            }
            format!("{h:016x}").repeat(4)
        })
        .collect()
}

pub fn plan_keys(ops: &[Value], fingerprint: &str) -> Vec<String> {
    prefix_hashes(ops)
        .iter()
        .zip(ops)
        .map(|(p, op)| format!("{p}|{}|{fingerprint}", op["opId"].as_str().unwrap()))
        .collect()
}

/// tmp + fsync + rename + dir fsync: the record survives a SIGKILL or power loss.
pub fn write_durable(path: &Path, value: &Value) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()
}

/// tmp + rename, no fsync: survives process death (page cache), not power loss.
pub fn write_fast(path: &Path, value: &Value) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(value)?)?;
    fs::rename(&tmp, path)
}

/// Drill-only fault injection, fired from inside the worker's kernel thread.
#[derive(Clone)]
pub enum Fault {
    /// SIGSEGV delivered to the kernel thread `after` this op began (the op is a
    /// `__slow` one, so the thread is inside the op when it lands).
    InPlanOp { op_id: String, after: Duration },
    /// SIGSEGV raised on the kernel thread as this verb's job begins.
    AtJobBegin { verb: String },
}

#[derive(Default)]
struct State {
    doc: String,
    record: Option<Value>,
}

/// The worker-side journal: one `inflight.json`, rewritten on every event.
pub struct WorkerJournal {
    path: PathBuf,
    state: Mutex<State>,
    fault: Option<Fault>,
    /// Wall time of each record write, for the kernel-lane cost measurement.
    pub write_times: Mutex<Vec<Duration>>,
}

impl WorkerJournal {
    pub fn new(path: PathBuf, fault: Option<Fault>) -> WorkerJournal {
        WorkerJournal {
            path,
            state: Mutex::default(),
            fault,
            write_times: Mutex::default(),
        }
    }

    /// Host side, once per session: which document the records belong to.
    pub fn set_document(&self, doc: &str) {
        self.state.lock().unwrap().doc = doc.to_string();
    }

    fn write(&self, record: &Value) {
        let s = Instant::now();
        if let Err(e) = write_fast(&self.path, record) {
            eprintln!("journal write failed: {e}");
        }
        self.write_times.lock().unwrap().push(s.elapsed());
    }

    /// Called on the worker's kernel thread (inproc.rs `journal_cb`).
    pub fn on_event(&self, ev: &JournalEventView) {
        let mut st = self.state.lock().unwrap();
        match ev.event {
            JOB_BEGIN => {
                let rec = json!({"v":3,"documentId":st.doc,"requestId":ev.request_id,
                                 "verb":ev.name,"cursor":null,"opId":null,"key":null});
                self.write(&rec);
                st.record = Some(rec);
                drop(st);
                if let Some(Fault::AtJobBegin { verb }) = &self.fault {
                    if *verb == ev.name {
                        // SAFETY: plain syscall; SIGSEGV is SIG_DFL in the child.
                        unsafe { libc::raise(libc::SIGSEGV) };
                    }
                }
            }
            PLAN_OP_BEGIN => {
                if let Some(rec) = st.record.as_mut() {
                    rec["cursor"] = json!(ev.op_position);
                    rec["stepIndex"] = json!(ev.step_index);
                    rec["opId"] = json!(ev.name);
                    rec["key"] =
                        json!(format!("{}|{}|{}", ev.prefix_hash, ev.name, ev.fingerprint));
                    let rec = rec.clone();
                    self.write(&rec);
                }
                drop(st);
                if let Some(Fault::InPlanOp { op_id, after }) = &self.fault {
                    if *op_id == ev.name {
                        arm_thread_fault(*after);
                    }
                }
            }
            JOB_END => {
                st.record = None;
                let s = Instant::now();
                let _ = fs::remove_file(&self.path);
                self.write_times.lock().unwrap().push(s.elapsed());
            }
            _ => {}
        }
    }
}

/// Delivers SIGSEGV to the CALLING (kernel) thread `after` from now, from a
/// helper thread, so the fault lands while the kernel thread is inside the op.
fn arm_thread_fault(after: Duration) {
    // SAFETY: pthread_self has no preconditions.
    let target = unsafe { libc::pthread_self() } as usize;
    std::thread::spawn(move || {
        std::thread::sleep(after);
        // SAFETY: `target` is the live kernel thread (it is inside a ~500 ms
        // `__slow` op); SIGSEGV is SIG_DFL in the child, so this kills the process.
        unsafe { libc::pthread_kill(target as libc::pthread_t, libc::SIGSEGV) };
    });
}

/// What the next launch learns from a leftover worker-side journal.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovered {
    Nothing,
    /// Crashed inside this plan op: poison its key.
    PlanOp {
        op_id: String,
        key: String,
    },
    /// Crashed in a non-plan kernel job: logged, nothing poisoned.
    Job {
        verb: String,
    },
}

pub fn recover_worker(dir: &Path) -> Result<Recovered, String> {
    let journal = dir.join("inflight.json");
    let Ok(bytes) = fs::read(&journal) else {
        return Ok(Recovered::Nothing);
    };
    let rec: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    fs::remove_file(&journal).map_err(|e| e.to_string())?;
    let verb = rec["verb"].as_str().unwrap_or("?").to_string();
    let Some(key) = rec["key"].as_str().map(str::to_string) else {
        return Ok(Recovered::Job { verb });
    };
    let op_id = rec["opId"].as_str().unwrap_or("?").to_string();
    if key.split('|').nth(1) != Some(op_id.as_str()) {
        return Err(format!("journal opId {op_id} does not match key {key}"));
    }
    add_poison(dir, &key)?;
    Ok(Recovered::PlanOp { op_id, key })
}

/// The pre-fix, host-side design: cursor = 1 + last planStep the host processed.
fn recover_host(dir: &Path) -> String {
    let Ok(bytes) = fs::read(dir.join("host_inflight.json")) else {
        return "no journal (nothing attributed)".into();
    };
    let rec: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let cursor = rec["cursor"].as_u64().unwrap_or(0) as usize;
    let key = rec["keys"][cursor].as_str().unwrap_or("?|?");
    format!("op '{}'", key.split('|').nth(1).unwrap_or("?"))
}

fn add_poison(dir: &Path, key: &str) -> Result<(), String> {
    let path = dir.join("poison.json");
    let mut poison: Value = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    poison[key] = json!(poison[key].as_u64().unwrap_or(0) + 1);
    write_fast(&path, &poison).map_err(|e| e.to_string())
}

fn poisoned(dir: &Path, keys: &[String]) -> Option<String> {
    let poison: Value = serde_json::from_slice(&fs::read(dir.join("poison.json")).ok()?).ok()?;
    keys.iter()
        .find(|k| poison.get(k.as_str()).is_some())
        .map(|k| k.split('|').nth(1).unwrap().to_string())
}

fn slow_ops(distance: f64) -> Vec<Value> {
    // `__slow` (PlanExecutor test hook, kept in library builds) holds op1 open
    // for ~500 ms in 10 ms slices, so the fault lands inside it.
    vec![sketch_op("op0"), extrude_op("op1__slow", distance)]
}

fn set_sigsegv_default() {
    // SAFETY: zeroed sigaction with SIG_DFL is a valid argument. Rust's std
    // installs a SIGSEGV handler (stack-overflow detection) that RETURNS for a
    // non-guard-page signal, which would make a pthread_kill'd SIGSEGV a no-op.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
    }
}

/// Child process: host app + in-process worker, killed by a fault on the
/// worker's kernel thread. `case`: `op1-stalled` (host never processes events),
/// `op1-live` (host processes planStep frames as they arrive), `tess` (fault in
/// a Tessellate job after a completed plan).
pub async fn child(dir: PathBuf, case: &str, sink: Arc<LogSink>) -> Result<(), String> {
    set_sigsegv_default();
    let fault = match case {
        "tess" => Fault::AtJobBegin {
            verb: "Tessellate".into(),
        },
        _ => Fault::InPlanOp {
            op_id: "op1__slow".into(),
            after: Duration::from_millis(100),
        },
    };
    let journal = Arc::new(WorkerJournal::new(dir.join("inflight.json"), Some(fault)));
    let w = InProcessWorker::start_with(sink, LOG_INFO, Some(journal.clone()), 0)
        .await
        .map_err(|e| e.to_string())?;
    let fp = w.client().hello().occt.fingerprint.clone();
    open_session(w.client(), "doc_crash").await?;
    journal.set_document("doc_crash");
    let ops = if case == "tess" {
        vec![sketch_op("op0"), extrude_op("op1", 25.0)]
    } else {
        slow_ops(25.0)
    };
    // Host-side keys: only the pre-fix host-side cursor design needs them.
    let keys = plan_keys(&ops, &fp);
    let host_journal = dir.join("host_inflight.json");
    let host_rec =
        |cursor: usize| json!({"v":1,"documentId":"doc_crash","keys":keys,"cursor":cursor});
    write_fast(&host_journal, &host_rec(0)).map_err(|e| e.to_string())?;
    let mut events = w.client().subscribe();
    let req = w
        .client()
        .start_request(
            "ExecutePlan",
            plan_args(9, ops.clone(), prefix_hashes(&ops)),
            Lane::Control,
        )
        .await
        .map_err(|e| e.to_string())?;
    match case {
        "op1-stalled" => {
            // The host's event handling is busy elsewhere (UI, a long sync
            // task): it never gets to the planStep frames before the crash.
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        "op1-live" => loop {
            match tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
                Ok(Ok(WorkerEvent::Event(e))) if e.id == req.id && e.event == "planStep" => {
                    let done = e.step_index.unwrap_or(0) as usize;
                    write_fast(&host_journal, &host_rec(done + 1)).map_err(|e| e.to_string())?;
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(format!("event stream ended: {e}")),
                Err(_) => break,
            }
        },
        "tess" => {
            let r = req.response().await.map_err(|e| e.to_string())?;
            crate::scenario::expect_ok("ExecutePlan", &r)?;
            let _ = fs::remove_file(&host_journal); // host-side: plan terminal seen
            let r = w
                .client()
                .request(
                    "AcceptPrepared",
                    json!({"jobId":9,"documentRevision":1,"workerEpoch":crate::scenario::EPOCH}),
                )
                .await
                .map_err(|e| e.to_string())?;
            crate::scenario::expect_ok("AcceptPrepared", &r)?;
            let _ = w
                .client()
                .request_timeout(
                    "Tessellate",
                    json!({"bodyIds":["body_op1"],"lod":"coarse","includeEdges":true}),
                    Duration::from_secs(10),
                )
                .await;
        }
        other => return Err(format!("unknown journal case {other}")),
    }
    Err("fault did not fire".into())
}

async fn run_case(dir: &Path, case: &str) -> Result<Option<i32>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = tokio::process::Command::new(exe)
        .arg("journal-child")
        .arg(dir)
        .arg(case)
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| e.to_string())?;
    use std::os::unix::process::ExitStatusExt;
    Ok(status.signal())
}

/// Kernel-lane cost of the worker-side journal: the same model round with and
/// without the callback writing records.
async fn journal_overhead(dir: &Path, sink: Arc<LogSink>) -> Result<(), String> {
    const N: u64 = 30;
    let mut plain = Vec::new();
    let w = InProcessWorker::start(sink.clone(), LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    for i in 0..N {
        plain.push(model_round(w.client(), i + 1).await?.plan);
    }
    w.shutdown(Duration::from_secs(5)).await;
    let journal = Arc::new(WorkerJournal::new(dir.join("overhead.json"), None));
    let w = InProcessWorker::start_with(sink, LOG_INFO, Some(journal.clone()), 0)
        .await
        .map_err(|e| e.to_string())?;
    journal.set_document("doc");
    let mut journaled = Vec::new();
    for i in 0..N {
        journaled.push(model_round(w.client(), i + 1).await?.plan);
    }
    w.shutdown(Duration::from_secs(5)).await;
    let writes = journal.write_times.lock().unwrap().clone();
    println!(
        "  kernel-lane cost: ExecutePlan median {:.3} ms without journal, {:.3} ms with (n={N}); \
         {} journal writes, median {:.3} ms, p95 {:.3} ms",
        crate::scenario::median_ms(plain),
        crate::scenario::median_ms(journaled),
        writes.len(),
        crate::scenario::median_ms(writes.clone()),
        crate::scenario::pct_ms(writes, 0.95)
    );
    Ok(())
}

pub async fn drill(fingerprint: &str, sink: Arc<LogSink>) -> Result<(), String> {
    println!("== drill (b): crash journal ==");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/drill-journal");
    let _ = fs::remove_dir_all(&root);
    for case in ["op1-stalled", "op1-live", "tess"] {
        let dir = root.join(case);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let signal = run_case(&dir, case).await?;
        let host = recover_host(&dir);
        let worker = recover_worker(&dir)?;
        println!(
            "  case {case}: child died by signal {signal:?}; worker-side journal → {}; \
             host-side planStep cursor (pre-fix design) → {host}",
            match &worker {
                Recovered::PlanOp { op_id, .. } => format!("poison op '{op_id}'"),
                Recovered::Job { verb } => format!("crash in {verb} job, nothing poisoned"),
                Recovered::Nothing => "no journal".into(),
            }
        );
        if signal != Some(libc::SIGSEGV) {
            return Err(format!("case {case}: expected SIGSEGV, got {signal:?}"));
        }
        if case.starts_with("op1") {
            let Recovered::PlanOp { op_id, key } = &worker else {
                return Err(format!(
                    "case {case}: worker journal did not attribute: {worker:?}"
                ));
            };
            if op_id != "op1__slow" {
                return Err(format!("case {case}: attributed to {op_id}"));
            }
            // The worker-built key must equal the host's plan_op_keys rule.
            let expected = &plan_keys(&slow_ops(25.0), fingerprint)[1];
            if key != expected {
                return Err(format!("case {case}: key {key} != host rule {expected}"));
            }
            println!("    worker-built key == host plan_op_keys rule: yes");
            println!(
                "    reopen same plan: gate → {:?}; op1 distance edited 25→30: gate → {:?}",
                poisoned(&dir, &plan_keys(&slow_ops(25.0), fingerprint)),
                poisoned(&dir, &plan_keys(&slow_ops(30.0), fingerprint))
            );
        } else if worker
            != (Recovered::Job {
                verb: "Tessellate".into(),
            })
        {
            return Err(format!("case tess: {worker:?}"));
        }
    }

    journal_overhead(&root, sink).await?;

    let same = plan_keys(&slow_ops(25.0), fingerprint);
    let probe = root.join("latency.json");
    let rec = json!({"v":2,"documentId":"doc","keys":same,"cursor":0});
    let mut lat = Vec::new();
    for _ in 0..100 {
        let s = Instant::now();
        write_durable(&probe, &rec).map_err(|e| e.to_string())?;
        lat.push(s.elapsed());
    }
    println!(
        "  journal write (tmp+F_FULLFSYNC+rename+dir sync), n=100: median {:.2} ms, p95 {:.2} ms",
        crate::scenario::median_ms(lat.clone()),
        crate::scenario::pct_ms(lat, 0.95)
    );
    let mut lat = Vec::new();
    for _ in 0..100 {
        let s = Instant::now();
        write_fast(&probe, &rec).map_err(|e| e.to_string())?;
        lat.push(s.elapsed());
    }
    println!(
        "  journal write (tmp+rename, no fsync), n=100: median {:.3} ms, p95 {:.3} ms",
        crate::scenario::median_ms(lat.clone()),
        crate::scenario::pct_ms(lat, 0.95)
    );
    Ok(())
}
