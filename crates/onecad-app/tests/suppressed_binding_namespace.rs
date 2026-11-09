//! Definition-local identity collisions must not reactivate suppressed stale origins.
//!
//! Face removal is supplied as accepted derived state through the production
//! component-adoption contract. This tests persistence and public solve admission,
//! not native CAD face deletion or quarantine behavior.
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use onecad_app::{
    document_runtime::DocumentRuntime,
    joint_execution,
    state::AppState,
    worker::{manager::SupervisorConfig, resolve_worker_path, WorkerManager},
};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Vec3d},
};
use onecad_core::{
    document::{
        body::BodyMeta,
        component::{
            ComponentDefId, JointBinding, JointSide, JointSurface, Occurrence, OccurrenceId,
            OccurrencePath, RigidTransform,
        },
        element_index::ElementEntry,
        refs::ElementKind,
        Document,
    },
    edit::{AssemblyEdit, DocumentSession, EditCommand},
    ids::{BodyId, DocumentId, ElementId, RecordId},
    io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
};
use uuid::Uuid;

const ELEMENT: &str = "el_00000000000000000000000000000011";

fn definition(n: u128) -> ComponentDefId {
    ComponentDefId(Uuid::from_u128(n))
}
fn occurrence(n: u128) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(n))
}
fn body(n: u128) -> BodyId {
    BodyId(Uuid::from_u128(n))
}

fn real_worker() -> Option<PathBuf> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    if let Some(path) = &path {
        assert!(
            path.is_file(),
            "resolved worker binary is missing: {}",
            path.display()
        );
    }
    assert!(
        path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    path
}

fn joint(n: u128, a: u128, b: u128, suppressed: bool) -> Joint {
    let frame = Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    Joint {
        id: JointId(Uuid::from_u128(n)),
        name: format!("joint {n}"),
        kind: if suppressed {
            JointKind::Rigid
        } else {
            JointKind::Slider
        },
        a: JointOrigin {
            occurrence: occurrence(a),
            frame,
        },
        b: JointOrigin {
            occurrence: occurrence(b),
            frame,
        },
        limits: None,
        value: 0.0,
        suppressed,
    }
}

fn binding(joint: &Joint) -> JointBinding {
    JointBinding {
        joint: joint.id,
        side: JointSide::A,
        path: OccurrencePath(vec![joint.a.occurrence]),
        definition: definition(1),
        body: body(10),
        element: serde_json::from_value(serde_json::json!({
            "primary":{"bodyId":body(10),"elementId":ELEMENT,"kind":"face"},
            "intent":{"version":1,"kind":"face","descriptor":{}},
            "anchor":{"worldPoint":[0.0,0.0,0.0]}
        }))
        .unwrap(),
        surface: JointSurface::Plane,
        radius: None,
        orientation_x: Vec3d::X,
        needs_repair: true,
    }
}

fn copied_document() -> Document {
    let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(20))));
    session
        .apply(EditCommand::AddBody {
            body: BodyMeta::new(body(10), "source body", RecordId(Uuid::from_u128(10))),
        })
        .unwrap();
    let mut document = session.into_document();
    document.elements.insert(
        ElementId::new(ELEMENT),
        ElementEntry::new(body(10), ElementKind::Face),
    );
    let mut session = DocumentSession::new(document);
    for n in [1, 2] {
        session
            .apply(EditCommand::Assembly {
                edit: Box::new(AssemblyEdit::CopyRoot {
                    id: definition(n),
                    name: format!("copy {n}"),
                }),
            })
            .unwrap();
    }
    for n in 1..=4 {
        session
            .apply(EditCommand::Assembly {
                edit: Box::new(AssemblyEdit::InsertOccurrence {
                    occurrence: Occurrence {
                        id: occurrence(n),
                        def: definition(if n == 1 { 1 } else { 2 }),
                        parent: None,
                        local: RigidTransform::IDENTITY,
                        grounded: n != 4,
                        name: format!("occurrence {n}"),
                    },
                }),
            })
            .unwrap();
    }
    session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::EditDefinition {
                id: definition(2),
                command: Box::new(EditCommand::AddBody {
                    body: BodyMeta::new(
                        body(11),
                        "foreign-only body",
                        RecordId(Uuid::from_u128(11)),
                    ),
                }),
            }),
        })
        .unwrap();
    let joints = vec![joint(30, 1, 2, true), joint(31, 3, 4, false)];
    let bindings = vec![binding(&joints[0])];
    session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::SetBoundJointsAndPoses {
                joints,
                bindings,
                poses: (1..=4)
                    .map(|n| (occurrence(n), RigidTransform::IDENTITY))
                    .collect(),
            }),
        })
        .unwrap();
    session.into_document()
}

fn accepted_face_removal(document: Document, sibling_retains_face: bool) -> Document {
    let element = ElementId::new(ELEMENT);
    let mut session = DocumentSession::new(document);
    let mut replayed = session.document().definitions[&definition(1)].clone();
    assert!(replayed.elements.remove(&element).is_some());
    assert!(replayed.bodies.get(body(10)).is_some());
    session
        .adopt_component_replay(definition(1), replayed)
        .unwrap();
    let mut document = session.into_document();
    document.elements.remove(&element);
    if !sibling_retains_face {
        document
            .definitions
            .get_mut(&definition(2))
            .unwrap()
            .elements
            .remove(&element);
    }
    document
}

async fn drive(
    state: &AppState,
    session: &str,
    joints: Vec<Joint>,
    bindings: Vec<JointBinding>,
    value: f64,
) -> Result<(), onecad_app::error::ApiError> {
    let generation = state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .persisted_generation();
    joint_execution::solve_bound_joints_at_generation_cancellable(
        state,
        session,
        generation,
        joints,
        vec![(JointId(Uuid::from_u128(31)), value)],
        bindings,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .map(|_| ())
}

async fn unchanged_refusals(state: &AppState, session: &str, source: &Document) {
    let (before, depth, generation) = {
        let runtime = state.runtime.lock().await;
        let runtime = runtime.as_ref().unwrap();
        (
            serde_json::to_value(runtime.assembly_projection().unwrap()).unwrap(),
            runtime.undo_depth(),
            runtime.persisted_generation(),
        )
    };
    let mut foreign = source.assembly.joint_bindings.clone();
    foreign[0].body = body(11);
    foreign[0].element.primary.as_mut().unwrap().body = body(11);
    let error = drive(state, session, source.assembly.joints.clone(), foreign, 0.5)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("belongs to another definition"),
        "{error}"
    );
    let mut active = source.assembly.joints.clone();
    active[0].suppressed = false;
    let error = drive(
        state,
        session,
        active,
        source.assembly.joint_bindings.clone(),
        0.5,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("joint origin needs repair"),
        "{error}"
    );
    let runtime = state.runtime.lock().await;
    let runtime = runtime.as_ref().unwrap();
    assert_eq!(
        serde_json::to_value(runtime.assembly_projection().unwrap()).unwrap(),
        before
    );
    assert_eq!(
        (runtime.undo_depth(), runtime.persisted_generation()),
        (depth, generation)
    );
}

async fn exercise(document: &Document, path: &Path, binary: &Path) -> Vec<RigidTransform> {
    ContainerWriter::save(
        path,
        document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
    assert_eq!(
        ContainerReader::open(path)
            .unwrap()
            .document()
            .assembly
            .joint_bindings,
        document.assembly.joint_bindings
    );
    let worker = WorkerManager::spawn(SupervisorConfig::production(binary.to_path_buf()));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let backend = Arc::new(worker.clone());
    let runtime =
        DocumentRuntime::open_assembly(path, backend.clone(), backend.clone(), backend).unwrap();
    let session = runtime.runtime_session();
    let generation = runtime.persisted_generation();
    let depth = runtime.undo_depth();
    let state = AppState::default();
    *state.runtime.lock().await = Some(runtime);
    drive(
        &state,
        &session,
        document.assembly.joints.clone(),
        document.assembly.joint_bindings.clone(),
        0.25,
    )
    .await
    .unwrap();
    let poses = {
        let runtime = state.runtime.lock().await;
        let runtime = runtime.as_ref().unwrap();
        assert_eq!(
            (runtime.undo_depth(), runtime.persisted_generation()),
            (depth + 1, generation + 1)
        );
        let projection = runtime.assembly_projection().unwrap();
        assert!(projection.joints[0].suppressed);
        assert!(projection.joint_bindings[0].needs_repair);
        projection
            .occurrences
            .iter()
            .map(|item| item.local)
            .collect::<Vec<_>>()
    };
    assert_eq!(&poses[..3], &[RigidTransform::IDENTITY; 3]);
    assert!((poses[3].translation.z - 0.25).abs() < 1e-6);
    unchanged_refusals(&state, &session, document).await;
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(path, SaveMeta::default())
        .unwrap();
    assert_eq!(
        ContainerReader::open(path)
            .unwrap()
            .document()
            .assembly
            .joint_bindings,
        document.assembly.joint_bindings
    );
    worker.shutdown().await;
    worker.retire();
    poses
}

#[tokio::test]
async fn suppressed_deleted_face_stays_in_its_copied_definition_namespace() {
    let Some(binary) = real_worker() else { return };
    let area = tempfile::tempdir().unwrap();
    let original = copied_document();
    let initial = exercise(
        &original,
        &area.path().join("before-removal.onecad"),
        &binary,
    )
    .await;
    let no_collision = accepted_face_removal(original.clone(), false);
    let removed = exercise(
        &no_collision,
        &area.path().join("removed-without-sibling.onecad"),
        &binary,
    )
    .await;
    let collision = accepted_face_removal(original, true);
    let copied = exercise(
        &collision,
        &area.path().join("removed-with-sibling.onecad"),
        &binary,
    )
    .await;
    assert_eq!(initial, removed);
    assert_eq!(removed, copied);
}
