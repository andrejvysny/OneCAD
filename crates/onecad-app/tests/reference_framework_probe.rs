//! Incremental editable Framework enclosure stock and floor-profile experiment.
//! This is not the finished enclosure or a public-reference qualification.
#[allow(dead_code)]
mod native_plate_support;
mod reference_framework_support;

use native_plate_support::Harness;
use onecad_app::api;
use onecad_core::{
    document::{
        datum::DatumPlane,
        record::{
            BooleanMode, BooleanOp, BooleanParams, KnownOperation, LoftParams, Operation,
            OperationRecord, RevolveParams,
        },
        refs::{AxisRef, SketchRegionRef},
        variables::Scalar,
    },
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, DatumPlaneId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use reference_framework_support::{
    add_sketch, cut_connector_end, extrusion, inboard_clip_cut, loft_clip_cut,
    loft_clip_section_cut, mesh_signature, rear_clip_cut, rear_gap_cut, rear_upper_section,
    ruled_ramp,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn sketch(n: u128) -> SketchId {
    SketchId(Uuid::from_u128(0xfac0000 + n))
}
fn record(n: u128) -> RecordId {
    RecordId(Uuid::from_u128(0xfac0100 + n))
}

#[tokio::test(flavor = "multi_thread")]
async fn framework_enclosure_editable_stock_and_floor_profile() {
    native_plate_support::required_worker();
    reference_framework_support::verify_source();
    let root = tempfile::tempdir().unwrap();
    let h = Harness::new(root.path()).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let recipe: Value = serde_json::from_str(include_str!(
        "../../../fixtures/reference-histories/framework-enclosure-profiles.json"
    ))
    .unwrap();
    let (body, upper_mass) = reference_framework_support::initial_body(&h, &recipe).await;
    if reference_framework_support::replay_checkpoint(&h, body, root.path(), "initial").await {
        return;
    }
    let rear = add_sketch(&h, 5, &recipe["rearPocketFloorFace"]["wires"], Some(6.8)).await;
    assert_eq!(rear.regions.len(), 1);
    extrusion(&h, 5, &rear, -5.6, Some(body)).await;
    let rear_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(rear_mass.volume < upper_mass.volume);
    eprintln!(
        "Framework intermediate rear pocket volume {} mm³, area {} mm²",
        rear_mass.volume, rear_mass.surface_area
    );
    let source_loop = recipe["interiorFloorFace"]["wires"][0].as_array().unwrap();
    assert_eq!(source_loop.len(), 32);
    let mut widening = source_loop[8..=12].to_vec();
    widening.push(json!({"kind":"line", "start":[-9.0,-30.5], "end":[9.0,-30.5]}));
    let widening = add_sketch(&h, 6, &json!([widening]), Some(6.8)).await;
    assert_eq!(widening.regions.len(), 1);
    extrusion(&h, 6, &widening, -5.3, Some(body)).await;
    let widened_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(widened_mass.volume < rear_mass.volume);
    eprintln!(
        "Framework intermediate widened rear pocket volume {} mm³, area {} mm²",
        widened_mass.volume, widened_mass.surface_area
    );
    let connector = reference_framework_support::connector_profile(&h).await;
    assert_eq!(connector.regions.len(), 1);
    extrusion(&h, 7, &connector, -1.5, Some(body)).await;
    let connector_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(connector_mass.volume < widened_mass.volume);
    eprintln!(
        "Framework intermediate connector core volume {} mm³, area {} mm²",
        connector_mass.volume, connector_mass.surface_area
    );
    reference_framework_support::fillet_connector_lip(&h, body).await;
    let lip_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(lip_mass.volume < connector_mass.volume);
    eprintln!(
        "Framework intermediate connector lip volume {} mm³, area {} mm²",
        lip_mass.volume, lip_mass.surface_area
    );
    for (index, offset) in [-11.7, 10.9].into_iter().enumerate() {
        let n = 30 + index as u128;
        let slot = reference_framework_support::slot_profile(
            &h,
            n,
            offset,
            WorldPlane::XZ,
            &[[13.5, 3.1], [15.1, 1.5], [15.1, 6.8], [13.5, 6.8]],
        )
        .await;
        assert_eq!(slot.regions.len(), 1);
        extrusion(&h, n, &slot, 0.8, Some(body)).await;
    }
    let slot_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(
        (lip_mass.volume - slot_mass.volume - 11.52).abs() < 1e-7,
        "slot removed volume {}",
        lip_mass.volume - slot_mass.volume
    );
    eprintln!(
        "Framework intermediate side slots volume {} mm³, area {} mm²",
        slot_mass.volume, slot_mass.surface_area
    );
    let edge = 13.52618259739_f64;
    for (index, side) in [-1.0_f64, 1.0].into_iter().enumerate() {
        let x = |v: f64| side * v;
        let vertices = [
            [x(edge), -26.5],
            [x(edge - 1.0), -27.5],
            [x(edge - 1.0), -30.5],
            [x(edge), -30.5],
        ];
        let wire: Vec<_> = (0..4)
            .map(|i| json!({"kind":"line", "start": vertices[i], "end": vertices[(i+1)%4]}))
            .collect();
        let n = 40 + index as u128;
        let profile = add_sketch(&h, n, &json!([wire]), Some(6.8)).await;
        assert_eq!(profile.regions.len(), 1);
        extrusion(&h, n, &profile, -1.5, Some(body)).await;
    }
    let rear_slot_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!((slot_mass.volume - rear_slot_mass.volume - 10.5).abs() < 1e-7);
    eprintln!(
        "Framework intermediate rear slots volume {} mm³, area {} mm²",
        rear_slot_mass.volume, rear_slot_mass.surface_area
    );
    for (index, offset) in [-edge, edge - 1.0].into_iter().enumerate() {
        let n = 50 + index as u128;
        let wedge = reference_framework_support::slot_profile(
            &h,
            n,
            offset,
            WorldPlane::XZ,
            &[[26.5, 4.0], [27.8, 5.3], [26.5, 5.3]],
        )
        .await;
        assert_eq!(wedge.regions.len(), 1);
        extrusion(&h, n, &wedge, 1.0, Some(body)).await;
    }
    let wedge_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(wedge_mass.volume < rear_slot_mass.volume);
    eprintln!(
        "Framework intermediate rear slot wedges volume {} mm³, area {} mm²",
        wedge_mass.volume, wedge_mass.surface_area
    );
    let draft_angle =
        std::env::var("ONECAD_FRAMEWORK_UNDERSIDE_DRAFT").unwrap_or_else(|_| "-1".into());
    let drafted_underside = draft_angle != "0";
    let underside = add_sketch(
        &h,
        8,
        &recipe["rearUndersideFace"]["wires"],
        drafted_underside.then_some(0.65),
    )
    .await;
    assert_eq!(underside.regions.len(), 1);
    extrusion(
        &h,
        8,
        &underside,
        if drafted_underside { -0.65 } else { 0.65 },
        Some(body),
    )
    .await;
    if draft_angle == "-1" {
        reference_framework_support::fillet_drafted_underside(&h, body).await;
    }
    let underside_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(underside_mass.volume < wedge_mass.volume);
    eprintln!(
        "Framework intermediate underside volume {} mm³, area {} mm²",
        underside_mass.volume, underside_mass.surface_area
    );
    if let Some(path) = std::env::var_os("ONECAD_FRAMEWORK_STOP_AFTER_UNDERSIDE") {
        api::export_step_file(
            &h.state,
            h.host.clone(),
            Some(path.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
        h.close().await;
        return;
    }
    for (index, side) in [-1.0_f64, 1.0].into_iter().enumerate() {
        let n = 60 + index as u128;
        let points = [
            [-side * 15.0, 3.09],
            [-side * 14.19, 3.9422608695652075],
            [-side * 14.19, 4.257549698780312],
            [-side * 15.0, 4.3],
        ];
        let groove_start = 2.203927026704;
        let profile =
            reference_framework_support::slot_profile(&h, n, groove_start, WorldPlane::YZ, &points)
                .await;
        assert_eq!(profile.regions.len(), 1);
        extrusion(&h, n, &profile, 26.0 - groove_start, Some(body)).await;
    }
    let groove_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(groove_mass.volume < underside_mass.volume);
    eprintln!(
        "Framework intermediate straight groove segments volume {} mm³, area {} mm²",
        groove_mass.volume, groove_mass.surface_area
    );
    if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_RAMP").is_some() {
        reference_framework_support::chamfer_rear_ramp(&h, body).await;
        let ramp_mass = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap();
        assert!(ramp_mass.volume < groove_mass.volume);
        eprintln!(
            "Framework intermediate rear ramp volume {} mm³, area {} mm²",
            ramp_mass.volume, ramp_mass.surface_area
        );
    }
    ruled_ramp(&h, &recipe, body).await;
    let loft_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(loft_mass.volume < groove_mass.volume);
    eprintln!(
        "Framework ruled ramp volume {} mm³, area {} mm²",
        loft_mass.volume, loft_mass.surface_area
    );
    let pre_flare_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    let flare = reference_framework_support::connector_flare_profile(&h).await;
    assert_eq!(flare.regions.len(), 1);
    extrusion(&h, 100, &flare, 5.85, Some(body)).await;
    let flare_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(flare_mass.volume < pre_flare_mass.volume);
    eprintln!(
        "Framework connector straight flare volume {} mm³, area {} mm²",
        flare_mass.volume, flare_mass.surface_area
    );
    let experimental_ends = std::env::var_os("ONECAD_FRAMEWORK_SKIP_ENDS").is_none();
    let (mut undo_volume, mut final_volume) = if experimental_ends {
        cut_connector_end(&h, body, 101, -2.925).await;
        let first = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap();
        cut_connector_end(&h, body, 102, 2.925).await;
        let end = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap();
        assert!(end.volume < first.volume && first.volume < flare_mass.volume);
        (first.volume, end.volume)
    } else {
        (pre_flare_mass.volume, flare_mass.volume)
    };
    if reference_framework_support::replay_checkpoint(&h, body, root.path(), "preclip").await {
        return;
    }
    let experimental_clips = std::env::var_os("ONECAD_FRAMEWORK_SKIP_CLIPS").is_none();
    let plane_clips = std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_CLIP_PLANES").is_some();
    if experimental_clips && plane_clips {
        for side in 0..2 {
            for (index, upper) in [false, true].into_iter().enumerate() {
                undo_volume = api::query_mass_properties(
                    &h.state,
                    onecad_app::worker::wire::body_id_wire(body),
                    None,
                )
                .await
                .unwrap()
                .volume;
                loft_clip_cut(
                    &h,
                    body,
                    &recipe,
                    side,
                    upper,
                    130 + 20 * side as u128 + 8 * index as u128,
                )
                .await;
                let stage = format!("clip-{side}-{index}");
                if reference_framework_support::replay_checkpoint(&h, body, root.path(), &stage)
                    .await
                {
                    return;
                }
            }
        }
        if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_CLIP_PLANES").is_some() {
            for side in 0..2 {
                undo_volume = api::query_mass_properties(
                    &h.state,
                    onecad_app::worker::wire::body_id_wire(body),
                    None,
                )
                .await
                .unwrap()
                .volume;
                let n = 180 + 12 * side as u128;
                if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_TRIANGLE").is_some() {
                    rear_clip_cut(&h, body, &recipe, side, n).await;
                } else {
                    let sign = if side == 0 { -1.0 } else { 1.0 };
                    let sections = [13.2 * sign, 16.0 * sign]
                        .map(|x| (x, rear_upper_section(&recipe, side, x)));
                    loft_clip_section_cut(&h, body, sections, n).await;
                }
            }
            if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_INBOARD").is_some() {
                for side in 0..2 {
                    undo_volume = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap()
                    .volume;
                    inboard_clip_cut(&h, body, &recipe, side, 260 + 12 * side as u128).await;
                }
            }
            if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_WALL121").is_some() {
                assert!(std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_INBOARD").is_some());
                for side in 0..2 {
                    undo_volume = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap()
                    .volume;
                    reference_framework_support::cut_straight_wall(
                        &h,
                        body,
                        side,
                        300 + side as u128,
                    )
                    .await;
                    let mass = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap();
                    assert!(mass.volume < undo_volume, "wall cut must remove material");
                    eprintln!(
                        "Framework wall side={side} removed {} mm³",
                        undo_volume - mass.volume
                    );
                }
            }
            if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_AFTER_WALL").is_some() {
                assert!(std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_WALL121").is_some());
                assert!(std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_TRIANGLE").is_none());
                assert!(std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_GAP").is_none());
                for side in 0..2 {
                    undo_volume = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap()
                    .volume;
                    rear_clip_cut(&h, body, &recipe, side, 320 + 12 * side as u128).await;
                    let mass = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap();
                    assert!(mass.volume < undo_volume, "rear cut must remove material");
                    eprintln!(
                        "Framework rear after wall side={side} removed {} mm³",
                        undo_volume - mass.volume
                    );
                }
            }
            if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_GAP").is_some() {
                assert!(std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_TRIANGLE").is_some());
                for side in 0..2 {
                    undo_volume = api::query_mass_properties(
                        &h.state,
                        onecad_app::worker::wire::body_id_wire(body),
                        None,
                    )
                    .await
                    .unwrap()
                    .volume;
                    rear_gap_cut(&h, body, &recipe, side, 220 + 15 * side as u128).await;
                }
            }
        }
        final_volume = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap()
        .volume;
    } else if experimental_clips {
        for (index, x) in [-13.2_f64, 13.2].into_iter().enumerate() {
            let n = 110 + index as u128;
            let clip = reference_framework_support::clip_profile(
                &h,
                n,
                x,
                &recipe["clipFloorFaces"][index]["wires"],
            )
            .await;
            assert_eq!(
                clip.regions.len(),
                1,
                "raw source clip must form one region"
            );
            undo_volume = api::query_mass_properties(
                &h.state,
                onecad_app::worker::wire::body_id_wire(body),
                None,
            )
            .await
            .unwrap()
            .volume;
            extrusion(&h, n, &clip, 1.8 * x.signum(), Some(body)).await;
        }
        final_volume = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap()
        .volume;
    }
    if std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_CLIP_BLEND").is_some() {
        assert!(experimental_clips);
        if let Some(path) = std::env::var_os("ONECAD_FRAMEWORK_CLIP_PREBLEND_OUT") {
            api::export_step_file(
                &h.state,
                h.host.clone(),
                Some(path.to_string_lossy().into_owned()),
            )
            .await
            .unwrap();
        }
        let blend_recipe = if plane_clips {
            reference_framework_support::clip_unfillet_recipe(&recipe)
        } else {
            recipe.clone()
        };
        for side in 0..2 {
            undo_volume = final_volume;
            reference_framework_support::fillet_clip_floors(&h, body, &blend_recipe, side).await;
            final_volume = api::query_mass_properties(
                &h.state,
                onecad_app::worker::wire::body_id_wire(body),
                None,
            )
            .await
            .unwrap()
            .volume;
            eprintln!(
                "Framework clip blend {side} volume delta {} mm³",
                final_volume - undo_volume
            );
        }
        if let Some(path) = std::env::var_os("ONECAD_FRAMEWORK_STOP_AFTER_CLIP_BLEND") {
            api::export_step_file(
                &h.state,
                h.host.clone(),
                Some(path.to_string_lossy().into_owned()),
            )
            .await
            .unwrap();
            h.close().await;
            return;
        }
    }
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let kinds: Vec<_> = projection
        .features
        .iter()
        .map(|f| f.op_type.as_str())
        .collect();
    assert!(kinds.iter().all(|kind| [
        "Sketch", "Extrude", "Hole", "Fillet", "Loft", "Boolean", "Revolve"
    ]
    .contains(kind)));
    assert!(["Extrude", "Hole", "Fillet", "Loft", "Boolean"]
        .iter()
        .all(|kind| kinds.contains(kind)));
    assert!(h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_blob_shas()
        .is_empty());
    if let Ok(path) = std::env::var("ONECAD_FRAMEWORK_DIAGNOSTIC_OUT") {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        api::export_step_file(&h.state, h.host.clone(), Some(path))
            .await
            .unwrap();
    }
    let pre_undo_signature = mesh_signature(&h, body).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert_eq!(
        h.projection().await.bodies.len(),
        if (experimental_ends && !experimental_clips)
            || (plane_clips
                && std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_CLIP_BLEND").is_none()
                && (std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_WALL121").is_none()
                    || std::env::var_os("ONECAD_FRAMEWORK_EXPERIMENT_REAR_AFTER_WALL").is_some()))
        {
            2
        } else {
            1
        }
    );
    assert!(h.projection().await.bodies.contains_key(&body.to_string()));
    let before_flare =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert_eq!(before_flare.volume, undo_volume);
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    let redo_projection = h.projection().await;
    assert_eq!(redo_projection.bodies.len(), 1);
    assert!(redo_projection.bodies.contains_key(&body.to_string()));
    assert_eq!(mesh_signature(&h, body).await, pre_undo_signature);
    let replay =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert_eq!(replay.volume, final_volume);
    let topology = api::query_body_topology(&h.state, onecad_app::worker::wire::body_id_wire(body))
        .await
        .unwrap();
    let signature = mesh_signature(&h, body).await;
    let audit = reference_framework_support::authored_float_snapshot(&h).await;
    let path = if let Ok(path) = std::env::var("ONECAD_FRAMEWORK_DOCUMENT_OUT") {
        let path = std::path::PathBuf::from(path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        path
    } else {
        root.path().join("partial-framework.onecad")
    };
    let saved = api::save_document(
        &h.state,
        h.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    reference_framework_support::audit_saved_floats(audit, &path);
    h.close().await;
    let reopened = Harness::new(&root.path().join("reopen")).await;
    api::open_document(
        &reopened.state,
        reopened.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    reopened.settled(&reopened.projection().await).await;
    if let Ok(path) = std::env::var("ONECAD_FRAMEWORK_REOPEN_DIAGNOSTIC_OUT") {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        api::export_step_file(&reopened.state, reopened.host.clone(), Some(path))
            .await
            .unwrap();
    }
    let mass = api::query_mass_properties(
        &reopened.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap();
    let reopened_topology = api::query_body_topology(
        &reopened.state,
        onecad_app::worker::wire::body_id_wire(body),
    )
    .await
    .unwrap();
    eprintln!(
        "Framework replay volume before={final_volume:.17} after={:.17}, ULP difference={}",
        mass.volume,
        mass.volume.to_bits().abs_diff(final_volume.to_bits())
    );
    assert_eq!(topology, reopened_topology);
    assert_eq!(signature, mesh_signature(&reopened, body).await);
    assert!(
        mass.volume.to_bits().abs_diff(final_volume.to_bits()) <= 8,
        "replay mass exceeds eight ULP"
    );
    reopened.close().await;
}
