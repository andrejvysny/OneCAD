//! Failure drills (ONECAD-258 task 4): wedged op, cooperative cancel, two
//! concurrent in-process workers, a held OCCT-globals lock, and run → shutdown
//! → run re-init.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use onecad_protocol::messages::{Lane, RespFrame};
use serde_json::json;

use crate::inproc::{ExitObservation, InProcessWorker, LogSink, LOG_INFO};
use crate::scenario::{expect_ok, extrude_op, model_round, open_session, plan_args, sketch_op};

fn cpu_seconds() -> f64 {
    // SAFETY: getrusage writes into a zeroed, correctly sized struct.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let t = |tv: libc::timeval| tv.tv_sec as f64 + tv.tv_usec as f64 * 1e-6;
    t(ru.ru_utime) + t(ru.ru_stime)
}

fn thread_count() -> usize {
    let out = std::process::Command::new("ps")
        .args(["-M", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    // header line + one line per thread
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .count()
        .saturating_sub(1)
}

/// (a) An op that never returns. `Debug.Busy` spins the kernel lane without a
/// cancellation check — the in-process stand-in for an OCCT call stuck in a loop.
pub async fn wedge(sink: Arc<LogSink>) -> Result<(), String> {
    println!("== drill (a): wedged op ==");
    let threads_before = thread_count();
    let mut a = InProcessWorker::start(sink.clone(), LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    open_session(a.client(), "doc_wedge").await?;
    let busy = a
        .client()
        .start_request(
            "Debug.Busy",
            json!({"durationMs": 3_600_000}),
            Lane::Control,
        )
        .await
        .map_err(|e| e.to_string())?;
    let busy_id = busy.id;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let s = Instant::now();
    let head = a
        .client()
        .request_timeout("GetWorkerHead", json!({}), Duration::from_secs(1))
        .await
        .map_err(|e| e.to_string())?;
    let inflight = head.result.unwrap_or_default()["inflight"].clone();
    println!(
        "  status lane answers while wedged: GetWorkerHead {:.2} ms, inflight={inflight}",
        s.elapsed().as_secs_f64() * 1e3
    );

    a.client()
        .cancel(busy_id)
        .await
        .map_err(|e| e.to_string())?;
    let after_cancel = tokio::time::timeout(Duration::from_secs(1), busy.response()).await;
    println!(
        "  cancel(id {busy_id}) → terminal within 1 s: {}",
        if after_cancel.is_ok() {
            "YES"
        } else {
            "NO (op ignores cancel)"
        }
    );

    let c0 = cpu_seconds();
    tokio::time::sleep(Duration::from_secs(1)).await;
    println!(
        "  process CPU while wedged: {:.2} CPU-s per wall-s",
        cpu_seconds() - c0
    );

    let shut = a
        .client()
        .request_timeout("Shutdown", json!({}), Duration::from_secs(2))
        .await;
    println!(
        "  Shutdown (2 s deadline): {}",
        match &shut {
            Ok(r) => format!("ok={}", r.ok),
            Err(e) => format!("{e} (queued behind the wedged kernel lane)"),
        }
    );
    a.close_host().await;
    let obs = a.wait_exit(Duration::from_secs(3));
    println!("  host end closed, waited 3 s for onecad_worker_run to return: {obs:?}");
    if obs != ExitObservation::RelaunchRequired {
        return Err(format!("expected RelaunchRequired, got {obs:?}"));
    }
    println!(
        "  threads: {threads_before} before start, {} with the wedged worker leaked",
        thread_count()
    );

    // A second worker in the same process while the first is still wedged.
    let b = InProcessWorker::start(sink.clone(), LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    let t = model_round(b.client(), 1).await?;
    println!(
        "  second in-process worker while first wedged: handshake {:.1} ms, plan {:.1} ms, mesh {} tris",
        b.handshake.as_secs_f64() * 1e3,
        t.plan.as_secs_f64() * 1e3,
        t.triangles
    );
    println!(
        "  second worker shutdown: {:?}",
        b.shutdown(Duration::from_secs(5)).await
    );
    a.leak();
    Ok(())
}

/// Cooperative cancel: the `__slow` hook checks the cancel token every 10 ms.
pub async fn cancel(sink: Arc<LogSink>) -> Result<(), String> {
    println!("== drill (a2): cooperative cancel ==");
    let w = InProcessWorker::start(sink, LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    open_session(w.client(), "doc_cancel").await?;
    let ops = vec![sketch_op("op0"), extrude_op("op1__slow", 25.0)];
    let req = w
        .client()
        .start_request(
            "ExecutePlan",
            plan_args(7, ops, crate::scenario::fixture_prefixes()),
            Lane::Control,
        )
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let s = Instant::now();
    w.client().cancel(req.id).await.map_err(|e| e.to_string())?;
    let r = req.response().await.map_err(|e| e.to_string())?;
    println!(
        "  ExecutePlan cancelled mid-op: terminal after {:.1} ms, ok={}, error={:?}",
        s.elapsed().as_secs_f64() * 1e3,
        r.ok,
        r.error.as_ref().map(|e| format!("{:?}", e.code))
    );
    println!("  shutdown: {:?}", w.shutdown(Duration::from_secs(5)).await);
    Ok(())
}

async fn concurrent_lane(
    w: &InProcessWorker,
    tag: &str,
    rounds: u64,
    dir: &std::path::Path,
    with_step: bool,
) -> Result<Vec<String>, String> {
    let mut shas = Vec::new();
    for i in 0..rounds {
        let t = model_round(w.client(), 100 + i).await?;
        shas.push(t.mesh_sha);
        if !with_step {
            continue;
        }
        let path = dir.join(format!("{tag}_{i}.step"));
        let p = path.to_string_lossy().to_string();
        let r = w
            .client()
            .request(
                "ExportStep",
                json!({"path":p,"bodyIds":["body_op1"],"schema":"AP242DIS"}),
            )
            .await
            .map_err(|e| e.to_string())?;
        expect_ok("ExportStep", &r)?;
        let r = w
            .client()
            .request("InspectStep", json!({"path":p,"includeGeometry":false}))
            .await
            .map_err(|e| e.to_string())?;
        expect_ok("InspectStep", &r)?;
        let solids = r.result.unwrap_or_default()["solidCount"]
            .as_u64()
            .unwrap_or(0);
        if solids != 1 {
            return Err(format!("{tag} round {i}: InspectStep solidCount {solids}"));
        }
    }
    Ok(shas)
}

/// (c) Two in-process workers in one process, concurrently: model rounds, plus
/// (`with_step`) STEP export + inspect (the Interface_Static / messenger / OCAF
/// globals, serialized by io::occt_globals_mutex).
pub async fn concurrent(sink: Arc<LogSink>, rounds: u64, with_step: bool) -> Result<(), String> {
    println!(
        "== drill (c): two concurrent in-process workers ({}) ==",
        if with_step {
            "modeling + STEP"
        } else {
            "modeling only"
        }
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/drill-step");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let a = InProcessWorker::start(sink.clone(), LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    let b = InProcessWorker::start(sink.clone(), LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    let s = Instant::now();
    let (ra, rb) = tokio::join!(
        concurrent_lane(&a, "a", rounds, &dir, with_step),
        concurrent_lane(&b, "b", rounds, &dir, with_step)
    );
    let wall = s.elapsed();
    let (ra, rb) = (ra?, rb?);
    let all: std::collections::BTreeSet<&String> = ra.iter().chain(rb.iter()).collect();
    println!(
        "  {rounds} rounds each ({}), wall {:.0} ms, distinct mesh sha256 across both: {}",
        if with_step {
            "plan+accept+tess+ExportStep+InspectStep"
        } else {
            "plan+accept+tess"
        },
        wall.as_secs_f64() * 1e3,
        all.len()
    );
    if all.len() != 1 {
        return Err(format!("mesh hashes diverged: {all:?}"));
    }
    println!(
        "  A shutdown: {:?}",
        a.shutdown(Duration::from_secs(5)).await
    );
    println!(
        "  B shutdown: {:?}",
        b.shutdown(Duration::from_secs(5)).await
    );
    Ok(())
}

/// (d) Run → Shutdown → run again in the same process, `cycles` times.
pub async fn restart(sink: Arc<LogSink>, cycles: u64) -> Result<(), String> {
    println!("== drill (d): restart in the same process ==");
    let mut fingerprints = std::collections::BTreeSet::new();
    let mut shas = std::collections::BTreeSet::new();
    for i in 0..cycles {
        let w = InProcessWorker::start(sink.clone(), LOG_INFO)
            .await
            .map_err(|e| e.to_string())?;
        fingerprints.insert(w.client().hello().occt.fingerprint.clone());
        let t = model_round(w.client(), 1).await?;
        shas.insert(t.mesh_sha.clone());
        let hs = w.handshake;
        // Odd cycles keep the host end open across Shutdown: the worker must
        // wake its own reader (library builds: shutdown(SHUT_RD), fd kept
        // reserved until onecad_worker_run returns).
        let obs = if i % 2 == 1 {
            let mut w = w;
            w.shutdown_keep_host_open(Duration::from_secs(5)).await
        } else {
            w.shutdown(Duration::from_secs(5)).await
        };
        println!(
            "  cycle {i}: handshake {:.2} ms, plan {:.1} ms, exit {obs:?}{}",
            hs.as_secs_f64() * 1e3,
            t.plan.as_secs_f64() * 1e3,
            if i % 2 == 1 {
                " (host end kept open across Shutdown)"
            } else {
                ""
            }
        );
        if obs != ExitObservation::Exited(0) {
            return Err(format!("cycle {i}: {obs:?}"));
        }
    }
    println!(
        "  distinct fingerprints {}, distinct mesh sha256 {}",
        fingerprints.len(),
        shas.len()
    );
    Ok(())
}

async fn export_step(w: &InProcessWorker, path: &std::path::Path) -> Result<RespFrame, String> {
    w.client()
        .request(
            "ExportStep",
            json!({"path":path.to_string_lossy(),"bodyIds":["body_op1"],"schema":"AP242DIS"}),
        )
        .await
        .map_err(|e| e.to_string())
}

/// A STEP/XCAF terminal that failed because a process-wide OCCT lock was held.
fn expect_busy(verb: &str, r: &RespFrame, elapsed: Duration) -> Result<(), String> {
    let Some(err) = r.error.as_ref().filter(|_| !r.ok) else {
        return Err(format!("{verb} succeeded while the lock was held"));
    };
    println!(
        "  {verb} while held: ok=false code={:?} retriable={} after {:.0} ms: {}",
        err.code,
        err.retriable,
        elapsed.as_secs_f64() * 1e3,
        err.message.chars().take(110).collect::<String>()
    );
    if !err.retriable || !err.message.contains("OCCT_GLOBALS_BUSY") {
        return Err(format!(
            "{verb}: not a retriable OCCT_GLOBALS_BUSY: {err:?}"
        ));
    }
    Ok(())
}

/// (a3) A sibling worker wedged INSIDE a STEP call holds io::occt_globals_mutex
/// for the rest of the process. Simulated by holding that mutex from the host
/// (csrc/lock_probe.cpp). The bounded lock must turn every STEP call into a
/// retriable busy failure instead of wedging this worker's kernel lane too.
pub async fn step_lock(sink: Arc<LogSink>) -> Result<(), String> {
    const WAIT_MS: u32 = 500;
    println!(
        "== drill (a3): OCCT-globals lock held by a wedged sibling (wait bound {WAIT_MS} ms) =="
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/drill-steplock");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (before, held) = (dir.join("before.step"), dir.join("held.step"));
    let _ = std::fs::remove_file(&held);
    let w = InProcessWorker::start_with(sink.clone(), LOG_INFO, None, WAIT_MS)
        .await
        .map_err(|e| e.to_string())?;
    model_round(w.client(), 1).await?;
    expect_ok("ExportStep (lock free)", &export_step(&w, &before).await?)?;

    crate::inproc::hold_occt_globals();
    let result = step_lock_held(&w, sink, &before, &held).await;
    crate::inproc::release_occt_globals();
    result?;

    let s = Instant::now();
    expect_ok("ExportStep (released)", &export_step(&w, &held).await?)?;
    println!(
        "  after release: ExportStep ok in {:.1} ms",
        s.elapsed().as_secs_f64() * 1e3
    );
    println!("  shutdown: {:?}", w.shutdown(Duration::from_secs(5)).await);
    Ok(())
}

async fn step_lock_held(
    w: &InProcessWorker,
    sink: Arc<LogSink>,
    before: &std::path::Path,
    held: &std::path::Path,
) -> Result<(), String> {
    let s = Instant::now();
    let r = export_step(w, held).await?;
    expect_busy("ExportStep", &r, s.elapsed())?;
    if held.exists() {
        return Err("busy ExportStep still wrote a file".into());
    }
    let s = Instant::now();
    let r = w
        .client()
        .request(
            "InspectStep",
            json!({"path":before.to_string_lossy(),"includeGeometry":false}),
        )
        .await
        .map_err(|e| e.to_string())?;
    expect_busy("InspectStep", &r, s.elapsed())?;
    let t = model_round(w.client(), 2).await?;
    println!(
        "  modeling while held: plan {:.1} ms, tessellate {:.1} ms (unaffected)",
        t.plan.as_secs_f64() * 1e3,
        t.tessellate.as_secs_f64() * 1e3
    );
    // Same bound: the wait is process-global and the last started run sets it.
    let b = InProcessWorker::start_with(sink, LOG_INFO, None, 500)
        .await
        .map_err(|e| e.to_string())?;
    let t = model_round(b.client(), 1).await?;
    println!(
        "  new worker started while held: handshake {:.2} ms, plan {:.1} ms; shutdown {:?}",
        b.handshake.as_secs_f64() * 1e3,
        t.plan.as_secs_f64() * 1e3,
        b.shutdown(Duration::from_secs(5)).await
    );
    Ok(())
}
