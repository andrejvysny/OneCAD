//! Private diagnostic for saved joint binding evidence.
use super::*;
use crate::assembly_controls::{Action, OriginPick, Request as AssemblyRequest};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId},
};
use onecad_core::{
    document::component::{JointSide, JointSurface, OccurrencePath},
    edit::AssemblyEdit,
};

const DEFINITION: ComponentDefId = ComponentDefId(uuid::Uuid::from_u128(0xE501));
const A: OccurrenceId = OccurrenceId(uuid::Uuid::from_u128(0xE502));
const B: OccurrenceId = OccurrenceId(uuid::Uuid::from_u128(0xE503));
const JOINT: JointId = JointId(uuid::Uuid::from_u128(0xE504));

async fn command(
    backend: &mut Backend,
    session: &str,
    action: Action,
) -> Result<crate::assembly_controls::Reply, String> {
    let generation = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap()
        .persisted_generation;
    crate::assembly_controls::run(
        backend,
        AssemblyRequest {
            sequence: 1,
            session: session.into(),
            generation,
            cancellation: matches!(&action, Action::Solve { .. } | Action::SolveBound { .. })
                .then(|| Arc::new(std::sync::atomic::AtomicBool::new(false))),
            action,
        },
    )
    .await
}
async fn accept(backend: &mut Backend, session: &str, action: Action) {
    let reply = command(backend, session, action).await.unwrap();
    assert!(
        reply.capture_error.is_none(),
        "accepted edit did not refresh scene: {:?}",
        reply.capture_error
    );
}
fn occurrence(id: OccurrenceId, x: f64, grounded: bool) -> Occurrence {
    Occurrence {
        id,
        def: DEFINITION,
        parent: None,
        local: RigidTransform::new(Quat::IDENTITY, Vec3d::new(x, 0.0, 0.0)),
        grounded,
        name: format!("Instance {id}"),
    }
}

async fn pick(
    backend: &mut Backend,
    session: &str,
    instance: OccurrenceId,
    cylinder: bool,
) -> (OriginPick, Frame) {
    let display = crate::capture::load_display(backend).await.unwrap();
    backend.shared.publish_display(display);
    let display = backend
        .shared
        .display
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    let target = display
        .scene
        .bodies()
        .iter()
        .find(|body| {
            body.occurrence_path()
                .and_then(|p| p.0.last())
                .is_some_and(|id| id.0 == instance.0.as_u128())
        })
        .unwrap();
    let bounds = target.bounds();
    let camera = onecad_scene::Camera {
        target: (bounds.min + bounds.max) * 0.5,
        offset: onecad_scene::DVec3::new(75.0, -75.0, 100.0),
        ..Default::default()
    };
    let viewport = onecad_scene::Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    let view =
        onecad_scene::ViewSnapshot::new(display.scene.identity(), 1, camera, viewport).unwrap();
    let generation = display
        .assembly
        .definitions
        .iter()
        .find(|d| d.id == DEFINITION.to_string())
        .unwrap()
        .mesh_generation
        .unwrap();
    let path = OccurrencePath(vec![instance]);
    let mut seen = std::collections::BTreeSet::new();
    for x in (20..780).step_by(3) {
        for y in (20..580).step_by(3) {
            let hit = onecad_scene::pick_with_policy(
                &display.scene,
                &view,
                view.identity(),
                onecad_scene::DVec2::new(x as f64, y as f64),
                onecad_scene::PickPolicy::FacesAndEdges,
            )
            .unwrap();
            let Some(hit) = hit.filter(|hit| {
                hit.body_id == target.body_id()
                    && hit.kind == onecad_scene::PickKind::Face
                    && hit.ids_have_element_ids
            }) else {
                continue;
            };
            if !seen.insert(hit.topology_id.to_owned()) {
                continue;
            }
            let resolved = api::component_joint_origin(
                &backend.state,
                api::JointOriginRequest {
                    session,
                    generation: display.assembly.persisted_generation,
                    path: &path,
                    body: hit.source_body_id,
                    mesh_generation: generation,
                    element: hit.topology_id,
                    x: Vec3d::X,
                },
            )
            .await;
            let Ok(resolved) = resolved else { continue };
            if cylinder
                && (resolved.surface != JointSurface::Cylinder
                    || (resolved.radius.unwrap() - 3.0).abs() > 1e-6)
            {
                continue;
            }
            if cylinder {
                let reference = serde_json::to_value(&resolved.reference).unwrap();
                if reference["intent"]["descriptor"]["generatorSlot"]["recordId"]
                    != "0f4a315f-2d43-4a14-adf9-a5e71294a3d9"
                {
                    continue;
                }
            }
            let selection = crate::viewport_selection::OccurrenceSelection::from_hit(&hit).unwrap();
            return (
                OriginPick {
                    side: JointSide::A,
                    selection,
                    mesh_generation: generation,
                },
                resolved.frame,
            );
        }
    }
    panic!("fresh occurrence face not pickable: {instance}, cylinder={cylinder}");
}

fn solve(joint: Joint, picks: Vec<OriginPick>) -> Action {
    Action::SolveBound {
        joint: JOINT,
        joints: vec![joint],
        drives: vec![],
        picks,
        unbound: vec![],
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn joint_evidence_mirror_probe() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let path = backend
        .paths
        .root
        .join("documents/joint-evidence-mirror.onecad");
    fs::copy(source, &path).unwrap();
    intent(&mut backend, Request::Open(Some(path.clone())))
        .await
        .unwrap();
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    accept(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::CopyRoot {
            id: DEFINITION,
            name: "Shared plate".into(),
        }),
    )
    .await;
    for instance in [occurrence(A, 200.0, true), occurrence(B, 300.0, false)] {
        accept(
            &mut backend,
            &session,
            Action::Edit(AssemblyEdit::InsertOccurrence {
                occurrence: instance,
            }),
        )
        .await;
    }
    let (picked, frame) = pick(&mut backend, &session, A, true).await;
    let mut other = frame;
    other.origin = other.origin - Vec3d::new(100.0, 0.0, 0.0);
    let joint = Joint {
        id: JOINT,
        name: "Shared source face".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: A,
            frame,
        },
        b: JointOrigin {
            occurrence: B,
            frame: other,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    accept(&mut backend, &session, solve(joint, vec![picked.clone()])).await;
    let saved = api::save_document(
        &backend.state,
        backend.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    let reader = onecad_core::io::container::ContainerReader::open(&path).unwrap();
    let document = reader.document();
    let def = document.definitions.get(&DEFINITION).unwrap();
    let binding = document
        .assembly
        .joint_bindings
        .iter()
        .find(|binding| binding.joint == JOINT && binding.side == JointSide::A)
        .unwrap();
    let reference = &binding.element;
    let primary = reference
        .primary
        .as_ref()
        .expect("binding has no primary identity");
    let entry = def
        .elements
        .get(&primary.element)
        .expect("target definition has no matching entry");
    let intent = reference.intent.as_ref();
    let fields = serde_json::json!({
        "definition": binding.definition == DEFINITION,
        "body": binding.body == primary.body && entry.body == primary.body,
        "kind": entry.kind == primary.kind,
        "primary_id_in_target_index": def.elements.contains(&primary.element),
        "anchor": entry.anchor == reference.anchor,
        "descriptor": intent.is_some_and(|intent| entry.descriptor.as_ref() == Some(&intent.descriptor)),
        "intent_kind": intent.is_some_and(|intent| intent.kind == primary.kind),
        "intent_version_default": intent.is_some_and(|intent| intent.version == onecad_core::regen::engine::PolicyVersions::default().descriptor),
        "import_identity_equal": entry.import_source_identity == reference.import_source_identity,
        "import_identity_absent": entry.import_source_identity.is_none() && reference.import_source_identity.is_none(),
        "reference_extra_empty": reference.extra.is_empty(),
        "primary_extra_empty": primary.extra.is_empty(),
        "intent_extra_empty": intent.is_some_and(|intent| intent.extra.is_empty()),
    });
    let keys = |value: Option<&serde_json::Value>| -> Vec<String> {
        value
            .and_then(serde_json::Value::as_object)
            .map_or_else(Vec::new, |map| map.keys().cloned().collect())
    };
    let output = serde_json::json!({
        "scope": "saved target definition local namespace; before source deletion",
        "definition_id": DEFINITION.to_string(), "binding": binding, "entry": entry,
        "element_id": primary.element, "comparison": fields,
        "entry_descriptor_keys": keys(entry.descriptor.as_ref()),
        "binding_descriptor_keys": keys(intent.map(|item| &item.descriptor)),
        "root_element_count": document.elements.len(), "target_element_count": def.elements.len(),
        "root_has_same_element_id": document.elements.contains(&primary.element),
        "root_extra": document.extra, "root_definition_extra": document.root_definition_extra,
        "target_extra": def.extra, "module_ids": document.modules.keys().map(ToString::to_string).collect::<Vec<_>>(),
        "root_operations": document.timeline.records().iter().map(|r| r.op.op_type()).collect::<Vec<_>>(),
        "target_operations": def.timeline.records().iter().map(|r| r.op.op_type()).collect::<Vec<_>>(),
    });
    let evidence = Path::new("/private/tmp/onecad-resume-evidence");
    write_exclusive(
        &evidence.join("joint-evidence-mirror-probe.json"),
        &serde_json::to_vec_pretty(&output).unwrap(),
    );
    write_exclusive(
        &evidence.join("joint-evidence-mirror-document.json"),
        &serde_json::to_vec_pretty(document).unwrap(),
    );
    write_exclusive(
        &evidence.join("joint-evidence-mirror-saved.onecad"),
        &fs::read(&path).unwrap(),
    );
    eprintln!("binding and target entry comparison: {fields}");
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

fn write_exclusive(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
