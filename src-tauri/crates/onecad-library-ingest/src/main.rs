//! `onecad-library-ingest` — headless vendor-STEP → component-library ingest.
//!
//! ```text
//! onecad-library-ingest <ingest.toml> [--library-root <dir>] [--worker <path>]
//!                       [--inspect] [--json]
//! ```
//!
//! Reads a tracked recipe, spawns the **real** OCCT worker, and runs the app's own
//! [`onecad_lib::library_ingest`] lane over it — so a recipe run and the in-app
//! "Import components…" run are the same code.
//!
//! `--inspect` writes NOTHING. It imports each part and prints its solid table
//! (index, product name, exact volume, face count, bbox), which is how a
//! `[part.keep]` list is authored in the first place: a NEMA 17 STEP carries 67
//! solids and only a handful of them are the exterior envelope.
//!
//! ## Exit-code policy
//!
//! * **0** — every part `ok` (or, under `--inspect`, every part read).
//! * **1** — any part `refused` or `failed`.
//! * **2** — usage error.
//! * **3** — environment error (no recipe, no worker binary, worker never ready).

use std::path::PathBuf;
use std::process::ExitCode;

use onecad_lib::dto::{IngestPartResultDto, IngestStatusDto};
use onecad_lib::library_ingest::{
    ingest_components_at, inspect_components_at, parse_recipe, spawn_ingest_worker, IngestPlan,
    InspectedPart,
};

const USAGE: &str = "usage: onecad-library-ingest <ingest.toml> [--library-root <dir>] \
                     [--worker <path>] [--inspect] [--json]";

struct Args {
    recipe: PathBuf,
    library_root: Option<PathBuf>,
    worker: Option<PathBuf>,
    inspect: bool,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut recipe: Option<PathBuf> = None;
    let mut library_root: Option<PathBuf> = None;
    let mut worker: Option<PathBuf> = None;
    let mut inspect = false;
    let mut json = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--inspect" => inspect = true,
            "--json" => json = true,
            "--library-root" => {
                library_root =
                    Some(PathBuf::from(it.next().ok_or_else(|| {
                        "--library-root requires a path".to_string()
                    })?));
            }
            "--worker" => {
                worker = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| "--worker requires a path".to_string())?,
                ));
            }
            "-h" | "--help" => return Err("help".to_string()),
            other if other.starts_with('-') => return Err(format!("unknown flag {other:?}")),
            other => {
                if recipe.replace(PathBuf::from(other)).is_some() {
                    return Err("more than one recipe given".to_string());
                }
            }
        }
    }
    let recipe = recipe.ok_or_else(|| "missing <ingest.toml>".to_string())?;
    if library_root.is_none() && !inspect {
        return Err("--library-root is required unless --inspect".to_string());
    }
    Ok(Args {
        recipe,
        library_root,
        worker,
        inspect,
        json,
    })
}

/// stderr-only diagnostics with a `RUST_LOG`-aware filter, matching
/// `onecad-regen`: stdout is the report, and only the app writes `logs/dev.jsonl`.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,onecad_lib=info"));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
    init_tracing();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) if e == "help" => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("onecad-library-ingest: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("onecad-library-ingest: {e}");
            ExitCode::from(3)
        }
    }
}

fn load_plan(recipe: &PathBuf) -> Result<IngestPlan, String> {
    let text = std::fs::read_to_string(recipe).map_err(|e| format!("read {recipe:?}: {e}"))?;
    let base = recipe.parent().unwrap_or_else(|| std::path::Path::new("."));
    parse_recipe(&text, base)
}

async fn run(args: Args) -> Result<ExitCode, String> {
    let plan = load_plan(&args.recipe)?;
    if let Some(w) = &args.worker {
        // The lane resolves its own binary; this is the documented override.
        std::env::set_var("ONECAD_WORKER_PATH", w);
    }
    eprintln!(
        "onecad-library-ingest: {} part(s) from {:?}",
        plan.parts.len(),
        args.recipe
    );
    let worker = spawn_ingest_worker().await.map_err(|e| e.to_string())?;

    if args.inspect {
        let inspected = inspect_components_at(&worker.0, &plan).await;
        let failures = inspected.iter().filter(|p| p.solids.is_err()).count();
        if args.json {
            println!("{}", inspect_json(&inspected));
        } else {
            print_inspect(&plan, &inspected);
        }
        return Ok(ExitCode::from(u8::from(failures > 0)));
    }

    let root = args.library_root.expect("checked in parse_args");
    let report = ingest_components_at(&worker.0, &root, &plan).await;
    let bad = report
        .parts
        .iter()
        .filter(|p| p.status != IngestStatusDto::Ok)
        .count();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        );
    } else {
        print_report(&report.parts, &report.library_root);
    }
    Ok(ExitCode::from(u8::from(bad > 0)))
}

fn print_inspect(plan: &IngestPlan, inspected: &[InspectedPart]) {
    for (part, got) in plan.parts.iter().zip(inspected) {
        println!("\n=== {} ({}) ===", part.id, got.path.display());
        match &got.solids {
            Err(e) => println!("  READ FAILED: {e}"),
            Ok(rows) => {
                let total: f64 = rows.iter().map(|r| r.volume_mm3).sum();
                println!(
                    "  {} solid(s), {} ms, total volume {:.1} mm³",
                    rows.len(),
                    got.import_ms,
                    total
                );
                println!(
                    "  {:>4}  {:>14}  {:>6}  {:>6}  {:>26}  product name",
                    "idx", "volume mm³", "%", "faces", "bbox mm (dx×dy×dz @ z0)"
                );
                for r in rows {
                    let pct = if total > 0.0 {
                        100.0 * r.volume_mm3 / total
                    } else {
                        0.0
                    };
                    let bbox = r
                        .bbox
                        .map(|(lo, hi)| {
                            format!(
                                "{:>6.1}×{:>6.1}×{:>6.1} @{:>6.1}",
                                hi[0] - lo[0],
                                hi[1] - lo[1],
                                hi[2] - lo[2],
                                lo[2]
                            )
                        })
                        .unwrap_or_else(|| format!("{:>26}", "-"));
                    println!(
                        "  {:>4}  {:>14.3}  {:>5.1}%  {:>6}  {bbox}  {}",
                        r.index, r.volume_mm3, pct, r.face_count, r.name
                    );
                }
            }
        }
    }
}

fn inspect_json(inspected: &[InspectedPart]) -> String {
    let rows: Vec<serde_json::Value> = inspected
        .iter()
        .map(|p| {
            serde_json::json!({
                "path": p.path.to_string_lossy(),
                "importMs": p.import_ms,
                "error": p.solids.as_ref().err(),
                "solids": p.solids.as_ref().ok().map(|rows| rows.iter().map(|r| serde_json::json!({
                    "index": r.index,
                    "name": r.name,
                    "volumeMm3": r.volume_mm3,
                    "faceCount": r.face_count,
                    "solidCount": r.solid_count,
                    "bboxMin": r.bbox.map(|(lo, _)| lo),
                    "bboxMax": r.bbox.map(|(_, hi)| hi),
                })).collect::<Vec<_>>()),
            })
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn print_report(parts: &[IngestPartResultDto], root: &str) {
    println!("\nlibrary root: {root}");
    println!(
        "{:<9}  {:<44}  {:<9}  {:>6}  {:>6}  {:>6}  {:>8}",
        "status", "id", "kind", "found", "kept", "faces", "import"
    );
    for p in parts {
        let status = match p.status {
            IngestStatusDto::Ok => "ok",
            IngestStatusDto::Refused => "REFUSED",
            IngestStatusDto::Failed => "FAILED",
        };
        let kind = match p.kind {
            Some(onecad_lib::dto::IngestPartKindDto::Embedded) => "embedded",
            Some(onecad_lib::dto::IngestPartKindDto::Profile) => "profile",
            None => "-",
        };
        println!(
            "{:<9}  {:<44}  {:<9}  {:>6}  {:>6}  {:>6}  {:>7}",
            status,
            p.id.as_deref().unwrap_or("-"),
            kind,
            p.solids_found.map(|v| v.to_string()).unwrap_or_default(),
            p.solids_kept.map(|v| v.to_string()).unwrap_or_default(),
            p.face_count.map(|v| v.to_string()).unwrap_or_default(),
            p.import_ms
                .map(|v| format!("{v} ms"))
                .unwrap_or_else(|| "-".into()),
        );
        if let Some(m) = &p.message {
            println!("           ↳ {m}");
        }
    }
}
