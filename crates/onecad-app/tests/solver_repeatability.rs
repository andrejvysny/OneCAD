//! ONECAD-305 same-build replay evidence. Open Arc movement is recorded separately
//! from constraint correctness; no preferred pose is imposed on its four free DOF.
#[path = "solver_replay/fixture.rs"]
mod fixture;
#[path = "solver_replay/statistics.rs"]
mod statistics;
#[path = "solver_replay/transport.rs"]
mod transport;

use onecad_app::worker::wire;
use onecad_protocol::framing::encode_frame;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use transport::{sha, Worker};

const REPEATS: usize = 12;

#[test]
#[ignore = "explicit production-worker replay; emits immutable request and response evidence"]
fn free_arc_radius_edit_same_process_and_fresh_worker_repeatability() {
    let binary =
        PathBuf::from(std::env::var_os("ONECAD_WORKER_PATH").expect("ONECAD_WORKER_PATH required"))
            .canonicalize()
            .unwrap();
    let out = PathBuf::from(
        std::env::var_os("ONECAD_SOLVER_REPLAY_OUT").expect("ONECAD_SOLVER_REPLAY_OUT required"),
    );
    fs::create_dir(&out).expect("evidence directory must be fresh");
    let mut report = json!({ "source": provenance(&binary), "repeatsPerMode": REPEATS,
        "scope": "Same-build same-host replay. Cross-host tolerance and desired free-Arc movement are separate decisions.",
        "cases": [] });
    for control in [false, true] {
        let name = if control {
            "constrained-control"
        } else {
            "reported-free-arc"
        };
        report["cases"]
            .as_array_mut()
            .unwrap()
            .push(replay(&binary, &out.join(name), control));
        save(&out.join("report.json"), &report);
    }
    for case in report["cases"].as_array().unwrap() {
        for check in [
            "sameProcessExactRepeatable",
            "freshProcessExactRepeatable",
            "sameAndFreshExactAgreement",
        ] {
            assert_eq!(
                case[check],
                true,
                "{} {check}; evidence: {}",
                case["name"],
                out.join("report.json").display()
            );
        }
    }
    assert_eq!(
        sha(&fs::read(&binary).unwrap()),
        report["source"]["workerSha256"]
    );
    eprintln!(
        "ONECAD-305 replay report: {}",
        out.join("report.json").display()
    );
}

fn replay(binary: &Path, out: &Path, control: bool) -> Value {
    fs::create_dir_all(out).unwrap();
    let mut worker = Worker::start(binary, &out.join("capture"));
    let mut sketch = fixture::sketch(control);
    save(
        &out.join("authored-sketch.json"),
        &serde_json::to_value(&sketch).unwrap(),
    );
    let seed = wire::sketch_upsert_args(&sketch);
    let bare = worker.request(&request(1, &seed), 1);
    fixture::radius(&mut sketch);
    let pre = wire::sketch_upsert_args(&sketch);
    let accepted = worker.request(&request(2, &pre), 2);
    let accepted = adopt(&pre, &accepted);
    save(&out.join("accepted-pre-edit.json"), &accepted);
    let mut edit = accepted.clone();
    edit["constraints"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["type"] == "Radius")
        .unwrap()["value"] = json!(14.);
    let frames = [request(1, &seed), request(2, &pre), request(3, &edit)];
    for (index, frame) in frames.iter().enumerate() {
        fs::write(out.join(format!("request-{}.ocw1", index + 1)), frame).unwrap();
    }
    let mut same = vec![];
    for _ in 0..REPEATS {
        same.push(sequence(&mut worker, &frames, &edit, control));
    }
    save(
        &out.join("same-process-transcript.json"),
        &json!(worker.transcript),
    );
    let hello = worker.hello.clone();
    drop(worker);
    let mut fresh = vec![];
    for index in 0..REPEATS {
        let directory = out.join(format!("fresh-{index:02}"));
        let mut worker = Worker::start(binary, &directory);
        assert_eq!(worker.hello["result"], hello["result"]);
        fresh.push(sequence(&mut worker, &frames, &edit, control));
        save(
            &directory.join("transcript.json"),
            &json!(worker.transcript),
        );
    }
    json!({ "name": if control { "constrained-control" } else { "reported-free-arc" },
        "hello": hello, "bareSolve": bare,
        "requestSha256": frames.iter().map(|f| sha(f)).collect::<Vec<_>>(),
        "sequenceSha256": sha(&frames.concat()), "sameProcess": same, "freshProcess": fresh,
        "sameProcessDiagnostics": statistics::summarize(&same),
        "freshProcessDiagnostics": statistics::summarize(&fresh),
        "combinedDiagnostics": statistics::summarize(&[same.clone(), fresh.clone()].concat()),
        "sameProcessExactRepeatable": same.iter().all(|s| s == &same[0]),
        "freshProcessExactRepeatable": fresh.iter().all(|s| s == &fresh[0]),
        "sameAndFreshExactAgreement": same[0] == fresh[0] })
}

fn sequence(worker: &mut Worker, frames: &[Vec<u8>; 3], edit: &Value, control: bool) -> Value {
    for (index, frame) in frames[..2].iter().enumerate() {
        let result = worker.request(frame, index as u64 + 1);
        assert_clean(&result);
    }
    let result = worker.request(&frames[2], 3);
    assert_clean(&result);
    assert_eq!(result["dof"], if control { 0 } else { 4 });
    let geometry = adopt(edit, &result);
    let arc = geometry["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "Arc")
        .unwrap();
    let r = arc["radius"].as_f64().unwrap();
    assert!((r - 14.).abs() <= 1e-6);
    let start = arc["startAngle"].as_f64().unwrap();
    let end = arc["endAngle"].as_f64().unwrap();
    assert!(end > start && end - start < std::f64::consts::TAU);
    if control {
        for coordinate in arc["center"].as_array().unwrap() {
            assert!(coordinate.as_f64().unwrap().abs() <= 1e-6);
        }
        assert!(start.abs() <= 1e-6);
        assert!((end - std::f64::consts::FRAC_PI_2).abs() <= 1e-6);
    }
    let center = [
        arc["center"][0].as_f64().unwrap(),
        arc["center"][1].as_f64().unwrap(),
    ];
    let endpoint = |angle: f64| [center[0] + r * angle.cos(), center[1] + r * angle.sin()];
    json!({ "geometry": geometry, "dof": result["dof"], "state": result["state"],
        "maxResidual": result["maxResidual"], "normalizedMaxResidual": result["normalizedMaxResidual"],
        "conflicting": result["conflicting"], "branch": { "start": start, "end": end, "sweep": end - start },
        "endpoints": { "start": endpoint(start), "end": endpoint(end) },
        "diagnostics": result["diagnostics"], "violations": result["violations"] })
}

fn assert_clean(result: &Value) {
    assert!(
        matches!(
            result["state"].as_str(),
            Some("UnderConstrained" | "FullyConstrained")
        ),
        "{result}"
    );
    assert!(
        result["conflicting"].as_array().unwrap().is_empty(),
        "{result}"
    );
    assert!(result["maxResidual"].as_f64().unwrap().is_finite());
    assert!(
        result["normalizedMaxResidual"].as_f64().unwrap() <= 1.,
        "{result}"
    );
}

fn request(id: u64, args: &Value) -> Vec<u8> {
    encode_frame(
        &serde_json::to_vec(
            &json!({ "v": 1, "t": "req", "id": id, "verb": "SketchUpsert", "args": args }),
        )
        .unwrap(),
        &[],
    )
    .unwrap()
}

fn adopt(args: &Value, result: &Value) -> Value {
    let mut geometry = args.clone();
    for entity in geometry["entities"].as_array_mut().unwrap() {
        let id = entity["id"].as_str().unwrap().to_owned();
        if entity["type"] == "Point" {
            if let Some(at) = result["positions"].get(&id) {
                entity["at"] = at.clone();
            }
            if id == fixture::entity(1000).to_string() {
                let alias = format!("{}.center", fixture::entity(1001));
                if let Some(at) = result["positions"].get(&alias) {
                    entity["at"] = at.clone();
                }
            }
        } else if entity["type"] == "Arc" {
            for member in ["radius", "startAngle", "endAngle"] {
                if let Some(value) = result["curves"][&id].get(member) {
                    entity[member] = value.clone();
                }
            }
        }
    }
    let center = geometry["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == fixture::entity(1000).to_string())
        .unwrap()["at"]
        .clone();
    for entity in geometry["entities"].as_array_mut().unwrap() {
        if entity["type"] == "Arc" {
            entity["center"] = center.clone();
        }
    }
    geometry
}

fn provenance(binary: &Path) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = |args: &[&str]| {
        String::from_utf8(
            Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };
    json!({ "head": git(&["rev-parse", "HEAD"]).trim(), "dirtyState": git(&["status", "--porcelain=v1"]),
        "diffSha256": sha(git(&["diff", "--binary"]).as_bytes()), "workerPath": binary,
        "workerSha256": sha(&fs::read(binary).unwrap()), "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "harnessSha256": {
            "solver_repeatability.rs": sha(include_bytes!("solver_repeatability.rs")),
            "solver_replay/fixture.rs": sha(include_bytes!("solver_replay/fixture.rs")),
            "solver_replay/transport.rs": sha(include_bytes!("solver_replay/transport.rs")),
            "solver_replay/statistics.rs": sha(include_bytes!("solver_replay/statistics.rs")),
        },
        "rustToolchain": fs::read_to_string(root.join("rust-toolchain.toml")).unwrap(),
        "workerCmakeCache": fs::read_to_string(binary.parent().unwrap().join("CMakeCache.txt")).unwrap(),
        "workerCompileCommandsSha256": fs::read(binary.parent().unwrap().join("compile_commands.json")).ok().map(|b| sha(&b)) })
}

fn save(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
