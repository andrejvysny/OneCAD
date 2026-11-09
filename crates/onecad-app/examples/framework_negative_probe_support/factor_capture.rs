//! Snapshot-fenced export of accepted inputs before the group Boolean.
use super::*;
use std::path::Path;
use std::time::Duration;

async fn scope(h: &Harness) -> onecad_app::read::PreviewContext {
    tokio::time::timeout(Duration::from_secs(5), api::preview_context(&h.state))
        .await
        .expect("preview context timeout")
        .expect("preview context")
}

async fn export_one(
    h: &Harness,
    body: BodyId,
    index: usize,
    dir: &Path,
    snapshot: onecad_core::ids::SnapshotId,
) -> serde_json::Value {
    let path = dir.join(format!("input-{index}.brep"));
    let worker = h.state.live_worker().expect("live worker");
    let bodies = [body];
    let export = worker.export_geometry(
        path.to_str().expect("UTF-8 output path"),
        &bodies,
        "brep",
        false,
        Some(snapshot),
    );
    let result = tokio::time::timeout(Duration::from_secs(30), export)
        .await
        .expect("operand export timeout")
        .expect("operand export");
    assert_eq!(result.codec, "brep");
    assert_eq!(result.format, 4);
    assert_eq!(result.solid_count, 1);
    let bytes = std::fs::metadata(&path).expect("exported file").len();
    assert_eq!(bytes, result.bytes_written);
    serde_json::json!({"index":index,"role":if index == 0 {"B"} else if index == 1 {"S"} else {"protector"},
        "protectorIndex":if index >= 2 {Some(index - 2)} else {None},"bodyId":body.to_string(),
        "path":path,"sha256":crate::hash(&path),"bytes":bytes,"codec":result.codec,"format":result.format,"solidCount":result.solid_count})
}

pub(super) async fn capture(
    h: &Harness,
    original: BodyId,
    sharp: BodyId,
    protections: &[BodyId],
    input: &Path,
    out: &Path,
) {
    assert_eq!(protections.len(), 9);
    let dir = out.join("factor-inputs");
    std::fs::create_dir(&dir).expect("fresh operand directory");
    let before = scope(h).await;
    let worker = onecad_app::worker::resolve_worker_path().expect("resolved worker");
    let worker_hash = crate::hash(&worker);
    let input_hash = crate::hash(input);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp");
    let source_hash = crate::hash(&source);
    assert_eq!(worker_hash, crate::WORKER_SHA);
    assert_eq!(input_hash, crate::INPUT_SHA);
    assert_eq!(source_hash, crate::SOURCE_SHA);
    let mut files = Vec::new();
    for (index, body) in [original, sharp]
        .into_iter()
        .chain(protections.iter().copied())
        .enumerate()
    {
        assert_eq!(
            scope(h).await,
            before,
            "publication scope changed before export"
        );
        files.push(export_one(h, body, index, &dir, before.snapshot_id).await);
    }
    assert_eq!(
        scope(h).await,
        before,
        "publication scope changed during export"
    );
    assert_eq!(crate::hash(input), input_hash);
    assert_eq!(crate::hash(&worker), worker_hash);
    let manifest = serde_json::json!({"order":"B, S, nine protectors in authored order","files":files,
        "snapshotId":before.snapshot_id.0,"documentId":before.document_id,"runtimeSession":before.runtime_session,
        "revision":before.revision.0,"workerEpoch":before.worker_epoch.0,"expectedBaseHash":before.expected_base_hash.as_str(),
        "sourcePath":source,"sourceSha256":source_hash,"inputPath":input,"inputSha256":input_hash,
        "workerPath":worker,"workerSha256":worker_hash,"scope":"accepted production-worker operands; no source-equivalence claim"});
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("manifest.json"))
        .unwrap();
    serde_json::to_writer_pretty(file, &manifest).unwrap();
}
