//! Multi-point Hole volumes, preview ≡ commit, all-or-nothing refusals,
//! Undo/Redo and save → fresh worker → reopen.

use onecad_core::document::record::{HolePlacement, HoleType};
use onecad_core::document::refs::ElementKind;
use onecad_core::document::variables::{Scalar, Unit, Variable};
use onecad_core::edit::EditCommand;
use onecad_core::ids::VariableId;
use onecad_core::regen::{CancelToken, Lod};

use onecad_app::dto::FeatureStatus;
use onecad_app::worker::PreviewEngine;
use onecad_protocol::mesh::validate_mesh_blob;

use crate::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn many_holes_in_one_step_remove_exact_volumes_and_preview_matches_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let plate_a = build_plate(&mut rt, SKETCH_A, EXTRUDE_A, 0.0).await;
    let plate_b = build_plate(&mut rt, SKETCH_B, EXTRUDE_B, 100.0).await;

    let through = holes(
        plate_a,
        top(-10.0, 30.0),
        &[top(-10.0, 10.0), top(-10.0, 30.0), top(-10.0, 50.0)],
        4.0,
    );
    assert_eq!(through.result_policy_version, Some(3));
    let candidate = hole_record(HOLE_A, through);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("V3 Hole PreviewOp");
    assert!(
        preview.needs_repair.is_empty(),
        "{:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.changed_bodies,
        vec![plate_a.to_string()],
        "ONE modified host"
    );
    let preview_faces = validate_mesh_blob(&preview.bodies[0].mesh)
        .expect("preview MESH1")
        .face_count;
    assert_volume(
        exact_volume(&wm, plate_a).await,
        PLATE_VOLUME,
        "preview leaves the head solid",
    );

    add_op(&mut rt, candidate);
    let mut counterbores = holes(
        plate_b,
        top(-10.0, 130.0),
        &[top(-10.0, 115.0), top(-10.0, 145.0)],
        4.0,
    );
    counterbores.hole_type = HoleType::Counterbore;
    counterbores.depth = Some(Scalar::new(6.0));
    counterbores.cb_diameter = Some(Scalar::new(8.0));
    counterbores.cb_depth = Some(Scalar::new(2.0));
    add_op(&mut rt, hole_record(HOLE_B, counterbores));
    let rep = clean_regen(&mut rt).await;
    let snap = published(&rep, "two multi-hole records");
    assert_eq!(
        snap.repair_summary.needs_repair_count,
        0,
        "{:?}",
        rt.repair_items()
    );
    assert_eq!(rt.head_body_ids().len(), 2, "holes mint nothing");

    assert_volume(
        exact_volume(&wm, plate_a).await,
        PLATE_VOLUME - 3.0 * cylinder(4.0, TOP),
        "3 through Ø4",
    );
    let per_counterbore = cylinder(4.0, 6.0 - 2.0) + cylinder(8.0, 2.0);
    assert_volume(
        exact_volume(&wm, plate_b).await,
        PLATE_VOLUME - 2.0 * per_counterbore,
        "2 blind counterbores",
    );
    let committed = rt.get_mesh(plate_a, Lod::Coarse, None).await.expect("mesh");
    let committed_faces = validate_mesh_blob(&committed)
        .expect("committed MESH1")
        .face_count;
    assert_eq!(
        committed_faces, preview_faces,
        "preview and commit have the same faces"
    );
    assert_eq!(committed_faces, 6 + 3, "one wall face per through hole");
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_bad_point_refuses_the_whole_feature_with_its_index() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let plate = build_plate(&mut rt, SKETCH_A, EXTRUDE_A, 0.0).await;
    // points[1] is on the top PLANE but beyond the plate's +x edge.
    let params = holes(
        plate,
        top(-10.0, 30.0),
        &[top(-10.0, 10.0), top(5.0, 30.0), top(-10.0, 50.0)],
        4.0,
    );
    add_op(&mut rt, hole_record(HOLE_A, params));
    let _ = clean_regen(&mut rt).await;

    let row = feature_row(&rt, HOLE_A);
    assert_eq!(
        row.status,
        FeatureStatus::Error,
        "the Hole step fails as a whole"
    );
    let diag = row
        .diagnostics
        .iter()
        .find(|d| d.reason_code.as_deref() == Some("HOLE_POINT_OUTSIDE_FACE"))
        .unwrap_or_else(|| panic!("named refusal, got {:?}", row.diagnostics));
    assert_eq!(diag.code, "OP_FAILED");
    assert_eq!(
        diag.evidence
            .as_ref()
            .and_then(|e| e["pointIndex"].as_u64()),
        Some(1)
    );
    assert_volume(
        exact_volume(&wm, plate).await,
        PLATE_VOLUME,
        "no partial holes, prefix intact",
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expression_driven_overlap_is_refused_by_the_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let plate = build_plate(&mut rt, SKETCH_A, EXTRUDE_A, 0.0).await;
    let d = VariableId::new();
    let variable = Variable {
        id: d,
        name: "d".into(),
        value: Scalar::new(4.0),
        unit: Unit::Mm,
    };
    rt.apply(EditCommand::AddVariable { variable })
        .expect("AddVariable d");
    let face = promote_top_face(&mut rt, &wm, plate).await;
    let mut params = holes(
        plate,
        top(-10.0, 30.0),
        &[top(-10.0, 20.0), top(-10.0, 30.0)],
        4.0,
    );
    params.face = anchored_ref(plate, &face, ElementKind::Face, top(-10.0, 30.0));
    params.diameter = Scalar::with_expr(4.0, "d");
    add_op(&mut rt, hole_record(HOLE_A, params));
    published(&clean_regen(&mut rt).await, "Ø4 pair 10 mm apart");
    assert_volume(
        exact_volume(&wm, plate).await,
        PLATE_VOLUME - 2.0 * cylinder(4.0, TOP),
        "two Ø4",
    );

    // Ø12 at 10 mm spacing: the variable edit never re-runs Hole authoring
    // validation, so the worker's own spacing rule is the backstop.
    rt.apply(EditCommand::SetVariable {
        variable: d,
        value: Scalar::new(12.0),
    })
    .expect("SetVariable d = 12");
    let _ = regen_from(&mut rt, 0).await;
    let row = feature_row(&rt, HOLE_A);
    assert_eq!(row.status, FeatureStatus::Error);
    let diag = row
        .diagnostics
        .iter()
        .find(|d| d.reason_code.as_deref() == Some("HOLE_POINTS_OVERLAP"))
        .unwrap_or_else(|| panic!("worker overlap refusal, got {:?}", row.diagnostics));
    let evidence = diag.evidence.clone().unwrap_or_default();
    assert_eq!(
        (
            evidence["pointIndex"].as_u64(),
            evidence["otherPointIndex"].as_u64()
        ),
        (Some(0), Some(1))
    );
    assert_volume(
        exact_volume(&wm, plate).await,
        PLATE_VOLUME,
        "refusal leaves the prefix",
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn placement_edits_undo_and_redo_across_one_three_two() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let plate = build_plate(&mut rt, SKETCH_A, EXTRUDE_A, 0.0).await;
    let one = holes(plate, top(-10.0, 30.0), &[top(-10.0, 30.0)], 4.0);
    assert!(
        one.points.is_none(),
        "one placement stays in the legacy shape"
    );
    add_op(&mut rt, hole_record(HOLE_A, one.clone()));
    published(&clean_regen(&mut rt).await, "1 hole");

    let mut three = one.clone();
    let mut list = three.placements();
    list.push(HolePlacement {
        id: None,
        at: top(-10.0, 10.0),
    });
    list.push(HolePlacement {
        id: None,
        at: top(-10.0, 50.0),
    });
    three.set_placements(list).unwrap();
    edit_hole(&mut rt, HOLE_A, three.clone());
    published(&regen_from(&mut rt, 2).await, "3 holes");

    let mut two = three.clone();
    let mut list = two.placements();
    list.remove(1);
    two.set_placements(list).unwrap();
    edit_hole(&mut rt, HOLE_A, two.clone());
    published(&regen_from(&mut rt, 2).await, "2 holes");

    let volume = |n: f64| PLATE_VOLUME - n * cylinder(4.0, TOP);
    assert_volume(
        exact_volume(&wm, plate).await,
        volume(2.0),
        "after 1 → 3 → 2",
    );
    for (expected, n, step) in [(&three, 3.0, "undo to 3"), (&one, 1.0, "undo to 1")] {
        let report = rt.undo().expect("undo");
        rt.run_regen(report.regen.expect("revert regen"), CancelToken::new())
            .await;
        assert_eq!(
            &stored_hole(&rt, HOLE_A),
            expected,
            "{step}: record restored"
        );
        assert_volume(exact_volume(&wm, plate).await, volume(n), step);
    }
    for (expected, n, step) in [(&three, 3.0, "redo to 3"), (&two, 2.0, "redo to 2")] {
        let report = rt.redo().expect("redo").expect("something to redo");
        rt.run_regen(report.regen.expect("revert regen"), CancelToken::new())
            .await;
        assert_eq!(
            &stored_hole(&rt, HOLE_A),
            expected,
            "{step}: record restored"
        );
        assert_volume(exact_volume(&wm, plate).await, volume(n), step);
    }
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_multi_hole_reopens_identically_on_a_fresh_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("holes.onecad");
    let (plate, volume, params) = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let plate = build_plate(&mut rt, SKETCH_A, EXTRUDE_A, 0.0).await;
        let params = holes(
            plate,
            top(-10.0, 30.0),
            &[top(-10.0, 10.0), top(-10.0, 30.0), top(-10.0, 50.0)],
            4.0,
        );
        add_op(&mut rt, hole_record(HOLE_A, params.clone()));
        published(&clean_regen(&mut rt).await, "session 1");
        let volume = exact_volume(&wm, plate).await;
        assert_volume(
            volume,
            PLATE_VOLUME - 3.0 * cylinder(4.0, TOP),
            "session 1 drills",
        );
        rt.save(&doc, save_meta()).expect("save");
        wm.shutdown().await;
        (plate, volume, params)
    };
    let json: serde_json::Value = serde_json::from_slice(&document_json(&doc)).unwrap();
    let text = json.to_string();
    for p in params.points.as_ref().unwrap() {
        assert!(
            text.contains(&p.id.to_string()),
            "point id {} persisted",
            p.id
        );
    }

    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &doc);
    let snap = clean_regen(&mut rt).await;
    assert_eq!(
        published(&snap, "reopen").repair_summary.needs_repair_count,
        0
    );
    assert_eq!(rt.head_body_ids(), vec![plate]);
    assert_eq!(
        exact_volume(&wm, plate).await,
        volume,
        "identical volume after reopen"
    );
    assert_eq!(
        stored_hole(&rt, HOLE_A),
        params,
        "ids and placements reload verbatim"
    );
    let doc2 = tmp.path().join("holes-again.onecad");
    rt.save(&doc2, save_meta()).expect("second save");
    assert_eq!(
        document_json(&doc),
        document_json(&doc2),
        "document.json byte-stable"
    );
    wm.shutdown().await;
}
