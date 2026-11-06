use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId, OccurrenceTree},
};
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

fn refuses_unknown<T: Serialize + DeserializeOwned>(value: T) {
    let mut raw = serde_json::to_value(value).unwrap();
    raw["futureMeaning"] = true.into();
    assert!(serde_json::from_value::<T>(raw).is_err());
}

#[test]
fn nested_pose_extensions_are_refused_instead_of_dropped() {
    refuses_unknown(Vec3d::ZERO);
    refuses_unknown(Quat::IDENTITY);
    refuses_unknown(RigidTransform::IDENTITY);
    refuses_unknown(Frame {
        origin: Vec3d::ZERO,
        x: Vec3d::X,
        z: Vec3d::Z,
    });
}

#[test]
fn occurrence_and_joint_extensions_are_refused_instead_of_dropped() {
    let id = OccurrenceId(Uuid::from_u128(1));
    let frame = Frame {
        origin: Vec3d::ZERO,
        x: Vec3d::X,
        z: Vec3d::Z,
    };
    refuses_unknown(Occurrence {
        id,
        def: ComponentDefId(Uuid::from_u128(2)),
        parent: None,
        local: RigidTransform::IDENTITY,
        grounded: true,
        name: "Part".into(),
    });
    refuses_unknown(OccurrenceTree::new());
    refuses_unknown(JointLimits { min: 0.0, max: 1.0 });
    refuses_unknown(JointOrigin {
        occurrence: id,
        frame,
    });
    refuses_unknown(Joint {
        id: JointId(Uuid::from_u128(3)),
        name: "Slider".into(),
        kind: JointKind::Slider,
        a: JointOrigin {
            occurrence: id,
            frame,
        },
        b: JointOrigin {
            occurrence: OccurrenceId(Uuid::from_u128(4)),
            frame,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    });
}
