//! Permanent source deletion must preserve occurrence ownership during repair.
use super::*;
use crate::assembly_controls::{Action, OriginPick, Request as AssemblyRequest};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId},
};
use onecad_core::{
    document::component::{JointSide, JointSurface, OccurrencePath},
    edit::{AssemblyEdit, EditCommand},
    ids::RecordId,
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

fn delete_source() -> Action {
    Action::Edit(AssemblyEdit::EditDefinition {
        id: DEFINITION,
        command: Box::new(EditCommand::RemoveOperation {
            record: "0f4a315f-2d43-4a14-adf9-a5e71294a3d9"
                .parse::<RecordId>()
                .unwrap(),
        }),
    })
}
fn poses(projection: &onecad_app::dto::AssemblyProjectionDto) -> Vec<RigidTransform> {
    projection.occurrences.iter().map(|row| row.local).collect()
}
async fn bindings(
    backend: &Backend,
    session: &str,
) -> Vec<onecad_core::document::component::JointBinding> {
    let p = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    api::get_joint_bindings(&backend.state, session, p.persisted_generation)
        .await
        .unwrap()
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
async fn deleted_source_repair_rejects_sibling_and_reopens() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let path = backend
        .paths
        .root
        .join("documents/deleted-source-repair.onecad");
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
    let before = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let original = bindings(&backend, &session).await;
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].path, OccurrencePath(vec![A]));
    assert!(!original[0].needs_repair);
    let (fresh_a, _) = pick(&mut backend, &session, A, true).await;
    let accepted_scene = backend
        .shared
        .display
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(fresh_a.selection.scene, accepted_scene.scene.identity());
    let deleted = command(&mut backend, &session, delete_source())
        .await
        .unwrap();
    assert!(
        deleted.capture_error.is_none(),
        "deleted-source capture failed: {:?}",
        deleted.capture_error
    );
    let damaged = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let marked = bindings(&backend, &session).await;
    let mut expected = original.clone();
    expected[0].needs_repair = true;
    assert_eq!(marked, expected);
    assert_eq!(poses(&damaged), poses(&before));
    let stale = command(
        &mut backend,
        &session,
        solve(damaged.joints[0].clone(), vec![fresh_a]),
    )
    .await
    .unwrap_err();
    assert!(
        stale.contains("Selected face belongs to a stale scene"),
        "{stale}"
    );
    assert_eq!(bindings(&backend, &session).await, marked);
    let restored = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(poses(&restored), poses(&before));
    let (sibling, _) = pick(&mut backend, &session, B, false).await;
    let fresh = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(poses(&fresh), poses(&before));
    let error = command(
        &mut backend,
        &session,
        solve(fresh.joints[0].clone(), vec![sibling]),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("Selected face belongs to another occurrence"),
        "{error}"
    );
    assert_eq!(bindings(&backend, &session).await, marked);
    assert_eq!(
        serde_json::to_value(
            api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&fresh).unwrap()
    );
    let (repick, _) = pick(&mut backend, &session, A, false).await;
    accept(
        &mut backend,
        &session,
        solve(restored.joints[0].clone(), vec![repick]),
    )
    .await;
    let repaired = bindings(&backend, &session).await;
    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].path, OccurrencePath(vec![A]));
    assert!(!repaired[0].needs_repair);
    assert_ne!(repaired[0].element, original[0].element);
    assert_eq!(
        poses(
            &api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
        ),
        poses(&before)
    );
    assert!(
        api::save_document(
            &backend.state,
            backend.host.clone(),
            Some(path.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    intent(&mut backend, Request::Open(Some(path)))
        .await
        .unwrap();
    let reopened = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    crate::services::load_ready_display(&mut backend)
        .await
        .unwrap();
    assert_eq!(bindings(&backend, &reopened).await, repaired);
    assert_eq!(
        poses(
            &api::get_assembly_projection(&backend.state, &reopened)
                .await
                .unwrap()
        ),
        poses(&before)
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}
