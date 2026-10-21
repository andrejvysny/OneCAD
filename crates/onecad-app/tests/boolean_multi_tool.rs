//! Multi-tool / retained-tool standalone Boolean (SCHEMA §7.3, ONECAD-362) against
//! the REAL C++ worker, through the app's `DocumentRuntime`.
//!
//! Independent oracles: analytic box volumes (MESH1 divergence theorem), the
//! published body-id set, per-body worker signatures and exact BRep
//! `QueryBodyTopology` counts. Geometry, in sketch space (every box starts at z=0):
//!
//! * T `[0,0]..[40,40]` × 20 = 32000 — the target;
//! * A `[5,5]..[15,15]` × 30, B `[10,10]..[20,20]` × 30 — through-tools that
//!   overlap each other (A∩B∩T = 5·5·20 = 500);
//! * C `[12,12]..[22,14]` × 30 — touches A (A∪C = 3000 + 600 − 180 = 3420);
//! * D `[100,100]..[105,105]` × 5 — far from everything;
//! * N `[5,5]..[10,10]` × 30 — nested in A, sharing its x=5, y=5 and base planes.

#[path = "boolean_multi_tool/support.rs"]
mod support;

use onecad_core::document::record::{BooleanOp, OperationRecord};
use onecad_core::edit::EditCommand;
use onecad_core::ids::BodyId;
use onecad_core::io::container::ContainerReader;
use onecad_core::regen::Lod;
use tempfile::TempDir;

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::worker::{ElementQuery, PreviewEngine, WorkerManager};

use support::*;

const T: [f64; 4] = [0.0, 0.0, 40.0, 40.0];
const A: [f64; 4] = [5.0, 5.0, 10.0, 10.0];
const B: [f64; 4] = [10.0, 10.0, 10.0, 10.0];
const C: [f64; 4] = [12.0, 12.0, 10.0, 2.0];
const D: [f64; 4] = [100.0, 100.0, 5.0, 5.0];
const N: [f64; 4] = [5.0, 5.0, 5.0, 5.0];
const OP_BOOLEAN: u128 = 0x200;
const OP_LATER: u128 = 0x210;

struct Scene {
    rt: DocumentRuntime,
    wm: WorkerManager,
    t: BodyId,
    a: BodyId,
    b: BodyId,
}

async fn scene(third: [f64; 4], third_height: f64) -> Option<Scene> {
    let bin = real_worker()?;
    let wm = spawn_worker(bin).await;
    // The app's capability gate and the worker's Hello must agree on the name, or
    // every array-form Boolean would fail as an unattributed protocol error.
    let hello = wm.hello().expect("hello");
    assert!(
        hello.capabilities.iter().any(|c| c == "op.boolean.tools"),
        "real worker advertises op.boolean.tools: {:?}",
        hello.capabilities
    );
    let mut rt = runtime_over(&wm);
    let t = add_box(&mut rt, 0x100, T, 20.0);
    let a = add_box(&mut rt, 0x110, A, 30.0);
    let b = add_box(&mut rt, 0x120, third, third_height);
    let base = regen_all(&mut rt).await;
    assert_eq!(published(&base, "scene").bodies.len(), 3, "three boxes");
    Some(Scene { rt, wm, t, a, b })
}

fn sorted(ids: &[BodyId]) -> Vec<String> {
    let mut v: Vec<String> = ids.iter().map(ToString::to_string).collect();
    v.sort();
    v
}

async fn preview(s: &Scene, record: &OperationRecord) -> onecad_app::dto::PreviewResultDto {
    s.wm.preview_op(
        record.op.clone(),
        record.record_id.to_string(),
        None,
        None,
        Lod::Coarse,
    )
    .await
    .expect("PreviewOp")
}

async fn faces(wm: &WorkerManager, body: BodyId) -> (u32, u32) {
    let topo = wm
        .query_body_topology(body, body.to_string())
        .await
        .expect("QueryBodyTopology");
    (topo.solid_count, topo.face_count)
}

/// Case 1: two consumed tools, preview == commit, for every operation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_tools_preview_matches_commit_for_every_operation() {
    for (op, expected) in [
        (BooleanOp::Cut, 28_500.0),
        (BooleanOp::Union, 33_750.0),
        (BooleanOp::Intersect, 500.0),
    ] {
        let Some(mut s) = scene(B, 30.0).await else {
            return;
        };
        let what = format!("{op:?}");
        let before = signatures(&head(&s.rt));
        let revision = s.rt.projection().revision;
        let record = boolean_record(OP_BOOLEAN, boolean_op(op, s.t, &[s.a, s.b], false));
        let p = preview(&s, &record).await;
        assert_eq!(p.changed_bodies, vec![s.t.to_string()], "{what}: changed");
        let mut deleted = p.deleted_bodies.clone();
        deleted.sort();
        assert_eq!(deleted, sorted(&[s.a, s.b]), "{what}: both tools consumed");
        assert_volume(blob_volume(&p.bodies[0].mesh), expected, &what);
        assert_eq!(
            signatures(&head(&s.rt)),
            before,
            "{what}: preview left head"
        );
        assert_eq!(s.rt.projection().revision, revision, "{what}: no revision");

        add_op(&mut s.rt, record);
        let report = regen_all(&mut s.rt).await;
        let snap = published(&report, &what);
        assert_eq!(snap.bodies.len(), 1, "{what}: only the target remains");
        let committed = volume(&mut s.rt, s.t).await;
        assert_volume(committed, expected, &format!("{what} commit"));
        assert_eq!(faces(&s.wm, s.t).await.0, 1, "{what}: one solid");
        s.wm.shutdown().await;
    }
}

/// Case 2: keepTools — the tools are inputs, never outputs or deletions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_tools_survive_bit_identical() {
    let Some(mut s) = scene(B, 30.0).await else {
        return;
    };
    let mut meshes = Vec::new();
    for tool in [s.a, s.b] {
        meshes.push(s.rt.get_mesh(tool, Lod::Coarse, None).await.expect("mesh"));
    }
    let record = boolean_record(
        OP_BOOLEAN,
        boolean_op(BooleanOp::Cut, s.t, &[s.a, s.b], true),
    );
    let p = preview(&s, &record).await;
    assert_eq!(p.changed_bodies, vec![s.t.to_string()]);
    assert!(p.deleted_bodies.is_empty(), "retained: nothing deleted");

    add_op(&mut s.rt, record);
    let report = regen_all(&mut s.rt).await;
    let after = signatures(published(&report, "keep commit"));
    assert_eq!(after.len(), 3, "target and both tools published");
    for (tool, before) in [s.a, s.b].into_iter().zip(meshes) {
        // Snapshot signatures are whole-model (executor.rs), so the per-body
        // oracle is the tool's own MESH1 bytes.
        let now = s.rt.get_mesh(tool, Lod::Coarse, None).await.expect("mesh");
        assert_eq!(now, before, "retained tool tessellation byte-identical");
        assert_eq!(
            faces(&s.wm, tool).await,
            (1, 6),
            "retained tool is still a box"
        );
        assert_volume(volume(&mut s.rt, tool).await, 3000.0, "retained tool");
    }
    assert_volume(volume(&mut s.rt, s.t).await, 28_500.0, "kept Cut target");
    s.wm.shutdown().await;
}

async fn assert_downstream_state(rt: &mut DocumentRuntime, s: (BodyId, BodyId, BodyId), t: f64) {
    let snap = head(rt);
    let ids: Vec<String> = snap.bodies.iter().map(|b| b.body.to_string()).collect();
    assert_eq!(snap.bodies.len(), 3, "T, A∪C, B published: {ids:?}");
    assert_volume(volume(rt, s.0).await, t, "target");
    assert_volume(volume(rt, s.1).await, 3_420.0, "later Union on retained A");
    assert_volume(volume(rt, s.2).await, 3_000.0, "retained B");
}

/// Case 3: a later op on a retained tool through edit, suppression, Undo/Redo
/// and save + fresh-worker reopen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_tool_feeds_later_op_through_edit_undo_and_reopen() {
    let Some(bin) = real_worker() else {
        return;
    };
    let Some(mut s) = scene(B, 30.0).await else {
        return;
    };
    let ids = (s.t, s.a, s.b);
    let kept = boolean_op(BooleanOp::Cut, s.t, &[s.a, s.b], true);
    add_op(&mut s.rt, boolean_record(OP_BOOLEAN, kept.clone()));
    let c = add_box(&mut s.rt, 0x130, C, 30.0);
    let later = boolean_op(BooleanOp::Union, s.a, &[c], false);
    add_op(&mut s.rt, boolean_record(OP_LATER, later));
    regen_all(&mut s.rt).await;
    assert_downstream_state(&mut s.rt, ids, 28_500.0).await;

    // keep → consume: A is deleted by the Boolean, so the later op's target no
    // longer exists. The edit is accepted (one history entry); the regen publishes
    // the prefix and fails the later op at its own step, naming the missing body.
    let revision = s.rt.projection().revision;
    let consume = boolean_op(BooleanOp::Cut, s.t, &[s.a, s.b], false);
    s.rt.apply(EditCommand::UpdateOperationParams {
        record: rid(OP_BOOLEAN),
        op: consume,
    })
    .expect("keep → consume is a legal edit");
    assert_eq!(s.rt.projection().revision, revision + 1, "one edit");
    let report = regen_all(&mut s.rt).await;
    let snap = published(&report, "consume");
    assert_eq!(
        snap.step_index,
        Some(8),
        "prefix through extrude C (step 8)"
    );
    let mut published_ids: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    published_ids.sort();
    assert_eq!(
        published_ids,
        vec![s.t, c],
        "A and B consumed; C left unconsumed"
    );
    let failed: Vec<_> = report
        .failed_steps
        .iter()
        .map(|f| f.record_id.clone())
        .collect();
    assert_eq!(
        failed,
        vec![rid(OP_LATER).to_string()],
        "the later op fails"
    );
    assert_eq!(
        report.failed_steps[0].message,
        format!("Boolean target body not found: body_{}", s.a),
        "REF_UNRESOLVED names the consumed target"
    );
    assert_eq!(report.failed_steps[0].diagnostics[0].code, "REF_UNRESOLVED");

    // Undo restores retention and the later op; Redo re-applies the failure.
    s.rt.undo().expect("undo");
    regen_all(&mut s.rt).await;
    assert_downstream_state(&mut s.rt, ids, 28_500.0).await;
    s.rt.redo().expect("redo").expect("redo entry");
    let redo = regen_all(&mut s.rt).await;
    assert_eq!(redo.failed_steps.len(), 1, "redo fails the later op again");
    s.rt.undo().expect("undo again");

    // Cut → Union, still retained: the later op is unaffected.
    let union = boolean_op(BooleanOp::Union, s.t, &[s.a, s.b], true);
    s.rt.apply(EditCommand::UpdateOperationParams {
        record: rid(OP_BOOLEAN),
        op: union,
    })
    .expect("Cut → Union");
    regen_all(&mut s.rt).await;
    assert_downstream_state(&mut s.rt, ids, 33_750.0).await;
    s.rt.undo().expect("undo Union");

    // Suppressing the Boolean leaves the target whole and the tools untouched.
    for (suppressed, t) in [(true, 32_000.0), (false, 28_500.0)] {
        s.rt.apply(EditCommand::SetOperationSuppression {
            record: rid(OP_BOOLEAN),
            suppressed,
            cascade: false,
        })
        .expect("suppression");
        regen_all(&mut s.rt).await;
        assert_downstream_state(&mut s.rt, ids, t).await;
    }

    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("kept.onecad");
    s.rt.save(&path, save_meta()).expect("save");
    let saved = ContainerReader::open(&path).expect("read back");
    let text = serde_json::to_string(saved.document()).expect("json");
    assert!(text.contains("\"toolBodyIds\"") && text.contains("\"keepTools\":true"));
    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &path);
    regen_all(&mut rt2).await;
    assert_downstream_state(&mut rt2, ids, 28_500.0).await;
    wm2.shutdown().await;
    s.wm.shutdown().await;
}

/// Case 4: whole-op refusals leave every body and the revision untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusals_leave_every_body_untouched() {
    for (op, code) in [
        (BooleanOp::Cut, "BOOLEAN_TOOL_NO_EFFECT"),
        (BooleanOp::Union, "BOOLEAN_DISJOINT_RESULT"),
        (BooleanOp::Intersect, "BOOLEAN_EMPTY_RESULT"),
    ] {
        let Some(mut s) = scene(D, 5.0).await else {
            return;
        };
        let before = signatures(&head(&s.rt));
        let undo_depth = s.rt.undo_depth();
        let record = boolean_record(OP_BOOLEAN, boolean_op(op, s.t, &[s.a, s.b], false));
        let refused =
            s.wm.preview_op(
                record.op.clone(),
                record.record_id.to_string(),
                None,
                None,
                Lod::Coarse,
            )
            .await;
        let refused = refused.expect_err("preview refuses").to_string();
        assert!(refused.contains("Boolean"), "{op:?}: {refused}");
        assert_eq!(signatures(&head(&s.rt)), before, "{op:?}: head untouched");
        assert_eq!(
            s.rt.undo_depth(),
            undo_depth,
            "{op:?}: preview adds no history"
        );

        add_op(&mut s.rt, record);
        let report = regen_all(&mut s.rt).await;
        let snap = published(&report, "refusal");
        // The whole-model geometry signature: any body change would flip it.
        assert_eq!(signatures(snap), before, "{op:?}: model geometry unchanged");
        let failed = &report.failed_steps;
        assert_eq!(failed.len(), 1, "{op:?}: the Boolean step fails");
        assert!(
            failed[0].diagnostics.iter().any(|d| d.code == code),
            "{op:?}: {code} in {:?}",
            failed[0].diagnostics
        );
        s.wm.shutdown().await;
    }
}

/// Case 4b: same-sketch-plane tools (the UI case). N shares walls and the base
/// plane with A, so it removes nothing A does not: the WHOLE op refuses, naming N.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coplanar_nested_tool_refuses_whole_op() {
    for keep in [false, true] {
        let Some(mut s) = scene(N, 30.0).await else {
            return;
        };
        let before = signatures(&head(&s.rt));
        let op = boolean_op(BooleanOp::Cut, s.t, &[s.a, s.b], keep);
        add_op(&mut s.rt, boolean_record(OP_BOOLEAN, op));
        let report = regen_all(&mut s.rt).await;
        assert_eq!(
            signatures(published(&report, "coplanar")),
            before,
            "keep={keep}: model geometry unchanged"
        );
        let failed = &report.failed_steps;
        assert_eq!(failed.len(), 1, "keep={keep}: the Boolean step fails");
        let diagnostic = failed[0]
            .diagnostics
            .iter()
            .find(|d| d.code == "BOOLEAN_TOOL_NO_EFFECT")
            .unwrap_or_else(|| panic!("keep={keep}: {:?}", failed[0].diagnostics));
        assert!(
            failed[0].message.contains(&format!("body_{}", s.b)),
            "keep={keep}: names the nested tool: {} / {diagnostic:?}",
            failed[0].message
        );
        s.wm.shutdown().await;
    }
}

/// Case 5: a legacy single-tool document keeps its exact persisted bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_single_tool_document_round_trips_byte_identical() {
    let Some(bin) = real_worker() else {
        return;
    };
    let Some(mut s) = scene(B, 30.0).await else {
        return;
    };
    let legacy = boolean_op(BooleanOp::Cut, s.t, &[s.a], false);
    add_op(&mut s.rt, boolean_record(OP_BOOLEAN, legacy));
    regen_all(&mut s.rt).await;
    let tmp = TempDir::new().expect("temp dir");
    let first = tmp.path().join("legacy.onecad");
    s.rt.save(&first, save_meta()).expect("save");
    let first_text =
        serde_json::to_string(ContainerReader::open(&first).unwrap().document()).unwrap();
    assert!(first_text.contains("\"toolBodyId\"") && !first_text.contains("toolBodyIds"));
    assert!(!first_text.contains("keepTools"));

    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &first);
    regen_all(&mut rt2).await;
    assert_volume(volume(&mut rt2, s.t).await, 30_000.0, "legacy Cut reopened");
    let second = tmp.path().join("legacy-again.onecad");
    rt2.save(&second, save_meta()).expect("save again");
    let second_text =
        serde_json::to_string(ContainerReader::open(&second).unwrap().document()).unwrap();
    assert_eq!(first_text, second_text, "legacy document bytes are stable");
    wm2.shutdown().await;
    s.wm.shutdown().await;
}
