//! P0.3 in-process OCCT worker spike (Plane ONECAD-258), macOS part.
//!
//! `cargo run --release -- [all|bench|wedge|cancel|step-lock|concurrent [n]|concurrent-modeling [n]|restart|journal|logs]`
//! Sidecar baseline binary: $ONECAD_WORKER_PATH or ../../worker/build/onecad-worker.
mod drills;
mod inproc;
mod journal;
mod scenario;
mod sidecar;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use inproc::{InProcessWorker, LogSink, LOG_INFO};
use scenario::{median_ms, model_round, pct_ms, ping_rtts, OpTimings};
use sidecar::SidecarWorker;

const HANDSHAKES: usize = 5;
const PINGS: usize = 200;
const ROUNDS: usize = 30;

fn sidecar_path() -> PathBuf {
    std::env::var_os("ONECAD_WORKER_PATH").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker/build/onecad-worker"),
        PathBuf::from,
    )
}

#[derive(Default)]
struct Bench {
    handshakes: Vec<Duration>,
    pings: Vec<Duration>,
    rounds: Vec<OpTimings>,
    fingerprint: String,
}

fn col(rounds: &[OpTimings], f: fn(&OpTimings) -> Duration) -> Vec<Duration> {
    rounds.iter().map(f).collect()
}

async fn bench_inproc(sink: Arc<LogSink>) -> Result<Bench, String> {
    let mut b = Bench::default();
    for _ in 0..HANDSHAKES {
        let w = InProcessWorker::start(sink.clone(), LOG_INFO)
            .await
            .map_err(|e| e.to_string())?;
        b.handshakes.push(w.handshake);
        w.shutdown(Duration::from_secs(5)).await;
    }
    let w = InProcessWorker::start(sink, LOG_INFO)
        .await
        .map_err(|e| e.to_string())?;
    b.fingerprint = w.client().hello().occt.fingerprint.clone();
    b.pings = ping_rtts(w.client(), PINGS).await?;
    for i in 0..ROUNDS {
        b.rounds.push(model_round(w.client(), i as u64 + 1).await?);
    }
    w.shutdown(Duration::from_secs(5)).await;
    Ok(b)
}

async fn bench_sidecar() -> Result<Bench, String> {
    let path = sidecar_path();
    let mut b = Bench::default();
    for _ in 0..HANDSHAKES {
        let w = SidecarWorker::start(&path)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
        b.handshakes.push(w.handshake);
        w.shutdown().await;
    }
    let w = SidecarWorker::start(&path)
        .await
        .map_err(|e| e.to_string())?;
    b.fingerprint = w.client().hello().occt.fingerprint.clone();
    b.pings = ping_rtts(w.client(), PINGS).await?;
    for i in 0..ROUNDS {
        b.rounds.push(model_round(w.client(), i as u64 + 1).await?);
    }
    w.shutdown().await;
    Ok(b)
}

fn print_row(name: &str, ip: Vec<Duration>, sc: Vec<Duration>) {
    println!(
        "| {name:<22} | {:>8.3} | {:>8.3} | {:>8.3} | {:>8.3} |",
        median_ms(ip.clone()),
        pct_ms(ip, 0.95),
        median_ms(sc.clone()),
        pct_ms(sc, 0.95)
    );
}

async fn bench(sink: Arc<LogSink>) -> Result<String, String> {
    println!("== bench: in-process vs sidecar (same ProtocolClient, same requests) ==");
    let ip = bench_inproc(sink).await?;
    let sc = bench_sidecar().await?;
    println!(
        "  handshake first-of-process: in-process {:.2} ms, sidecar {:.2} ms",
        ip.handshakes[0].as_secs_f64() * 1e3,
        sc.handshakes[0].as_secs_f64() * 1e3
    );
    println!("| ms (median / p95)      | in-proc  |  p95     | sidecar  |  p95     |");
    println!("|------------------------|----------|----------|----------|----------|");
    print_row(
        &format!("handshake (n={HANDSHAKES})"),
        ip.handshakes.clone(),
        sc.handshakes.clone(),
    );
    print_row(
        &format!("GetWorkerHead (n={PINGS})"),
        ip.pings.clone(),
        sc.pings.clone(),
    );
    // Round 0 is reported inside the medians; first-op warmup shown separately.
    print_row(
        &format!("OpenSession (n={ROUNDS})"),
        col(&ip.rounds, |t| t.open),
        col(&sc.rounds, |t| t.open),
    );
    print_row(
        &format!("ExecutePlan (n={ROUNDS})"),
        col(&ip.rounds, |t| t.plan),
        col(&sc.rounds, |t| t.plan),
    );
    print_row(
        &format!("AcceptPrepared (n={ROUNDS})"),
        col(&ip.rounds, |t| t.accept),
        col(&sc.rounds, |t| t.accept),
    );
    print_row(
        &format!("Tessellate (n={ROUNDS})"),
        col(&ip.rounds, |t| t.tessellate),
        col(&sc.rounds, |t| t.tessellate),
    );
    println!(
        "  first ExecutePlan of the worker: in-process {:.2} ms, sidecar {:.2} ms",
        ip.rounds[0].plan.as_secs_f64() * 1e3,
        sc.rounds[0].plan.as_secs_f64() * 1e3
    );
    let shas: std::collections::BTreeSet<&str> = ip
        .rounds
        .iter()
        .chain(sc.rounds.iter())
        .map(|t| t.mesh_sha.as_str())
        .collect();
    println!(
        "  fingerprint in-process {} / sidecar {}; mesh {} tris {} bytes; distinct mesh sha256 over {} rounds: {}",
        ip.fingerprint,
        sc.fingerprint,
        ip.rounds[0].triangles,
        ip.rounds[0].mesh_bytes,
        2 * ROUNDS,
        shas.len()
    );
    if ip.fingerprint != sc.fingerprint || shas.len() != 1 {
        return Err("in-process and sidecar disagree".into());
    }
    Ok(ip.fingerprint)
}

async fn logs(sink: Arc<LogSink>, cout_sink: &'static LogSink) -> Result<(), String> {
    println!("== logging routing ==");
    inproc::cout_selftest("spike cout probe");
    println!(
        "  std::cout bridge: {} line(s) captured",
        cout_sink.count_containing("cout: spike cout probe")
    );
    let before = sink.lines.lock().unwrap().len();
    drills::concurrent(sink.clone(), 1, true).await?;
    let lines = sink.lines.lock().unwrap();
    let new = &lines[before..];
    let occt = new.iter().filter(|(_, l)| l.starts_with("occt: ")).count();
    let starting = new
        .iter()
        .filter(|(_, l)| l.contains("onecad-worker 0.1.0 starting"))
        .count();
    println!(
        "  log_cb received {} lines during one ExportStep/InspectStep round: {starting} Log.h 'starting', {occt} via the OCCT Message_Printer bridge",
        new.len()
    );
    let multiline = lines
        .iter()
        .filter(|(_, l)| l.contains('\n') || l.is_empty())
        .count();
    println!("  lines with an embedded newline or empty, over the whole run: {multiline}");
    if multiline != 0 {
        return Err("log_cb received a multi-line or empty line".into());
    }
    for (lvl, l) in new.iter().filter(|(_, l)| l.starts_with("occt: ")).take(3) {
        println!(
            "    e.g. L{lvl} {}",
            l.chars().take(100).collect::<String>()
        );
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("all", String::as_str);
    let sink = Arc::new(LogSink {
        echo: std::env::var_os("SPIKE_ECHO_LOGS").is_some(),
        ..LogSink::default()
    });
    let cout_sink: &'static LogSink = Box::leak(Box::default());
    inproc::route_cout(cout_sink);

    if mode == "journal-child" {
        let dir = PathBuf::from(&args[2]);
        if let Err(e) = journal::child(dir, &args[3], sink).await {
            eprintln!("journal-child: {e}");
            std::process::exit(1);
        }
        return;
    }

    let abi = inproc::abi_version();
    println!("onecad_worker ABI version {abi}");
    if abi != inproc::EXPECTED_ABI {
        eprintln!(
            "FAILED: library ABI {abi}, spike written for {}",
            inproc::EXPECTED_ABI
        );
        std::process::exit(1);
    }
    let run = |r: Result<(), String>| {
        if let Err(e) = r {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    };
    let all = mode == "all";
    let mut fingerprint = String::new();
    if all || mode == "bench" {
        match bench(sink.clone()).await {
            Ok(fp) => fingerprint = fp,
            Err(e) => run(Err(e)),
        }
    }
    if all || mode == "logs" {
        run(logs(sink.clone(), cout_sink).await);
    }
    if all || mode == "cancel" {
        run(drills::cancel(sink.clone()).await);
    }
    if all || mode == "step-lock" {
        run(drills::step_lock(sink.clone()).await);
    }
    if all || mode == "restart" {
        run(drills::restart(sink.clone(), 5).await);
    }
    if all || mode == "concurrent" {
        let rounds = args.get(2).and_then(|r| r.parse().ok()).unwrap_or(10);
        run(drills::concurrent(sink.clone(), rounds, true).await);
    }
    if all || mode == "concurrent-modeling" {
        let rounds = args.get(2).and_then(|r| r.parse().ok()).unwrap_or(10);
        run(drills::concurrent(sink.clone(), rounds, false).await);
    }
    if all || mode == "journal" {
        if fingerprint.is_empty() {
            let w = InProcessWorker::start(sink.clone(), LOG_INFO)
                .await
                .expect("start");
            fingerprint = w.client().hello().occt.fingerprint.clone();
            w.shutdown(Duration::from_secs(5)).await;
        }
        run(journal::drill(&fingerprint, sink.clone()).await);
    }
    // Last: it leaks a spinning thread for the rest of the process.
    if all || mode == "wedge" {
        run(drills::wedge(sink.clone()).await);
    }
    println!(
        "spike done: {} worker log lines via log_cb",
        sink.lines.lock().unwrap().len()
    );
    // A wedged worker thread cannot be joined; exit the process explicitly.
    std::process::exit(0);
}
