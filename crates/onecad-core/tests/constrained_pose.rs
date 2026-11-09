use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId},
};
use onecad_core::{
    document::{component::ComponentDef, Document},
    edit::{AssemblyEdit, DocumentSession, EditCommand},
    ids::DocumentId,
};
use uuid::Uuid;

fn session(suppressed: bool) -> DocumentSession {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
    let def = ComponentDefId(Uuid::from_u128(2));
    doc.definitions.insert(def, ComponentDef::new(def, "part"));
    for n in 3..=5 {
        doc.assembly
            .occurrences
            .insert(Occurrence {
                id: OccurrenceId(Uuid::from_u128(n)),
                def,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: n.to_string(),
            })
            .unwrap();
    }
    let frame = Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    doc.assembly.joints.push(Joint {
        id: JointId(Uuid::from_u128(6)),
        name: "joint".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: OccurrenceId(Uuid::from_u128(3)),
            frame,
        },
        b: JointOrigin {
            occurrence: OccurrenceId(Uuid::from_u128(4)),
            frame,
        },
        limits: None,
        value: 0.0,
        suppressed,
    });
    DocumentSession::new(doc)
}

fn pose(id: u128, moved: bool) -> EditCommand {
    EditCommand::Assembly {
        edit: Box::new(AssemblyEdit::SetPose {
            occurrence: OccurrenceId(Uuid::from_u128(id)),
            pose: if moved {
                RigidTransform::new(Quat::IDENTITY, Vec3d::X)
            } else {
                RigidTransform::IDENTITY
            },
        }),
    }
}

#[test]
fn constrained_endpoints_refuse_changes_without_history() {
    let mut session = session(false);
    for id in [3, 4] {
        session.apply(pose(id, false)).unwrap();
        assert!(
            session.apply(pose(id, true)).is_err(),
            "endpoint {id} moved outside solver"
        );
        assert_eq!(session.undo_depth(), 0);
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(OccurrenceId(Uuid::from_u128(id)))
                .unwrap()
                .local,
            RigidTransform::IDENTITY
        );
    }
}

#[test]
fn suppressed_endpoints_and_free_occurrences_remain_undoable() {
    for (suppressed, id) in [(true, 3), (true, 4), (false, 5)] {
        let mut session = session(suppressed);
        session.apply(pose(id, true)).unwrap();
        session.undo().unwrap();
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(OccurrenceId(Uuid::from_u128(id)))
                .unwrap()
                .local,
            RigidTransform::IDENTITY
        );
        session.redo().unwrap().unwrap();
        assert_eq!(
            session
                .document()
                .assembly
                .occurrences
                .get(OccurrenceId(Uuid::from_u128(id)))
                .unwrap()
                .local
                .translation,
            Vec3d::X
        );
    }
}
