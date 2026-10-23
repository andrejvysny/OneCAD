//! Real operations expose consumed operands and curved-face handle geometry.
use super::*;
use crate::document_flow::Request;
use onecad_core::{
    document::{Document, record::OffsetDistanceType},
    ids::DocumentId,
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
};
use onecad_interaction::{ModelingOptions, ProfileSelection, sketch::SketchAction};
use std::path::{Path, PathBuf};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn reopened_shell_resolves_its_consumed_opening_handle() {
    let root = tempfile::tempdir().unwrap();
    let path = blank_document(root.path());
    let mut host = solid(root.path(), &path, false).await;
    let pick = hole_pick(&host.app);
    let onecad_interaction::selection_set::SelectionItem::Face(picked) = &pick else {
        panic!()
    };
    let picked_id = match &picked.id {
        onecad_interaction::selection_set::TopologyId::Element(id) => id.as_str(),
        onecad_interaction::selection_set::TopologyId::Snapshot(id) => id.as_str(),
    };
    let original = face_basis(&host, picked_id).unwrap();
    crate::item_selection::apply(
        &mut host.app,
        onecad_interaction::selection_set::SelectionMode::Replace,
        vec![pick],
    );
    crate::operation_start::begin_shell(&mut host.app);
    ready(&mut host).await;
    let record = host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .record
        .clone();
    let Operation::Known(KnownOperation::Shell(shell)) = &record.op else {
        panic!("real Shell staging must author Shell")
    };
    let opening = shell.faces[0].primary.as_ref().unwrap().element.clone();
    confirm(&mut host).await;
    eprintln!(
        "Shell opening {opening}: original={original:?}, after Shell={:?}",
        face_basis(&host, opening.as_str())
    );
    move_downstream(&mut host).await;
    eprintln!(
        "Shell opening {opening}: after downstream Move={:?}",
        face_basis(&host, opening.as_str())
    );
    save(&mut host).await;
    host.stop().await;
    let mut host = opened(root.path(), &path).await;
    edit(&mut host, record.record_id).await;
    let (base, direction, scale) = model_handle::testing_axis(&host.app, N::ShellThickness)
        .expect("reopened Shell must resolve its authoritative prefix opening");
    assert!(
        base.distance(original.0) < 1e-6,
        "Shell handle must stay at pre-Shell opening {:?}; final-head target is {base:?}",
        original.0
    );
    assert!(direction.dot(-original.1) > 0.999999);
    assert_eq!(scale, 1.0);
    host.app.dispatch(Action::Cancel);
    assert!(!host.app.document_dirty());
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn cylinder_diameter_handle_uses_the_worker_sample_and_radial_scale() {
    let root = tempfile::tempdir().unwrap();
    let path = blank_document(root.path());
    let mut host = solid(root.path(), &path, true).await;
    let pick = cylinder_wall(&host);
    crate::item_selection::apply(
        &mut host.app,
        onecad_interaction::selection_set::SelectionMode::Replace,
        vec![pick],
    );
    crate::operation_start::begin_offset_face(&mut host.app);
    ready(&mut host).await;
    let tag = host.app.doc.interaction.tool.as_ref().unwrap().tag.clone();
    host.app.dispatch(Action::ModelingOptionsChanged {
        tag,
        options: ModelingOptions::FaceOffsetMode(OffsetDistanceType::Diameter),
    });
    host.app.dispatch(Action::ParameterChanged {
        key: N::OffsetFaceDistance.into(),
        text: "22".into(),
    });
    ready(&mut host).await;
    let record = host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .record
        .record_id;
    assert_diameter_scale(&host);
    confirm(&mut host).await;
    save(&mut host).await;
    host.stop().await;
    let mut host = opened(root.path(), &path).await;
    edit(&mut host, record).await;
    assert_diameter_scale(&host);
    host.app.dispatch(Action::Cancel);
    assert!(!host.app.document_dirty());
    host.stop().await;
}

fn blank_document(root: &Path) -> PathBuf {
    let paths = crate::qa::QaPaths::from_args(
        ["--qa-root".into(), root.join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    let path = paths.documents.join("Handle regression.onecad");
    ContainerWriter::save(
        &path,
        &Document::new(DocumentId::new()),
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "native-handle-regression".into(),
            created: "2026-10-22T00:00:00Z".into(),
            modified: "2026-10-22T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .unwrap();
    path
}

async fn opened(root: &Path, path: &Path) -> Host {
    let mut host = Host::new(root).await;
    host.app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
    })
    .await;
    host.app
        .document_request(Request::Open(Some(path.to_path_buf())));
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy()
            && app
                .doc
                .document_state
                .as_ref()
                .is_some_and(|state| state.path.as_deref() == Some(path.to_string_lossy().as_ref()))
    })
    .await;
    host.app.ui.start_open = false;
    host.app.ui.inspector_open = false;
    host.app.ui.explorer_open = false;
    host.app.doc.camera = Camera {
        offset: DVec3::new(100.0, -180.0, 130.0),
        projection: Projection::Perspective,
        ..Default::default()
    };
    host.redraw();
    host
}

async fn solid(root: &Path, path: &Path, cylinder: bool) -> Host {
    let mut host = opened(root, path).await;
    host.app.new_sketch();
    pump_until(&mut host.app, &host.context, &mut host.frame, active).await;
    host.app.doc.camera = Camera {
        offset: DVec3::Z * 120.0,
        projection: Projection::Orthographic,
        ..Default::default()
    };
    host.control(if cylinder { "Circle" } else { "Rectangle" });
    host.xy(0.0, 0.0);
    host.xy(
        if cylinder { 10.0 } else { 40.0 },
        if cylinder { 0.0 } else { 30.0 },
    );
    host.count(if cylinder { 2 } else { 8 }).await;
    host.app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| !d.profiles.is_empty())
    })
    .await;
    extrude_profile(&mut host).await;
    host
}

async fn extrude_profile(host: &mut Host) {
    let display = host.app.doc.display.clone().unwrap();
    host.app.doc.camera = Camera {
        target: display.profiles[0].centroid(),
        offset: DVec3::new(100.0, -180.0, 130.0),
        projection: Projection::Perspective,
        ..Default::default()
    };
    host.redraw();
    host.app
        .dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile: display.profiles[0].reference.clone(),
            context: crate::sketch_services::document_context(&display),
            capture: host.app.doc.view.unwrap().identity(),
        })));
    host.app.dispatch(Action::BeginExtrude {
        record_id: RecordId::new(),
        gesture: 1,
        step_index: display.projection.applied_ops as u32,
    });
    host.app.dispatch(Action::DraftChanged("12".into()));
    ready(host).await;
    confirm(host).await;
    host.redraw();
}

async fn save(host: &mut Host) {
    host.app.document_request(Request::Save { save_as: false });
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.documents.running && !app.document_dirty()
    })
    .await;
    assert!(
        host.app.doc.status.starts_with("Saved "),
        "{}",
        host.app.doc.status
    );
}

async fn edit(host: &mut Host, record: RecordId) {
    crate::pill::select_feature(&mut host.app, record);
    crate::inspector_history::edit_selected_feature(&mut host.app);
    ready(host).await;
    assert!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .editing_existing
    );
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        let parameter = app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .parameter()
            .unwrap();
        model_handle::testing_axis(app, parameter).is_some()
    })
    .await;
}

async fn move_downstream(host: &mut Host) {
    let body = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].body_id();
    crate::item_selection::apply(
        &mut host.app,
        onecad_interaction::selection_set::SelectionMode::Replace,
        vec![onecad_interaction::selection_set::SelectionItem::Body(body)],
    );
    host.app.begin_transform();
    host.app.dispatch(Action::ParameterChanged {
        key: N::TransformTranslation(onecad_interaction::ParameterAxis::Z).into(),
        text: "20".into(),
    });
    ready(host).await;
    confirm(host).await;
    host.redraw();
}

fn face_basis(host: &Host, id: &str) -> Option<(DVec3, DVec3)> {
    let mesh = host.app.doc.display.as_ref()?.scene.bodies()[0]
        .mesh()
        .ok()?;
    let face = (0..mesh.header().face_count as usize).find(|&i| mesh.face_id(i) == Some(id))?;
    let range = mesh.face_range(face)?;
    let (mut point, mut normal, mut area) = (DVec3::ZERO, DVec3::ZERO, 0.0);
    for index in range.first..range.first + range.count {
        let [a, b, c] = mesh
            .triangle(index as usize)?
            .map(|i| DVec3::from_array(mesh.position(i as usize).unwrap().map(f64::from)));
        let cross = (b - a).cross(c - a);
        point += (a + b + c) / 3.0 * cross.length();
        normal += cross;
        area += cross.length();
    }
    Some((point / area, normal.try_normalize()?))
}

fn cylinder_wall(host: &Host) -> onecad_interaction::selection_set::SelectionItem {
    let display = host.app.doc.display.as_ref().unwrap();
    let body = &display.scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let face = (0..mesh.header().face_count as usize)
        .find(|&face| {
            let range = mesh.face_range(face).unwrap();
            let (mut low, mut high, mut sum, mut area) =
                (f64::INFINITY, f64::NEG_INFINITY, DVec3::ZERO, 0.0);
            for index in range.first..range.first + range.count {
                let [a, b, c] = mesh
                    .triangle(index as usize)
                    .unwrap()
                    .map(|i| DVec3::from_array(mesh.position(i as usize).unwrap().map(f64::from)));
                low = low.min(a.z).min(b.z).min(c.z);
                high = high.max(a.z).max(b.z).max(c.z);
                let normal = (b - a).cross(c - a);
                sum += normal;
                area += normal.length();
            }
            if high - low < 11.9 {
                return false;
            }
            eprintln!(
                "real-worker cylinder: triangles={}, normal_sum={sum:?}, length={}, area={area}",
                range.count,
                sum.length()
            );
            assert!(
                sum.length() / area < 1e-9,
                "full lateral normals cancel; no aggregate direction exists"
            );
            true
        })
        .expect("extruded circle has a cylindrical wall");
    onecad_interaction::selection_set::SelectionItem::Face(
        onecad_interaction::selection_set::TopologyRef {
            scene: display.scene.identity(),
            body: body.body_id(),
            id: crate::model_pick::topology_id(mesh.face_id(face).unwrap()),
        },
    )
}

fn assert_diameter_scale(host: &Host) {
    let tool = host.app.doc.interaction.tool.as_ref().unwrap();
    let Operation::Known(KnownOperation::OffsetFace(params)) = tool.effective_op() else {
        panic!()
    };
    let point = params.faces[0]
        .anchor
        .as_ref()
        .expect("worker supplied the sampled surface anchor")
        .world_point;
    let point = DVec3::new(point.x, point.y, point.z);
    let center = DVec3::new(0.0, 0.0, point.z);
    let radial = (point - center).normalize();
    let view = host.app.doc.view.unwrap();
    let rect = canvas(host);
    let at = |value| {
        let tip = model_handle::testing_tip(&host.app, rect, Some(N::OffsetFaceDistance), value)
            .expect("cylindrical handle must resolve a stable local direction");
        crate::gizmo::along(&view, rect, tip, center, radial).unwrap()
    };
    let displacement = at(24.0) - at(22.0);
    assert!(
        (displacement - 1.0).abs() < 1e-3,
        "diameter +2 mm must move radially +1 mm along the worker sample; got {displacement}"
    );
}
