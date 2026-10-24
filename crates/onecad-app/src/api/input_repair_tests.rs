use super::*;
use onecad_core::{
    document::{record::*, refs::PrimaryRef, variables::Scalar},
    ids::ElementId,
};
fn reference(kind: ElementKind) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body: BodyId::new(),
            kind,
            element: ElementId::new("el_test"),
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}
fn record(op: KnownOperation) -> OperationRecord {
    OperationRecord::new(RecordId::new(), 3, "repair", Operation::Known(op))
}
#[test]
fn only_complete_single_input_features_are_eligible() {
    let edge = reference(ElementKind::Edge);
    let mut fillet = FilletParams {
        radius: Scalar::new(3.0),
        edge_ids: vec![edge.primary.as_ref().unwrap().element.clone()],
        edges: vec![edge.clone()],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    };
    assert_eq!(
        input_repair_target(&record(KnownOperation::Fillet(fillet.clone())))
            .unwrap()
            .kind,
        ElementKind::Edge
    );
    fillet.edges.push(edge.clone());
    fillet.edge_ids.push(ElementId::new("el_second"));
    assert!(input_repair_target(&record(KnownOperation::Fillet(fillet))).is_err());
    let mut chamfer = ChamferParams {
        radius: Scalar::new(2.0),
        distance2: None,
        angle_deg: None,
        edge_ids: vec![edge.primary.as_ref().unwrap().element.clone()],
        edges: vec![edge],
        reference_faces: vec![],
        reference_face_refs: vec![],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    };
    assert!(input_repair_target(&record(KnownOperation::Chamfer(chamfer.clone()))).is_ok());
    chamfer.distance2 = Some(Scalar::new(1.0));
    assert!(input_repair_target(&record(KnownOperation::Chamfer(chamfer))).is_err());
    let face = reference(ElementKind::Face);
    let shell = ShellParams {
        thickness: Scalar::new(1.0),
        open_faces: vec![face.primary.as_ref().unwrap().element.clone()],
        target_body: Some(face.primary.as_ref().unwrap().body),
        faces: vec![face],
        extra: Default::default(),
    };
    assert_eq!(
        input_repair_target(&record(KnownOperation::Shell(shell)))
            .unwrap()
            .kind,
        ElementKind::Face
    );
}

#[test]
fn repair_preview_requires_reached_success_and_preserves_only_later_repair_gates() {
    use onecad_core::{
        ids::SnapshotId,
        regen::{Signature, StepSignatures},
    };
    let signature = Signature::new("0000000000000000");
    let mut preview = EditPreview {
        record: RecordId::new(),
        edited_step: Some(3),
        snapshot_id: SnapshotId(1),
        outcome: super::super::EditPreviewOutcome::Ready,
        bodies: vec![],
        meshes: vec![],
        suppressed: vec![],
        needs_repair: vec![],
        step_signatures: vec![(
            3,
            StepSignatures {
                geometry: signature.clone(),
                body_lifecycle: signature.clone(),
                referenced_binding: signature,
            },
        )],
        isolated: false,
    };
    assert!(repair_preview_ready(&preview));
    for (step, allowed) in [(2, false), (3, false), (4, true)] {
        preview.outcome = super::super::EditPreviewOutcome::DownstreamNeedsRepair {
            step,
            record: RecordId::new(),
        };
        assert_eq!(repair_preview_ready(&preview), allowed);
    }
    preview.isolated = true;
    assert!(!repair_preview_ready(&preview));
    preview.isolated = false;
    preview.outcome = super::super::EditPreviewOutcome::DownstreamFailed {
        step: 4,
        record: RecordId::new(),
        message: "bad geometry".into(),
    };
    assert!(!repair_preview_ready(&preview));
    preview.outcome = super::super::EditPreviewOutcome::Unchanged;
    assert!(!repair_preview_ready(&preview));
    preview.outcome = super::super::EditPreviewOutcome::Ready;
    preview.step_signatures.clear();
    assert!(
        !repair_preview_ready(&preview),
        "a Ready label alone cannot prove this step executed"
    );
}
