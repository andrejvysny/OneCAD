//! Native negative retention experiment; cold 30-second timeout remains a failure.
//! Optional `--factor-cutter` constructs the complete nine-protector cutter.
//! `--profile-cutter` replaces five cylinder complements with native quarter profiles.
//! This is diagnostic only and does not establish source equivalence.
#[allow(dead_code)]
mod framework_negative_probe_support;
#[allow(dead_code)]
#[path = "framework_negative_probe_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, FeatureStatus, RegenFinished},
    host::AppEvent,
};
use onecad_core::{
    document::record::{KnownOperation, Operation},
    ids::BodyId,
    io::container::ContainerReader,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

const INPUT_SHA: &str = "42ed28779aab0a220288ca27c7876c759665fc4659068cb6a2b367774c255a5a";
const WORKER_SHA: &str = "4272493274b789539a0e731afe0d45084e344ccf646f46bec35af6561a3fcfb0";
const SOURCE_SHA: &str = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09";
fn hash(path: &Path) -> String {
    let mut file = std::fs::File::open(path).expect("open file for hashing");
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).expect("read file for hashing");
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}
fn next_uuid(value: &Value, max: &mut u128) {
    match value {
        Value::String(s) => {
            if let Ok(id) = Uuid::parse_str(s) {
                let n = id.as_u128();
                if (0xfabc_0000..(1_u128 << 64)).contains(&n) {
                    *max = (*max).max(n);
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                next_uuid(v, max);
            }
        }
        Value::Object(items) => {
            for (k, v) in items {
                next_uuid(&Value::String(k.clone()), max);
                next_uuid(v, max);
            }
        }
        _ => {}
    }
}
fn seed(input: &Path) -> u128 {
    let loaded = ContainerReader::open(input).unwrap();
    assert!(
        loaded.import_blobs().is_empty(),
        "native history cannot contain imported substitutes"
    );
    assert_eq!(loaded.document().timeline.records().len(), 204);
    assert!(
        loaded
            .document()
            .timeline
            .records()
            .iter()
            .all(|r| !matches!(r.op, Operation::Known(KnownOperation::ImportStep(_)))),
        "native history cannot include ImportStep"
    );
    let value = serde_json::to_value(loaded.document()).unwrap();
    let mut max = 0xfabc_0000;
    next_uuid(&value, &mut max);
    max.checked_sub(0xfabc_0000)
        .unwrap()
        .checked_add(1)
        .unwrap()
}
fn matching(h: &Harness, p: &DocumentProjection) -> Option<RegenFinished> {
    h.events
        .records()
        .into_iter()
        .rev()
        .find_map(|event| match event {
            AppEvent::RegenFinished(r)
                if r.runtime_session == p.runtime_session
                    && r.source_revision == p.revision
                    && r.outcome != "superseded" =>
            {
                Some(r)
            }
            _ => None,
        })
}
async fn terminal(
    h: &Harness,
    p: &DocumentProjection,
    deadline: tokio::time::Instant,
) -> Option<RegenFinished> {
    tokio::time::timeout_at(deadline, async {
        loop {
            if let Some(report) = matching(h, p) {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .ok()
}
fn summary(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
async fn cold(h: &Harness, input: &Path, status: &Path) -> Option<BodyId> {
    let start = tokio::time::Instant::now();
    let strict = start + Duration::from_secs(30);
    let open = tokio::time::timeout_at(
        strict,
        api::open_document(
            &h.state,
            h.host.clone(),
            input.to_string_lossy().into_owned(),
            None,
        ),
    )
    .await;
    if !matches!(open, Ok(Ok(_))) {
        summary(
            status,
            &json!({"strict":"open_failed_or_timeout","detail":format!("{open:?}")}),
        );
        return None;
    }
    let p = match tokio::time::timeout_at(strict, api::get_projection(&h.state)).await {
        Ok(Ok(p)) => p,
        other => {
            summary(
                status,
                &json!({"strict":"projection_failed_or_timeout","detail":format!("{other:?}")}),
            );
            return None;
        }
    };
    let first = terminal(h, &p, strict).await;
    let strict_result = match &first {
        Some(r) => json!({"status":"terminal","report":r}),
        None => json!({"status":"terminal_timeout","acceptance":"failed"}),
    };
    summary(
        &status.parent().unwrap().join("strict-cold.json"),
        &json!({"strictCold":strict_result,"diagnosticOnly":true}),
    );
    summary(
        status,
        &json!({"strictCold":strict_result,"diagnosticOnly":true,"stage":"late_observation"}),
    );
    let report = match first {
        Some(r) => Some(r),
        None => terminal(h, &p, start + Duration::from_secs(60)).await,
    };
    let latest =
        match tokio::time::timeout(Duration::from_secs(5), api::get_projection(&h.state)).await {
            Ok(Ok(p)) => p,
            other => {
                summary(
                    status,
                    &json!({"strictCold":strict_result,"lateTerminal":report,
            "diagnosticOnly":true,"stage":"projection_failure","detail":format!("{other:?}")}),
                );
                return None;
            }
        };
    summary(
        status,
        &json!({"strictCold":strict_result,"lateTerminal":report,"diagnosticOnly":true,"stage":"cold_complete"}),
    );
    let r = report?;
    if r.outcome != "published"
        || !r.failed_steps.is_empty()
        || !r.repair_steps.is_empty()
        || latest.runtime_session != p.runtime_session
        || latest.revision != p.revision
        || latest
            .features
            .iter()
            .any(|f| f.status != FeatureStatus::Ok)
        || latest.bodies.len() != 1
    {
        return None;
    }
    latest.bodies.keys().next()?.parse().ok()
}
async fn author(
    h: Arc<Harness>,
    original: BodyId,
    next: u128,
    out: PathBuf,
    input: PathBuf,
    factor_cutter: bool,
    profile_cutter: bool,
) -> Result<Value, String> {
    let query = api::query_mass_properties(
        &h.state,
        onecad_app::worker::wire::body_id_wire(original),
        None,
    );
    let initial_mass = tokio::time::timeout(Duration::from_secs(10), query)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    summary(
        &out.join("initial-live.json"),
        &json!({"bodyId":original.to_string(),"mass":initial_mass,
        "nativeFeatureCount":204,"importCount":0,"diagnosticOnly":true}),
    );
    let final_bodies = if factor_cutter {
        Some(
            framework_negative_probe_support::factor_cutter(
                &h,
                original,
                next,
                &out,
                &input,
                profile_cutter,
            )
            .await,
        )
    } else {
        framework_negative_probe_support::apply_negative(&h, original, next, &out).await;
        None
    };
    let p = tokio::time::timeout(Duration::from_secs(5), api::get_projection(&h.state))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut mass = Vec::new();
    let mut meshes = Vec::new();
    if let Some(ref expected) = final_bodies {
        for body in expected {
            let id = body.to_string();
            if !p.bodies.contains_key(&id) {
                return Err(format!("reported final body missing: {id}"));
            }
            let request = api::get_mesh(
                &h.state,
                id.clone(),
                "fine".into(),
                None,
                Some(p.runtime_session.clone()),
            );
            let blob = tokio::time::timeout(Duration::from_secs(10), request)
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let mesh = onecad_protocol::mesh::MeshView::decode(
                &blob,
                onecad_protocol::mesh::MeshLimits::default(),
            )
            .map_err(|e| format!("mesh: {e:?}"))?;
            if mesh.header().triangle_count == 0 {
                return Err(format!("empty final mesh for {id}"));
            }
            meshes.push(json!({"bodyId":id,"sha256":format!("{:x}",Sha256::digest(&blob)),"triangles":mesh.header().triangle_count}));
        }
    }
    for id in p.bodies.keys() {
        let body: BodyId = id.parse().map_err(|e| format!("body id: {e}"))?;
        let query = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        );
        mass.push(
            tokio::time::timeout(Duration::from_secs(10), query)
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?,
        );
    }
    let save_path = out.join("native-negative.onecad");
    let save = api::save_document(
        &h.state,
        h.host.clone(),
        Some(save_path.to_string_lossy().into_owned()),
        None,
    );
    let saved = tokio::time::timeout(Duration::from_secs(30), save)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if !saved.clean {
        return Err("save did not persist clean revision".into());
    }
    let saved_doc = ContainerReader::open(&save_path).map_err(|e| e.to_string())?;
    let import_count = saved_doc
        .document()
        .timeline
        .records()
        .iter()
        .filter(|r| matches!(r.op, Operation::Known(KnownOperation::ImportStep(_))))
        .count();
    if import_count != 0 || !saved_doc.import_blobs().is_empty() {
        return Err("saved history contains imports".into());
    }
    Ok(
        json!({"stage":"saved","mass":mass,"checkedMeshes":meshes,"bodyIds":p.bodies.keys().collect::<Vec<_>>(),
        "nativeFeatureCount":saved_doc.document().timeline.records().len(),"importCount":import_count,
        "outputSha256":hash(&save_path)}),
    )
}
async fn cleanup(h: &Harness) -> Value {
    let worker = h.state.live_worker();
    let close = tokio::time::timeout(
        Duration::from_secs(10),
        api::close_document(&h.state, h.host.clone()),
    )
    .await;
    h.state.retire_all();
    let torn_down = match worker {
        Some(w) => Some(w.wait_torn_down(Duration::from_secs(10)).await),
        None => None,
    };
    json!({"close":format!("{close:?}"),"workerTornDown":torn_down})
}
async fn run(input: &Path, out: &Path, factor_cutter: bool, profile_cutter: bool) {
    assert!(
        std::env::vars_os().all(|(key, _)| !key.to_string_lossy().starts_with("ONECAD_FRAMEWORK_")),
        "framework recipe overrides are not allowed"
    );
    assert_eq!(hash(input), INPUT_SHA);
    let worker = onecad_app::worker::resolve_worker_path().expect("worker resolver");
    assert_eq!(hash(&worker), WORKER_SHA);
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp");
    assert_eq!(hash(&source), SOURCE_SHA);
    let next = seed(input);
    let h = Arc::new(Harness::new(out).await);
    let status = out.join("status.json");
    let original = cold(&h, input, &status).await;
    let strict = std::fs::read(out.join("strict-cold.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .or_else(|| {
            std::fs::read(&status)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        })
        .unwrap_or_else(|| json!({"status":"cold_evidence_missing"}));
    summary(
        &status,
        &json!({"strictCold":strict,"diagnosticOnly":true,"stage":"authoring_or_cleanup","seed":next}),
    );
    let result = if let Some(original) = original {
        match h.manual_save("cold-published").await {
            Err(error) => json!({"stage":"initial_manual_save_failure","error":error}),
            Ok(()) => {
                let task_h = h.clone();
                let task_out = out.to_path_buf();
                let task_input = input.to_path_buf();
                match tokio::spawn(async move {
                    author(
                        task_h,
                        original,
                        next,
                        task_out,
                        task_input,
                        factor_cutter,
                        profile_cutter,
                    )
                    .await
                })
                .await
                {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => json!({"stage":"authoring_error","error":error}),
                    Err(error) => json!({"stage":"authoring_panic","error":error.to_string()}),
                }
            }
        }
    } else {
        json!({"stage":"cold_incomplete"})
    };
    let cleanup_result = cleanup(&h).await;
    summary(
        &status,
        &json!({"strictCold":strict,"diagnosticOnly":true,"result":result,
        "cleanup":cleanup_result,"mode":if profile_cutter {"profile-cutter"} else if factor_cutter {"factor-cutter"} else {"retention"},"eventRecordsDropped":h.events.dropped(),
        "priorUncachedEditFailure":"/private/tmp/onecad-resume-evidence/framework-native-negative-restricted-v1/status.json","inputSha256After":hash(input),"workerPath":worker,
        "workerSha256After":hash(&worker),"sourceSha256":SOURCE_SHA,"acceptance":false}),
    );
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    assert!(
        args.len() == 3 || (args.len() == 4 && (args[3] == "--factor-cutter" || args[3] == "--profile-cutter")),
        "usage: framework_negative_probe INPUT.onecad NEW_OUTPUT_DIR [--factor-cutter|--profile-cutter]"
    );
    let input = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir(&out).expect("output directory must be new");
    let temp = out.join("temp");
    std::fs::create_dir(&temp).unwrap();
    std::env::set_var("TMPDIR", temp);
    std::env::set_var("ONECAD_LIBRARY_ROOT", out.join("library"));
    std::env::set_var("ONECAD_LOG_DIR", out.join("logs"));
    let _guard = onecad_app::logging::init().expect("diagnostic logs");
    let profile_cutter = args.len() == 4 && args[3] == "--profile-cutter";
    run(&input, &out, args.len() == 4, profile_cutter).await;
    std::process::ExitCode::FAILURE
}
