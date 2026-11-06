use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointOrigin},
    math::{Frame, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrencePath},
};
use onecad_core::{
    document::{
        component::{ComponentDef, JointBinding, JointSide, JointSurface},
        refs::{AnchorIntent, ElementKind, ElementRef, IntentQuery, PrimaryRef},
        variables::{Scalar, Variable},
        Document,
    },
    edit::{AssemblyEdit, DocumentSession, EditCommand},
    ids::{BodyId, DocumentId, ElementId, VariableId},
    math::Vec3,
};
use uuid::Uuid;

#[test]
fn bound_origin_preserves_sibling_identity_and_repair_history() {
    let id = |n| Uuid::from_u128(n);
    let definition = ComponentDefId(id(2));
    let a = OccurrenceId(id(3));
    let b = OccurrenceId(id(4));
    let mut document = Document::new(DocumentId(id(1)));
    document
        .definitions
        .insert(definition, ComponentDef::new(definition, "part"));
    for occurrence in [a, b] {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occurrence,
                def: definition,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: "part".into(),
            })
            .unwrap();
    }
    let frame = Frame {
        origin: Vec3d::ZERO,
        x: Vec3d::X,
        z: Vec3d::Z,
    };
    let joint = Joint {
        id: JointId(id(5)),
        name: "joint".into(),
        kind: JointKind::Rigid,
        a: JointOrigin {
            occurrence: a,
            frame,
        },
        b: JointOrigin {
            occurrence: b,
            frame,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    let body = BodyId(id(6));
    let binding = JointBinding {
        joint: joint.id,
        side: JointSide::A,
        path: OccurrencePath(vec![a]),
        definition,
        body,
        element: ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new("el_test"),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: Some(IntentQuery {
                version: 1,
                kind: ElementKind::Face,
                descriptor: serde_json::json!({"surfaceType": 0}),
                extra: Default::default(),
            }),
            anchor: Some(AnchorIntent {
                world_point: Vec3::new(0.0, 0.0, 0.0).unwrap(),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
        surface: JointSurface::Plane,
        radius: None,
        orientation_x: Vec3d::X,
        needs_repair: false,
    };
    let mut session = DocumentSession::new(document);
    let apply = |session: &mut DocumentSession, edit| {
        session.apply(EditCommand::Assembly {
            edit: Box::new(edit),
        })
    };
    let poses = vec![(a, RigidTransform::IDENTITY), (b, RigidTransform::IDENTITY)];
    let mut wrong = binding.clone();
    wrong.path = OccurrencePath(vec![b]);
    assert!(apply(
        &mut session,
        AssemblyEdit::SetBoundJointsAndPoses {
            joints: vec![joint.clone()],
            bindings: vec![wrong],
            poses: poses.clone()
        }
    )
    .is_err());
    apply(
        &mut session,
        AssemblyEdit::SetBoundJointsAndPoses {
            joints: vec![joint],
            bindings: vec![binding],
            poses,
        },
    )
    .unwrap();
    apply(
        &mut session,
        AssemblyEdit::EditDefinition {
            id: definition,
            command: Box::new(EditCommand::AddVariable {
                variable: Variable {
                    id: VariableId(id(7)),
                    name: "width".into(),
                    value: Scalar::new(1.0),
                    unit: Default::default(),
                },
            }),
        },
    )
    .unwrap();
    assert!(session.document().assembly.joint_bindings[0].needs_repair);
    session.undo().unwrap();
    assert!(!session.document().assembly.joint_bindings[0].needs_repair);
    session.redo().unwrap().unwrap();
    assert!(session.document().assembly.joint_bindings[0].needs_repair);
    assert_eq!(
        session
            .document()
            .assembly
            .occurrences
            .get(b)
            .unwrap()
            .local,
        RigidTransform::IDENTITY
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bound.onecad");
    let meta = onecad_core::io::container::SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    onecad_core::io::container::ContainerWriter::save(
        &path,
        session.document(),
        &onecad_core::io::container::ContainerCaches::none(),
        &meta,
    )
    .unwrap();
    let reopened = onecad_core::io::container::ContainerReader::open(&path).unwrap();
    assert!(reopened.document().assembly.joint_bindings[0].needs_repair);
    assert_eq!(
        reopened.document().assembly.joint_bindings[0].path,
        OccurrencePath(vec![a])
    );
}
