//! R-WP5 edit/session integration tests.
//!
//! Covers (task point 10): (a) per-command apply→undo→redo structural equality
//! across ALL `EditCommand` variants, (b) proptest random valid sequences
//! (apply-all → undo-all == initial; stack cap 200), (c) transactions as a
//! single undo unit with combined outcome, (d) the RegenHint/DirtyRange table,
//! (e) anti-time-travel rejection, (g) a session insta snapshot.
//!
//! (f) selection rules live in the `selection` module's unit tests.

use proptest::prelude::*;
use uuid::Uuid;

use onecad_core::document::body::BodyMeta;
use onecad_core::document::datum::DatumPlane;
use onecad_core::document::record::PlaneKind;
use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ChamferParams, ComponentSourceRef,
    DetachComponentParams, ExtrudeMode, ExtrudeParams, FilletParams, FrozenPlacement, HoleParams,
    HoleType, KnownOperation, OffsetDistanceType, OffsetFaceParams, Operation, OperationRecord,
    PlaceComponentParams, RevolveParams, ShellParams, SketchOpParams, SketchPlaneRef,
    HOLE_RESULT_POLICY_VERSION,
};
use onecad_core::document::refs::{
    AnchorIntent, AxisRef, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::{Scalar, Unit, Variable};
use onecad_core::document::Document;
use onecad_core::edit::{
    DocumentSession, EditCommand, InputPath, InputRef, RegenHint, SketchEditOp, VisibilityTarget,
};
use onecad_core::history::{DirtyRange, Timeline};
use onecad_core::ids::{
    BodyId, ConstraintId, DatumPlaneId, DocumentId, ElementId, EntityId, RecordId, RegionId,
    SketchId, VariableId,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane,
};

// ── id helpers ───────────────────────────────────────────────────────────────

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn rid(n: u128) -> RecordId {
    RecordId(u(0x2E00 + n))
}
fn bid(n: u128) -> BodyId {
    BodyId(u(0xB000 + n))
}
fn sid(n: u128) -> SketchId {
    SketchId(u(0x5C00 + n))
}
fn eid(n: u128) -> EntityId {
    EntityId(u(0xE000 + n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(u(0xC000 + n))
}
fn vid(n: u128) -> VariableId {
    VariableId(u(0x7000 + n))
}
fn did(n: u128) -> DatumPlaneId {
    DatumPlaneId(u(0xD000 + n))
}

const S1: fn() -> SketchId = || sid(1);
const BX: fn() -> BodyId = || bid(1);
const BY: fn() -> BodyId = || bid(2);
const B0: fn() -> BodyId = || bid(0);

fn s(v: f64) -> Scalar {
    Scalar::new(v)
}

// ── op-record builders ───────────────────────────────────────────────────────

fn profile() -> SketchRegionRef {
    SketchRegionRef {
        sketch: S1(),
        region: RegionId::new("r0"),
        region_identity_version: None,
        extra: Default::default(),
    }
}

fn record(id: RecordId, name: &str, op: Operation, outputs: Vec<BodyId>) -> OperationRecord {
    let mut r = OperationRecord::new(id, 0, name, op);
    r.outputs = outputs;
    r
}

fn sketch_op(id: RecordId, sketch: SketchId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch,
        plane: SketchPlaneRef {
            kind: PlaneKind::Xy,
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: Default::default(),
        },
        entities: vec![],
        constraints: vec![],
        host_face: None,
        extra: Default::default(),
    }));
    record(id, "Sketch", op, vec![])
}

fn extrude_newbody(id: RecordId, distance: f64, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile()),
        distance: s(distance),
        draft_angle_deg: s(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: s(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    record(id, "Extrude", op, vec![out])
}

fn extrude_cut(id: RecordId, target: BodyId, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile()),
        distance: s(5.0),
        draft_angle_deg: s(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::Cut,
        target_body: Some(target),
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: s(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    record(id, "Cut", op, vec![out])
}

fn boolean(id: RecordId, target: BodyId, tool: BodyId, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Boolean(BooleanParams {
        operation: BooleanOp::Union,
        target_body: target,
        tool_body: tool,
        extra: Default::default(),
    }));
    record(id, "Boolean", op, vec![out])
}

fn edge_ref(body: BodyId, el: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(el),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}

fn fillet(id: RecordId, body: BodyId, el: &str, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: s(2.0),
        edge_ids: vec![ElementId::new(el)],
        edges: vec![edge_ref(body, el)],
        chain_tangent_edges: true,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    record(id, "Fillet", op, vec![out])
}

fn revolve(id: RecordId, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Revolve(RevolveParams {
        profile: Some(profile()),
        angle_deg: s(360.0),
        axis: Some(AxisRef::SketchLine {
            sketch: S1(),
            line: eid(0x20),
            extra: Default::default(),
        }),
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        extra: Default::default(),
    }));
    record(id, "Revolve", op, vec![out])
}

// ── base document ────────────────────────────────────────────────────────────

/// A sketch with two points, a line and a distance constraint (so SketchEdit
/// SetDimension / SetEntityPositions have valid targets).
fn base_sketch() -> Sketch {
    let mut sk = Sketch::on_world_plane(S1(), "Sketch 1", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        eid(1),
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::point(
        eid(2),
        Vec2::new_unchecked(40.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::line(eid(0x20), eid(1), eid(2), false))
        .unwrap();
    sk.add_constraint(Constraint::Distance {
        id: cid(1),
        entity1: eid(1),
        entity1_position: CurvePosition::Arbitrary,
        entity2: eid(2),
        entity2_position: CurvePosition::Arbitrary,
        value: s(40.0),
    })
    .unwrap();
    sk
}

fn base_document() -> Document {
    let mut doc = Document::new(DocumentId(u(1)));
    doc.settings.tolerance_policy_hash = "tol".into();
    doc.sketches.insert(S1(), base_sketch());
    doc.datum_planes.insert(
        did(1),
        DatumPlane::offset_from_plane(did(1), "Datum 1", "XY", 0.0),
    );
    doc.bodies.register(BodyMeta::new(B0(), "Imported", rid(0)));
    doc.variables.upsert(Variable {
        id: vid(1),
        name: "width".into(),
        value: s(40.0),
        unit: Unit::Mm,
    });
    doc.timeline = Timeline::from_records(vec![
        sketch_op(rid(0), S1()),
        extrude_newbody(rid(1), 25.0, BX()),
        extrude_newbody(rid(2), 10.0, BY()),
        boolean(rid(3), BX(), BY(), BX()),
        fillet(rid(4), BX(), "el_e1", BX()),
    ]);
    doc
}

fn json(doc: &Document) -> serde_json::Value {
    serde_json::to_value(doc).unwrap()
}

// ── (a) per-command apply → undo → redo across ALL variants ──────────────────

/// One representative, valid command per `EditCommand` variant.
fn all_variant_commands() -> Vec<(&'static str, EditCommand)> {
    let after_sketch = {
        let mut sk = Sketch::on_world_plane(S1(), "Sketch 1", WorldPlane::XY);
        sk.add_entity(SketchEntity::point(
            eid(1),
            Vec2::new_unchecked(5.0, 5.0),
            false,
            false,
        ))
        .unwrap();
        sk
    };
    vec![
        (
            "AddOperation",
            EditCommand::AddOperation {
                record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
                at_cursor: true,
            },
        ),
        (
            "UpdateOperationParams",
            EditCommand::UpdateOperationParams {
                record: rid(1),
                op: extrude_newbody(rid(1), 99.0, BX()).op,
            },
        ),
        (
            "EditOperationInput",
            EditCommand::EditOperationInput {
                record: rid(4),
                path: InputPath::FilletEdges { index: 0 },
                reference: InputRef::Element(edge_ref(BX(), "el_e_new")),
            },
        ),
        (
            "RemoveOperation",
            EditCommand::RemoveOperation { record: rid(1) },
        ),
        ("SetRollback", EditCommand::SetRollback { cursor: 2 }),
        (
            "SetOperationSuppression",
            EditCommand::SetOperationSuppression {
                record: rid(4),
                suppressed: true,
                cascade: false,
            },
        ),
        (
            "AddSketch",
            EditCommand::AddSketch {
                sketch: Sketch::on_world_plane(sid(2), "Sketch 2", WorldPlane::XZ),
            },
        ),
        ("DeleteSketch", EditCommand::DeleteSketch { sketch: S1() }),
        (
            "RenameSketch",
            EditCommand::RenameSketch {
                sketch: S1(),
                name: "Renamed".into(),
            },
        ),
        (
            "UpdateSketchAttachment",
            EditCommand::UpdateSketchAttachment {
                sketch: S1(),
                plane: WorldPlane::XZ.plane(),
                attachment: SketchAttachment::World {
                    plane: WorldPlane::XZ,
                },
            },
        ),
        (
            "SketchEdit",
            EditCommand::SketchEdit {
                sketch: S1(),
                ops: vec![
                    SketchEditOp::SetDimension {
                        constraint: cid(1),
                        value: s(55.0),
                    },
                    SketchEditOp::SetEntityPositions {
                        positions: vec![(eid(1), Vec2::new_unchecked(1.0, 2.0))],
                    },
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(
                            eid(9),
                            Vec2::new_unchecked(9.0, 9.0),
                            false,
                            false,
                        ),
                    },
                    SketchEditOp::SetEntityConstruction {
                        entity: eid(1),
                        construction: true,
                    },
                ],
            },
        ),
        (
            "SketchDragGesture",
            EditCommand::SketchDragGesture {
                sketch: S1(),
                before: base_sketch(),
                after: after_sketch,
            },
        ),
        (
            "AddBody",
            EditCommand::AddBody {
                body: BodyMeta::new(bid(0x50), "New Body", rid(0)),
            },
        ),
        ("DeleteBody", EditCommand::DeleteBody { body: B0() }),
        (
            "RenameBody",
            EditCommand::RenameBody {
                body: B0(),
                name: "Renamed Body".into(),
            },
        ),
        (
            "SetVisibilityBody",
            EditCommand::SetVisibility {
                target: VisibilityTarget::Body(B0()),
                visible: false,
            },
        ),
        (
            "SetVisibilitySketch",
            EditCommand::SetVisibility {
                target: VisibilityTarget::Sketch(S1()),
                visible: false,
            },
        ),
        (
            "AddDatumPlane",
            EditCommand::AddDatumPlane {
                datum: DatumPlane::offset_from_plane(did(2), "Datum 2", "XZ", 10.0),
            },
        ),
        (
            // `base_document` seeds did(1) with no sketch attached to it.
            "DeleteDatum",
            EditCommand::DeleteDatum { datum: did(1) },
        ),
        (
            "SetVariable",
            EditCommand::SetVariable {
                variable: vid(1),
                value: s(88.0),
            },
        ),
        (
            "AddVariable",
            EditCommand::AddVariable {
                variable: Variable {
                    id: vid(2),
                    name: "height".into(),
                    value: s(20.0),
                    unit: Unit::Mm,
                },
            },
        ),
        (
            "RemoveVariable",
            EditCommand::RemoveVariable { variable: vid(1) },
        ),
    ]
}

#[test]
fn every_command_apply_undo_restores_and_redo_reapplies() {
    let base = base_document();
    let initial = json(&base);
    let mut count = 0;
    for (name, cmd) in all_variant_commands() {
        count += 1;
        let mut sess = DocumentSession::new(base.clone());
        sess.apply(cmd)
            .unwrap_or_else(|e| panic!("{name}: apply failed: {e}"));
        let after = json(sess.document());
        assert_ne!(after, initial, "{name}: apply must change the document");

        assert!(sess.undo().is_some(), "{name}: undo available");
        assert_eq!(
            json(sess.document()),
            initial,
            "{name}: undo must restore exactly"
        );

        assert!(sess.redo().unwrap().is_some(), "{name}: redo available");
        assert_eq!(
            json(sess.document()),
            after,
            "{name}: redo must reapply exactly"
        );

        // undo→redo is idempotent for state.
        assert!(sess.undo().is_some());
        assert_eq!(
            json(sess.document()),
            initial,
            "{name}: second undo restores"
        );
    }
    assert_eq!(count, 22, "all 22 EditCommand variants covered");
}

// ── EditOperationInput: every InputPath branch ───────────────────────────────

#[test]
fn edit_operation_input_all_paths() {
    // A document with an extrude, a boolean and a revolve to exercise each path.
    let mut doc = Document::new(DocumentId(u(2)));
    doc.timeline = Timeline::from_records(vec![
        extrude_newbody(rid(1), 25.0, BX()),
        extrude_newbody(rid(2), 10.0, BY()),
        boolean(rid(3), BX(), BY(), BX()),
        fillet(rid(4), BX(), "el_e1", BX()),
        revolve(rid(5), bid(3)),
    ]);
    let initial = json(&doc);

    let cases: Vec<(&str, RecordId, InputPath, InputRef)> = vec![
        (
            "extrude profile",
            rid(1),
            InputPath::ExtrudeProfile,
            InputRef::Region(SketchRegionRef {
                sketch: sid(9),
                region: RegionId::new("r9"),
                region_identity_version: None,
                extra: Default::default(),
            }),
        ),
        (
            "fillet edge",
            rid(4),
            InputPath::FilletEdges { index: 0 },
            InputRef::Element(edge_ref(BX(), "el_edited")),
        ),
        (
            // swap target BX -> BY (produced before R3, so not time-travel)
            "boolean target",
            rid(3),
            InputPath::BooleanTarget,
            InputRef::Body(BY()),
        ),
        (
            // swap tool BY -> BX
            "boolean tool",
            rid(3),
            InputPath::BooleanTool,
            InputRef::Body(BX()),
        ),
        (
            "revolve axis",
            rid(5),
            InputPath::RevolveAxis,
            InputRef::Element(edge_ref(BX(), "el_axis")),
        ),
    ];

    for (name, rec, path, reference) in cases {
        let mut sess = DocumentSession::new(doc.clone());
        sess.apply(EditCommand::EditOperationInput {
            record: rec,
            path,
            reference,
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_ne!(json(sess.document()), initial, "{name} changed the doc");
        assert!(sess.undo().is_some());
        assert_eq!(json(sess.document()), initial, "{name} undo restored");
    }
}

#[test]
fn fillet_edit_keeps_edge_ids_and_edges_in_lockstep() {
    let mut doc = Document::new(DocumentId(u(3)));
    doc.timeline = Timeline::from_records(vec![fillet(rid(4), BX(), "el_old", BX())]);
    let mut sess = DocumentSession::new(doc);
    sess.apply(EditCommand::EditOperationInput {
        record: rid(4),
        path: InputPath::FilletEdges { index: 0 },
        reference: InputRef::Element(edge_ref(BX(), "el_fresh")),
    })
    .unwrap();
    let v = json(sess.document());
    let params = &v["timeline"]["records"][0]["params"];
    assert_eq!(params["edgeIds"][0], "el_fresh", "bare edge_ids updated");
    assert_eq!(
        params["edges"][0]["primary"]["elementId"], "el_fresh",
        "typed edges ref updated in lockstep"
    );
}

// ── (e) anti-time-travel ─────────────────────────────────────────────────────

#[test]
fn add_operation_referencing_later_body_is_rejected() {
    let mut doc = Document::new(DocumentId(u(4)));
    doc.timeline = Timeline::from_records(vec![
        extrude_newbody(rid(1), 25.0, bid(0xA)),
        extrude_newbody(rid(2), 10.0, bid(0xC)), // produces bodyC at index 1
    ]);
    doc.timeline.set_cursor(1); // insert before the op that produces bodyC
    let mut sess = DocumentSession::new(doc);
    // A Cut that targets bodyC (produced by a LATER op) is anti-time-travel.
    let err = sess
        .apply(EditCommand::AddOperation {
            record: extrude_cut(rid(9), bid(0xC), bid(0xD)),
            at_cursor: true,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("anti-time-travel"),
        "expected anti-time-travel rejection, got: {err}"
    );
}

// ── (d) RegenHint / DirtyRange table ─────────────────────────────────────────

#[test]
fn regen_hint_and_dirty_range_table() {
    let base = base_document();
    let len = base.timeline.len(); // 5

    let check = |cmd: EditCommand, want_dirty: Option<DirtyRange>, want_regen: RegenHint| {
        let mut sess = DocumentSession::new(base.clone());
        let out = sess.apply(cmd).unwrap();
        assert_eq!(out.dirty, want_dirty, "dirty mismatch");
        assert_eq!(out.regen, want_regen, "regen mismatch");
    };

    // AddOperation at cursor (=len): inserts at len, dirties [len, len+1), ToEnd.
    check(
        EditCommand::AddOperation {
            record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
            at_cursor: true,
        },
        Some(DirtyRange::new(len, len + 1)),
        RegenHint::ToEnd,
    );
    // UpdateOperationParams on index 1: [1, len), ToEnd.
    check(
        EditCommand::UpdateOperationParams {
            record: rid(1),
            op: extrude_newbody(rid(1), 99.0, BX()).op,
        },
        Some(DirtyRange::new(1, len)),
        RegenHint::ToEnd,
    );
    // RemoveOperation index 1: [1, len-1), ToEnd.
    check(
        EditCommand::RemoveOperation { record: rid(1) },
        Some(DirtyRange::new(1, len - 1)),
        RegenHint::ToEnd,
    );
    // SetRollback to 2 (backward from len): span [2, len), PreviewTo(1).
    // DOMAIN SPLIT: the command's `cursor` is an applied-op COUNT, `PreviewTo` (and the
    // `RegenRequest::ToStep` the scheduler maps it to) is a step INDEX — the last applied
    // step of a 2-op prefix is index 1. Pinning the count here pinned the defect where
    // every non-zero roll asked for a step past the applied end and published nothing.
    check(
        EditCommand::SetRollback { cursor: 2 },
        Some(DirtyRange::new(2, len)),
        RegenHint::PreviewTo(1),
    );
    // SetVisibility body: metadata only.
    check(
        EditCommand::SetVisibility {
            target: VisibilityTarget::Body(B0()),
            visible: false,
        },
        None,
        RegenHint::None,
    );
    // RenameSketch: metadata only.
    check(
        EditCommand::RenameSketch {
            sketch: S1(),
            name: "x".into(),
        },
        None,
        RegenHint::None,
    );
    // SetVariable: conservative whole-timeline dirty, ToEnd.
    check(
        EditCommand::SetVariable {
            variable: vid(1),
            value: s(1.0),
        },
        Some(DirtyRange::new(0, len)),
        RegenHint::ToEnd,
    );
    // SketchEdit on S1: produced by the Sketch op at index 0 (regen re-runs region
    // detection there), so it dirties from the producer, not the first consumer at
    // index 1 (F4): [0, len), ToEnd.
    check(
        EditCommand::SketchEdit {
            sketch: S1(),
            ops: vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(eid(9), Vec2::new_unchecked(1.0, 1.0), false, false),
            }],
        },
        Some(DirtyRange::new(0, len)),
        RegenHint::ToEnd,
    );
    // AddSketch (no dependents yet): metadata only.
    check(
        EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(sid(5), "S5", WorldPlane::XY),
        },
        None,
        RegenHint::None,
    );
}

// ── Frontier-append promotion (MODEL-HARDEN W0) ──────────────────────────────
//
// The frontend always commits with `at_cursor:false`; a fresh commit at the
// applied frontier (`cursor == len`) must JOIN the applied prefix so it regens
// (`ToEnd`), not sit as a permanent draft (the reported P0 defect). A rolled-back
// append (`cursor < len`) stays a draft (`RegenHint::None`).

#[test]
fn add_operation_append_at_frontier_is_applied_and_regens_to_end() {
    let base = base_document();
    let len = base.timeline.len(); // 5, cursor at the end (all applied)
    assert_eq!(base.timeline.cursor(), len, "base is fully applied");
    let mut sess = DocumentSession::new(base);

    let out = sess
        .apply(EditCommand::AddOperation {
            record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
            at_cursor: false, // the exact frontend wire shape
        })
        .unwrap();

    assert_eq!(out.regen, RegenHint::ToEnd, "frontier append regens to end");
    assert_eq!(
        out.dirty,
        Some(DirtyRange::new(len, len + 1)),
        "dirties exactly the appended step"
    );
    assert_eq!(
        sess.document().timeline.cursor(),
        len + 1,
        "the record joined the applied prefix (cursor advanced)"
    );
    assert!(
        out.projection_delta.cursor_changed,
        "frontier append moves the cursor ⇒ cursor_changed"
    );
}

#[test]
fn add_operation_append_while_rolled_back_stays_draft() {
    let base = base_document();
    let len = base.timeline.len(); // 5
    let mut sess = DocumentSession::new(base);
    // Roll back to k=2 (a real rollback: ops 2..len become drafts).
    sess.apply(EditCommand::SetRollback { cursor: 2 }).unwrap();
    assert_eq!(sess.document().timeline.cursor(), 2);

    let out = sess
        .apply(EditCommand::AddOperation {
            record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
            at_cursor: false,
        })
        .unwrap();

    assert_eq!(
        out.regen,
        RegenHint::None,
        "a rolled-back append stays a draft (no regen)"
    );
    assert!(
        !out.projection_delta.cursor_changed,
        "the draft append does not move the cursor"
    );
    assert_eq!(
        sess.document().timeline.cursor(),
        2,
        "cursor is unchanged (record sits beyond the rollback bar)"
    );
    assert_eq!(
        sess.document().timeline.len(),
        len + 1,
        "the draft record is still stored"
    );
}

#[test]
fn append_undo_redo_cursor_roundtrip() {
    let base = base_document();
    let len = base.timeline.len(); // 5
    let mut sess = DocumentSession::new(base);

    // Frontier append → the record joins the applied prefix.
    sess.apply(EditCommand::AddOperation {
        record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
        at_cursor: false,
    })
    .unwrap();
    assert!(sess.document().timeline.record_by_id(rid(0x99)).is_some());
    assert_eq!(sess.document().timeline.cursor(), len + 1);

    // Undo: the record is gone and the cursor steps back into the prefix.
    assert!(sess.undo().is_some(), "undo the append");
    assert!(
        sess.document().timeline.record_by_id(rid(0x99)).is_none(),
        "undo removed the appended record"
    );
    assert_eq!(sess.document().timeline.len(), len, "back to base length");
    assert_eq!(
        sess.document().timeline.cursor(),
        len,
        "undo restored the applied cursor"
    );

    // Redo re-runs the (fixed) forward command: the record re-joins the applied
    // prefix (pins the redo-draft regression — a re-applied commit must not become
    // a permanent draft).
    assert!(sess.redo().unwrap().is_some(), "redo the append");
    assert!(
        sess.document().timeline.record_by_id(rid(0x99)).is_some(),
        "redo re-added the record"
    );
    assert_eq!(
        sess.document().timeline.cursor(),
        len + 1,
        "redo re-advanced the cursor (record applied again, not a draft)"
    );
}

#[test]
fn frontier_append_persists_applied_cursor_across_roundtrip() {
    // A frontier-appended commit that joined the applied prefix must SURVIVE a
    // save/reopen with `cursor == len` (Timeline persists `{records, cursor}`;
    // document/mod.rs). A document saved under the old draft bug keeps `cursor < len`
    // and is intentionally NOT auto-promoted on load (indistinguishable from a real
    // rollback) — this pins the fixed-path persistence, not legacy recovery.
    let base = base_document();
    let len = base.timeline.len(); // 5
    let mut sess = DocumentSession::new(base);
    sess.apply(EditCommand::AddOperation {
        record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
        at_cursor: false,
    })
    .unwrap();
    assert_eq!(sess.document().timeline.cursor(), len + 1);

    let wire = serde_json::to_value(sess.document()).unwrap();
    let reloaded: Document = serde_json::from_value(wire).unwrap();
    assert_eq!(reloaded.timeline.len(), len + 1, "record persisted");
    assert_eq!(
        reloaded.timeline.cursor(),
        reloaded.timeline.len(),
        "reopened cursor == len (the appended op stays applied)"
    );
}

// ── (c) transactions ─────────────────────────────────────────────────────────

#[test]
fn transaction_is_single_undo_unit_with_combined_outcome() {
    let base = base_document();
    let initial = json(&base);
    let mut sess = DocumentSession::new(base);

    sess.begin_transaction("batch");
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: extrude_newbody(rid(1), 99.0, BX()).op,
    })
    .unwrap();
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(3),
        op: boolean(rid(3), BX(), BY(), BX()).op,
    })
    .unwrap();
    let combined = sess.end_transaction().unwrap();

    // One undo unit.
    assert_eq!(sess.undo_depth(), 1, "batched into a single undo step");
    // Combined dirty is the union: [1, len) ∪ [3, len) = [1, len).
    assert_eq!(
        combined.dirty,
        Some(DirtyRange::new(1, base_document().timeline.len()))
    );
    assert_eq!(combined.regen, RegenHint::ToEnd);
    assert!(combined.projection_delta.timeline_changed);

    // A single undo restores BOTH edits.
    assert!(sess.undo().is_some());
    assert_eq!(
        json(sess.document()),
        initial,
        "one undo reverts the whole batch"
    );
}

#[test]
fn cancel_transaction_rolls_back_without_undo_entry() {
    let base = base_document();
    let initial = json(&base);
    let mut sess = DocumentSession::new(base);
    sess.begin_transaction("batch");
    sess.apply(EditCommand::RenameBody {
        body: B0(),
        name: "temp".into(),
    })
    .unwrap();
    sess.cancel_transaction();
    assert_eq!(json(sess.document()), initial, "cancel rolls back");
    assert_eq!(sess.undo_depth(), 0, "cancelled txn leaves no undo entry");
}

// ── (F1) transaction failure auto-cancels the whole batch ────────────────────

#[test]
fn failed_apply_in_transaction_auto_cancels_and_undo_hits_previous_txn() {
    let base = base_document();
    let initial = json(&base);
    let mut sess = DocumentSession::new(base);

    // A first, committed transaction (its own single undo step).
    sess.apply(EditCommand::RenameBody {
        body: B0(),
        name: "Committed".into(),
    })
    .unwrap();
    let after_committed = json(sess.document());
    assert_ne!(after_committed, initial);
    assert_eq!(sess.undo_depth(), 1);

    // Open a transaction, apply one good edit, then one that fails validation.
    sess.begin_transaction("batch");
    sess.apply(EditCommand::RenameBody {
        body: B0(),
        name: "PartialBatch".into(),
    })
    .unwrap();
    let err = sess
        .apply(EditCommand::RemoveOperation { record: rid(999) }) // not present
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("not found")
            || err.to_string().to_lowercase().contains("recordnotfound")
            || matches!(err, onecad_core::error::DomainError::RecordNotFound(_)),
        "expected a not-found error, got: {err}"
    );

    // The whole transaction rolled back to the pre-transaction state.
    assert_eq!(
        json(sess.document()),
        after_committed,
        "auto-cancel restores the pre-transaction state (partial batch edit reverted)"
    );

    // The transaction is closed: undo runs again (it is ignored while a txn is
    // open) and the cancelled batch left no undo entry.
    assert_eq!(
        sess.undo_depth(),
        1,
        "cancelled batch adds no undo entry; only the committed txn remains"
    );
    assert!(sess.undo().is_some(), "undo runs — no open transaction");
    assert_eq!(
        json(sess.document()),
        initial,
        "undo reverts the PREVIOUS committed txn, not the cancelled batch"
    );
}

// ── (F2) fillet/chamfer edge lockstep on add/update entry paths ──────────────

fn fillet_op(edge_ids: &[&str], edges: Vec<ElementRef>) -> Operation {
    Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: s(2.0),
        edge_ids: edge_ids.iter().map(|e| ElementId::new(*e)).collect(),
        edges,
        chain_tangent_edges: true,
        tangent_closure_version: None,
        extra: Default::default(),
    }))
}

#[test]
fn add_operation_validates_fillet_edge_lockstep() {
    let fresh = || {
        let mut d = Document::new(DocumentId(u(0x5A)));
        d.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
        d
    };
    let add = |op: Operation| EditCommand::AddOperation {
        record: record(rid(1), "Fillet", op, vec![BX()]),
        at_cursor: true,
    };

    // Mismatched count rejected (2 ids, 1 typed edge).
    let mut sess = DocumentSession::new(fresh());
    let err = sess
        .apply(add(fillet_op(&["e1", "e2"], vec![edge_ref(BX(), "e1")])))
        .unwrap_err();
    assert!(
        err.to_string().contains("mismatch"),
        "count mismatch: {err}"
    );

    // Primary element disagreeing with the parallel edge_ids entry rejected.
    let mut sess = DocumentSession::new(fresh());
    let err = sess
        .apply(add(fillet_op(&["e1"], vec![edge_ref(BX(), "eX")])))
        .unwrap_err();
    assert!(
        err.to_string().contains("!="),
        "primary disagreement: {err}"
    );

    // Consistent (equal length, matching primary) accepted.
    let mut sess = DocumentSession::new(fresh());
    sess.apply(add(fillet_op(&["e1"], vec![edge_ref(BX(), "e1")])))
        .expect("consistent fillet accepted");

    // Empty typed edges (legacy bare-ids form) accepted.
    let mut sess = DocumentSession::new(fresh());
    sess.apply(add(fillet_op(&["e1"], vec![])))
        .expect("bare-ids fillet accepted");
}

#[test]
fn update_operation_params_validates_fillet_edge_lockstep() {
    let mut doc = Document::new(DocumentId(u(0x5B)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Fillet",
        fillet_op(&["e1"], vec![]),
        vec![BX()],
    )]);
    let mut sess = DocumentSession::new(doc);
    // Update to a mismatched typed-edge set is rejected.
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet_op(&["e1", "e2"], vec![edge_ref(BX(), "e1")]),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("mismatch"),
        "update mismatch: {err}"
    );
    // Update to a consistent set is accepted.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: fillet_op(&["e9"], vec![edge_ref(BX(), "e9")]),
    })
    .expect("consistent update accepted");
}

// ── (W3) the ONE sanctioned opType edit: Fillet ⇄ Chamfer ────────────────────

fn chamfer_op(edge_ids: &[&str], edges: Vec<ElementRef>, radius: f64) -> Operation {
    Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: s(radius),
        distance2: None,
        angle_deg: None,
        edge_ids: edge_ids.iter().map(|e| ElementId::new(*e)).collect(),
        edges,
        chain_tangent_edges: true,
        tangent_closure_version: None,
        extra: Default::default(),
    }))
}

/// A one-record fillet document: `rid(1)` fillets `el` on body `BX` at r=2.
fn fillet_doc(el: &str) -> DocumentSession {
    let mut doc = Document::new(DocumentId(u(0x5D)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Fillet",
        fillet_op(&[el], vec![edge_ref(BX(), el)]),
        vec![BX()],
    )]);
    DocumentSession::new(doc)
}

/// `(edge_ids, typed-edge count, radius, `extra.mode`)` of a fillet/chamfer op.
fn edge_op_facts(op: &Operation) -> (Vec<String>, usize, f64, Option<String>) {
    let (ids, edges, radius, extra) = match op {
        Operation::Known(KnownOperation::Fillet(p)) => {
            (&p.edge_ids, &p.edges, p.radius.value, &p.extra)
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            (&p.edge_ids, &p.edges, p.radius.value, &p.extra)
        }
        other => panic!("not an edge op: {}", other.op_type()),
    };
    (
        ids.iter().map(|e| e.as_str().to_string()).collect(),
        edges.len(),
        radius,
        extra
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    )
}

fn set_closure_version(op: &mut Operation, version: Option<u8>) {
    match op {
        Operation::Known(KnownOperation::Fillet(p)) => p.tangent_closure_version = version,
        Operation::Known(KnownOperation::Chamfer(p)) => p.tangent_closure_version = version,
        _ => panic!("not an edge op"),
    }
}

fn closure_version(op: &Operation) -> Option<u8> {
    match op {
        Operation::Known(KnownOperation::Fillet(p)) => p.tangent_closure_version,
        Operation::Known(KnownOperation::Chamfer(p)) => p.tangent_closure_version,
        _ => panic!("not an edge op"),
    }
}

#[test]
fn update_operation_params_swaps_fillet_to_chamfer() {
    let mut sess = fillet_doc("e1");
    // A record that carried the legacy SCHEMA §7.3 `mode` string, with a payload
    // whose own `mode` is STALE ("Fillet" on a Chamfer op) — the exact shape a
    // caller that forgot to patch it would send. Core is the single writer, so
    // the contradiction must not survive.
    let stamp_mode = |op: &mut Operation, v: &str| match op {
        Operation::Known(KnownOperation::Fillet(p)) => {
            p.extra.insert("mode".into(), v.into());
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            p.extra.insert("mode".into(), v.into());
        }
        _ => unreachable!(),
    };
    let prior = {
        let mut op = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
        stamp_mode(&mut op, "Fillet");
        op
    };
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: prior,
    })
    .expect("seed the legacy `mode` string");

    let mut swap = chamfer_op(&["e1"], vec![edge_ref(BX(), "e1")], 3.5);
    stamp_mode(&mut swap, "Fillet"); // stale on purpose
    let out = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: swap,
        })
        .expect("the sanctioned Fillet→Chamfer swap is accepted");

    // The regen contract is IDENTICAL to an in-type param update: dirty from this
    // step to the end, `ToEnd` (a swap is not a special regen shape).
    assert_eq!(out.dirty, Some(DirtyRange::new(0, 1)), "dirty span");
    assert_eq!(out.regen, RegenHint::ToEnd, "regen hint");

    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "Chamfer", "the record IS a chamfer now");
    assert_eq!(
        closure_version(&rec.op),
        None,
        "legacy absence stays absent"
    );
    let (ids, typed, radius, mode) = edge_op_facts(&rec.op);
    assert_eq!(ids, vec!["e1".to_string()], "edgeIds preserved");
    assert_eq!(typed, 1, "typed edge refs preserved");
    assert_eq!(radius, 3.5, "the new size landed");
    assert_eq!(
        mode.as_deref(),
        Some("Chamfer"),
        "the legacy `mode` string is NORMALIZED to the new opType, never left stale"
    );
    // Inputs are re-derived from the swapped op (the body the edge belongs to).
    assert_eq!(
        rec.inputs,
        rec.op.derive_inputs(),
        "inputs re-derived from the new op"
    );

    // Undo restores the WHOLE prior record — opType, size, and `mode` alike.
    assert!(sess.undo().is_some(), "the swap is undoable");
    let back = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(back.op.op_type(), "Fillet", "undo restores the opType");
    let (_, _, r0, m0) = edge_op_facts(&back.op);
    assert_eq!(r0, 2.0, "undo restores the radius");
    assert_eq!(m0.as_deref(), Some("Fillet"), "undo restores `mode`");

    // …and redo re-runs the same symmetric guard.
    assert!(
        sess.redo().expect("redo applies").is_some(),
        "redo available"
    );
    let again = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(again.op.op_type(), "Chamfer", "redo re-applies the swap");
    assert_eq!(edge_op_facts(&again.op).2, 3.5, "redo restores the size");
}

#[test]
fn prepared_closure_version_survives_fillet_chamfer_swap() {
    let mut sess = fillet_doc("e1");
    let mut prepared_fillet = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
    set_closure_version(&mut prepared_fillet, Some(1));
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: prepared_fillet,
    })
    .expect("seed prepared closure");

    let mut prepared_chamfer = chamfer_op(&["e1"], vec![edge_ref(BX(), "e1")], 3.0);
    set_closure_version(&mut prepared_chamfer, Some(1));
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: prepared_chamfer,
    })
    .expect("prepared Fillet to Chamfer swap");

    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "Chamfer");
    assert_eq!(closure_version(&rec.op), Some(1));
    assert_eq!(edge_op_facts(&rec.op).0, vec!["e1".to_string()]);
}

#[test]
fn edge_closure_version_is_optional_and_only_version_one_is_writable() {
    let legacy = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
    let legacy_bytes = serde_json::to_vec(&legacy).expect("serialize legacy fillet");
    let legacy_json: serde_json::Value =
        serde_json::from_slice(&legacy_bytes).expect("parse legacy bytes");
    assert!(
        legacy_json["params"].get("tangentClosureVersion").is_none(),
        "legacy records must remain byte-shape stable"
    );

    let reopened: Operation =
        serde_json::from_slice(&legacy_bytes).expect("deserialize literal legacy record");
    let mut legacy_session = fillet_doc("e1");
    legacy_session
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: reopened,
        })
        .expect("legacy no-op re-edit");
    let saved = serde_json::to_vec(
        &legacy_session
            .document()
            .timeline
            .record_by_id(rid(1))
            .unwrap()
            .op,
    )
    .expect("save re-edited legacy record");
    assert_eq!(
        saved, legacy_bytes,
        "legacy re-edit/save must be byte-stable"
    );

    let mut sess = fillet_doc("e1");
    let mut missing_refs = fillet_op(&["e1"], Vec::new());
    set_closure_version(&mut missing_refs, Some(1));
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: missing_refs,
        })
        .expect_err("versioned closure without typed refs must be rejected");
    assert!(err.to_string().contains("full typed edge refs"));

    let mut intent_only = fillet_op(
        &["e1"],
        vec![ElementRef {
            primary: None,
            intent: None,
            anchor: None,
            extra: Default::default(),
        }],
    );
    set_closure_version(&mut intent_only, Some(1));
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: intent_only,
        })
        .expect_err("versioned closure requires an EDGE primary");
    assert!(err.to_string().contains("EDGE primary"));

    let mut invalid = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
    set_closure_version(&mut invalid, Some(2));
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: invalid,
        })
        .expect_err("unknown closure policy must be rejected");
    assert!(err.to_string().contains("tangentClosureVersion"));
    assert_eq!(
        closure_version(&sess.document().timeline.record_by_id(rid(1)).unwrap().op),
        None
    );
}

#[test]
fn update_operation_params_normalizes_mode_carried_only_by_the_prior_record() {
    let mut sess = fillet_doc("e1");
    let seeded = {
        let mut op = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
        if let Operation::Known(KnownOperation::Fillet(p)) = &mut op {
            p.extra.insert("mode".into(), "Fillet".into());
        }
        op
    };
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: seeded,
    })
    .expect("seed `mode`");

    // The incoming payload carries NO `mode` — but the record did, so the swap
    // must not silently drop it into a stale-on-reload state either.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op(&["e1"], vec![edge_ref(BX(), "e1")], 2.0),
    })
    .expect("swap accepted");
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(edge_op_facts(&rec.op).3.as_deref(), Some("Chamfer"));

    // A document that never carried `mode` never grows one.
    let mut clean = fillet_doc("e2");
    clean
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: chamfer_op(&["e2"], vec![edge_ref(BX(), "e2")], 2.0),
        })
        .expect("swap accepted");
    let rec = clean.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(
        edge_op_facts(&rec.op).3,
        None,
        "no `mode` key is invented on a record that never had one"
    );
}

#[test]
fn update_operation_params_still_refuses_other_op_type_changes() {
    // Fillet → Extrude.
    let mut sess = fillet_doc("e1");
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: extrude_newbody(rid(9), 5.0, BY()).op,
        })
        .unwrap_err();
    assert!(err.to_string().contains("opType"), "Fillet→Extrude: {err}");

    // Extrude → Revolve.
    let mut doc = Document::new(DocumentId(u(0x5E)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![extrude_newbody(rid(1), 5.0, BX())]);
    let mut sess = DocumentSession::new(doc);
    let revolve = Operation::Known(KnownOperation::Revolve(RevolveParams {
        profile: Some(profile()),
        angle_deg: s(90.0),
        axis: None,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        extra: Default::default(),
    }));
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: revolve,
        })
        .unwrap_err();
    assert!(err.to_string().contains("opType"), "Extrude→Revolve: {err}");

    // Shell → Fillet. Shell folds into the SAME frontend feature bucket as the
    // edge ops (`dto.rs feature_kind`), so it is the nearest miss to the
    // sanctioned pair and must still be refused.
    let mut doc = Document::new(DocumentId(u(0x5F)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    let shell = Operation::Known(KnownOperation::Shell(ShellParams {
        thickness: s(1.0),
        open_faces: vec![ElementId::new("f1")],
        faces: vec![],
        target_body: Some(BX()),
        extra: Default::default(),
    }));
    doc.timeline = Timeline::from_records(vec![record(rid(1), "Shell", shell, vec![BX()])]);
    let mut sess = DocumentSession::new(doc);
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
        })
        .unwrap_err();
    assert!(err.to_string().contains("opType"), "Shell→Fillet: {err}");
}

#[test]
fn fillet_chamfer_swap_still_enforces_edge_lockstep() {
    let mut sess = fillet_doc("e1");
    // Widening the opType guard must NOT widen the F2 lockstep guard: a swap
    // carrying 2 ids and 1 typed edge is still rejected.
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: chamfer_op(&["e1", "e2"], vec![edge_ref(BX(), "e1")], 2.0),
        })
        .unwrap_err();
    assert!(err.to_string().contains("mismatch"), "swap lockstep: {err}");

    // …and a typed ref pointing at a different element than its parallel id.
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: chamfer_op(&["e1"], vec![edge_ref(BX(), "e7")], 2.0),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("!="),
        "swap element mismatch: {err}"
    );
    // The record is untouched by either rejection.
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "Fillet", "a rejected swap writes nothing");
}

// ── Component Library WP-1.2: PlaceComponent → DetachComponent swap ─────────

fn place_component_op() -> Operation {
    Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "onecad.std.iso4762".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: format!("sha256:{}", "0".repeat(64)),
        params: Default::default(),
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: Default::default(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [s(0.0), s(0.0), s(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }))
}

fn detach_component_op() -> Operation {
    Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: Default::default(),
            extra: Default::default(),
        },
        placement: FrozenPlacement {
            translate: [s(0.0), s(0.0), s(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }))
}

fn place_component_doc() -> DocumentSession {
    let mut doc = Document::new(DocumentId(u(0x5E)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Place Component",
        place_component_op(),
        vec![BX()],
    )]);
    DocumentSession::new(doc)
}

#[test]
fn update_operation_params_swaps_place_component_to_detach_component() {
    let mut sess = place_component_doc();
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: detach_component_op(),
    })
    .expect("the sanctioned PlaceComponent→DetachComponent swap is accepted");

    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "DetachComponent");
    assert_eq!(rec.record_id, rid(1), "RecordId preserved across the swap");
}

#[test]
fn detach_component_to_place_component_is_not_a_sanctioned_reverse_swap() {
    let mut sess = place_component_doc();
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: detach_component_op(),
    })
    .expect("forward swap");

    // The "honest break link" is one-directional — re-attaching a library
    // identity to an already-detached body is not a sanctioned edit.
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: place_component_op(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("opType"), "reverse swap: {err}");
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(
        rec.op.op_type(),
        "DetachComponent",
        "a rejected swap writes nothing"
    );
}

#[test]
fn place_component_cannot_swap_to_an_unrelated_op_type() {
    let mut sess = place_component_doc();
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("opType"),
        "PlaceComponent→Fillet: {err}"
    );
}

// ── (WP-C T2a) two-distance chamfer: `distance2` ─────────────────────────────

/// A Chamfer op whose SECOND leg is set (SCHEMA §7.3, 2026-08-03).
fn chamfer_op_d2(edge_ids: &[&str], edges: Vec<ElementRef>, radius: f64, d2: f64) -> Operation {
    let Operation::Known(KnownOperation::Chamfer(mut p)) = chamfer_op(edge_ids, edges, radius)
    else {
        unreachable!("chamfer_op builds a Chamfer")
    };
    p.distance2 = Some(s(d2));
    Operation::Known(KnownOperation::Chamfer(p))
}

/// The stored `distance2` of a record's op (`None` for an equal-leg chamfer).
fn stored_d2(sess: &DocumentSession, r: RecordId) -> Option<f64> {
    match &sess.document().timeline.record_by_id(r).unwrap().op {
        Operation::Known(KnownOperation::Chamfer(p)) => p.distance2.as_ref().map(|s| s.value),
        _ => None,
    }
}

/// `distance2` is `skip_serializing_if = "Option::is_none"`: an equal-leg chamfer
/// is byte-identical to every document authored before the field existed, and the
/// asymmetric form renders the key in the SCHEMA §7.3 position (right after
/// `radius`) so a reader diffing the two forms sees ONE inserted key.
#[test]
fn chamfer_distance2_is_skip_none_and_round_trips() {
    let equal = chamfer_op(&["e:14"], vec![], 1.0);
    let json = serde_json::to_string(&equal).expect("serialize");
    assert!(
        !json.contains("distance2"),
        "an equal-leg chamfer must not grow a `distance2` key: {json}"
    );
    assert_eq!(
        serde_json::from_str::<Operation>(&json).unwrap(),
        equal,
        "equal-leg round-trips"
    );

    let asym = chamfer_op_d2(&["e:14"], vec![], 1.0, 2.5);
    let json = serde_json::to_string(&asym).expect("serialize");
    assert!(
        json.contains(r#""radius":{"value":1.0},"distance2":{"value":2.5}"#),
        "SCHEMA §7.3 field order (radius then distance2): {json}"
    );
    assert_eq!(
        serde_json::from_str::<Operation>(&json).unwrap(),
        asym,
        "the two-distance form round-trips"
    );

    // A SCHEMA §7.3 payload with a bare-number `distance2` (the hand-authored
    // form `Scalar` documents) parses to the same op.
    let wire = r#"{ "opType": "Chamfer", "params": {
        "mode": "Chamfer", "radius": 1.0, "distance2": 2.5,
        "edgeIds": ["e:14"], "chainTangentEdges": true } }"#;
    match serde_json::from_str::<Operation>(wire).unwrap() {
        Operation::Known(KnownOperation::Chamfer(p)) => {
            assert_eq!(p.distance2.expect("distance2 parsed").value, 2.5);
        }
        other => panic!("expected Chamfer, got {other:?}"),
    }
}

#[test]
fn chamfer_rejects_a_non_positive_distance2_on_every_entry_path() {
    for bad in [0.0, -1.0] {
        // AddOperation.
        let mut doc = Document::new(DocumentId(u(0x5A)));
        doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
        let mut sess = DocumentSession::new(doc);
        let err = sess
            .apply(EditCommand::AddOperation {
                record: record(
                    rid(1),
                    "Chamfer",
                    chamfer_op_d2(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, bad),
                    vec![BX()],
                ),
                at_cursor: false,
            })
            .unwrap_err();
        assert!(err.to_string().contains("distance2"), "add {bad}: {err}");

        // UpdateOperationParams.
        let mut sess = fillet_doc("e1");
        let err = sess
            .apply(EditCommand::UpdateOperationParams {
                record: rid(1),
                op: chamfer_op_d2(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, bad),
            })
            .unwrap_err();
        assert!(err.to_string().contains("distance2"), "update {bad}: {err}");
        let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
        assert_eq!(rec.op.op_type(), "Fillet", "a rejected edit writes nothing");
    }
}

/// `FilletParams` has no typed `distance2`, so an unguarded payload naming one
/// would land in the `extra` flatten and round-trip VERBATIM — a second leg
/// persisted on an op that never reads it, and resurrected by a later flip.
#[test]
fn a_fillet_may_not_carry_distance2() {
    let mut fillet = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
    if let Operation::Known(KnownOperation::Fillet(p)) = &mut fillet {
        p.extra.insert("distance2".into(), serde_json::json!(2.5));
    }
    let mut sess = fillet_doc("e1");
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("distance2") && err.to_string().contains("Chamfer-only"),
        "Fillet carrying distance2: {err}"
    );
}

/// The FILLET-CHAMFER-UNIFY W3 swap has a FIELD-IDENTITY precondition. A Chamfer
/// carrying `distance2` breaks it — flipping it to Fillet would silently drop the
/// user's second leg — so the edit is refused with the standard allow-list reason,
/// NAMING the field, until it is cleared.
#[test]
fn a_two_distance_chamfer_may_not_flip_to_fillet_until_distance2_is_cleared() {
    let mut sess = fillet_doc("e1");
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op_d2(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, 2.5),
    })
    .expect("Fillet → two-distance Chamfer invents nothing the target cannot hold");
    assert_eq!(stored_d2(&sess, rid(1)), Some(2.5));

    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("opType"), "standard allow-list reason: {msg}");
    assert!(
        msg.contains("distance2"),
        "the reason NAMES the field: {msg}"
    );
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(
        rec.op.op_type(),
        "Chamfer",
        "a rejected flip writes nothing"
    );
    assert_eq!(
        stored_d2(&sess, rid(1)),
        Some(2.5),
        "…and keeps the 2nd leg"
    );

    // Clearing `distance2` is an ordinary (visible, undoable) params edit; the
    // flip is then the plain W3 swap again.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op(&["e1"], vec![edge_ref(BX(), "e1")], 1.0),
    })
    .expect("clearing distance2 is a plain params edit");
    assert_eq!(stored_d2(&sess, rid(1)), None);
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
    })
    .expect("an equal-leg Chamfer still flips to Fillet");
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "Fillet");

    // …and undo walks the whole sequence back, second leg included.
    assert!(sess.undo().is_some(), "flip undone");
    assert!(sess.undo().is_some(), "clear undone");
    assert_eq!(
        stored_d2(&sess, rid(1)),
        Some(2.5),
        "undo restores the cleared second leg"
    );
}

// ── distance-angle chamfer: `angleDeg` ───────────────────────────────────────

/// A Chamfer op in the distance-angle mode (SCHEMA §7.3): `radius` on the
/// reference face, `angleDeg` off it, in DEGREES.
fn chamfer_op_angle(
    edge_ids: &[&str],
    edges: Vec<ElementRef>,
    radius: f64,
    angle: f64,
) -> Operation {
    let Operation::Known(KnownOperation::Chamfer(mut p)) = chamfer_op(edge_ids, edges, radius)
    else {
        unreachable!("chamfer_op builds a Chamfer")
    };
    p.angle_deg = Some(s(angle));
    Operation::Known(KnownOperation::Chamfer(p))
}

/// The stored `angleDeg` of a record's op (`None` unless it is distance-angle).
fn stored_angle(sess: &DocumentSession, r: RecordId) -> Option<f64> {
    match &sess.document().timeline.record_by_id(r).unwrap().op {
        Operation::Known(KnownOperation::Chamfer(p)) => p.angle_deg.as_ref().map(|s| s.value),
        _ => None,
    }
}

/// The bound is EXCLUSIVE at both ends and enforced on every entry path — a
/// rejected edit writes nothing. Geometric feasibility against the local dihedral
/// stays the worker's recoverable failure, not a rejected edit.
#[test]
fn chamfer_rejects_an_out_of_range_angle_deg_on_every_entry_path() {
    for bad in [0.0, 180.0, -30.0, 200.0] {
        // AddOperation.
        let mut doc = Document::new(DocumentId(u(0x5A)));
        doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
        let mut sess = DocumentSession::new(doc);
        let err = sess
            .apply(EditCommand::AddOperation {
                record: record(
                    rid(1),
                    "Chamfer",
                    chamfer_op_angle(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, bad),
                    vec![BX()],
                ),
                at_cursor: false,
            })
            .unwrap_err();
        assert!(err.to_string().contains("angleDeg"), "add {bad}: {err}");

        // UpdateOperationParams.
        let mut sess = fillet_doc("e1");
        let err = sess
            .apply(EditCommand::UpdateOperationParams {
                record: rid(1),
                op: chamfer_op_angle(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, bad),
            })
            .unwrap_err();
        assert!(err.to_string().contains("angleDeg"), "update {bad}: {err}");
        let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
        assert_eq!(rec.op.op_type(), "Fillet", "a rejected edit writes nothing");
    }

    // 45° on a 90° dihedral is the equal-leg chamfer — squarely in range.
    let mut sess = fillet_doc("e1");
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op_angle(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, 45.0),
    })
    .expect("45° is a valid distance-angle chamfer");
    assert_eq!(stored_angle(&sess, rid(1)), Some(45.0));
}

/// The two second-leg spellings name the same thing twice. Refusing BY NAME (both
/// fields) is the point: a precedence rule would silently discard one of the two
/// numbers the user typed.
#[test]
fn a_chamfer_may_not_carry_distance2_and_angle_deg_together() {
    let both = {
        let Operation::Known(KnownOperation::Chamfer(mut p)) =
            chamfer_op_d2(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, 2.5)
        else {
            unreachable!()
        };
        p.angle_deg = Some(s(30.0));
        Operation::Known(KnownOperation::Chamfer(p))
    };

    let mut doc = Document::new(DocumentId(u(0x5B)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    let mut sess = DocumentSession::new(doc);
    let err = sess
        .apply(EditCommand::AddOperation {
            record: record(rid(1), "Chamfer", both.clone(), vec![BX()]),
            at_cursor: false,
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("distance2") && msg.contains("angleDeg"),
        "add names BOTH fields: {msg}"
    );

    let mut sess = fillet_doc("e1");
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: both,
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("distance2") && msg.contains("angleDeg"),
        "update names BOTH fields: {msg}"
    );
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(rec.op.op_type(), "Fillet", "a rejected edit writes nothing");
}

/// `FilletParams` has no typed `angleDeg`, so an unguarded payload naming one would
/// land in the `extra` flatten and round-trip VERBATIM — an angle persisted on an op
/// that never reads it, and resurrected by a later flip.
#[test]
fn a_fillet_may_not_carry_angle_deg() {
    let mut fillet = fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]);
    if let Operation::Known(KnownOperation::Fillet(p)) = &mut fillet {
        p.extra.insert("angleDeg".into(), serde_json::json!(30.0));
    }
    let mut sess = fillet_doc("e1");
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("angleDeg") && err.to_string().contains("Chamfer-only"),
        "Fillet carrying angleDeg: {err}"
    );
}

/// Same FIELD-IDENTITY precondition `distance2` carries: a Chamfer with `angleDeg`
/// set is not flippable to Fillet — the flip would silently drop the angle — so the
/// edit is refused with the standard allow-list reason, NAMING the field.
#[test]
fn a_distance_angle_chamfer_may_not_flip_to_fillet_until_angle_deg_is_cleared() {
    let mut sess = fillet_doc("e1");
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op_angle(&["e1"], vec![edge_ref(BX(), "e1")], 1.0, 30.0),
    })
    .expect("Fillet → distance-angle Chamfer invents nothing the target cannot hold");
    assert_eq!(stored_angle(&sess, rid(1)), Some(30.0));

    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("opType"), "standard allow-list reason: {msg}");
    assert!(
        msg.contains("angleDeg"),
        "the reason NAMES the field: {msg}"
    );
    let rec = sess.document().timeline.record_by_id(rid(1)).unwrap();
    assert_eq!(
        rec.op.op_type(),
        "Chamfer",
        "a rejected flip writes nothing"
    );
    assert_eq!(
        stored_angle(&sess, rid(1)),
        Some(30.0),
        "…and keeps the angle"
    );

    // Clearing `angleDeg` is an ordinary (visible, undoable) params edit; the flip
    // is then the plain W3 swap again.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: chamfer_op(&["e1"], vec![edge_ref(BX(), "e1")], 1.0),
    })
    .expect("clearing angleDeg is a plain params edit");
    assert_eq!(stored_angle(&sess, rid(1)), None);
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: fillet_op(&["e1"], vec![edge_ref(BX(), "e1")]),
    })
    .expect("an equal-leg Chamfer still flips to Fillet");
    assert_eq!(
        sess.document()
            .timeline
            .record_by_id(rid(1))
            .unwrap()
            .op
            .op_type(),
        "Fillet"
    );

    // …and undo walks the whole sequence back, angle included.
    assert!(sess.undo().is_some(), "flip undone");
    assert!(sess.undo().is_some(), "clear undone");
    assert_eq!(
        stored_angle(&sess, rid(1)),
        Some(30.0),
        "undo restores the cleared angle"
    );
}

// ── (b) proptest: random valid sequences ─────────────────────────────────────

/// Builds a valid command from an action selector + a unique index.
fn action_command(kind: u8, i: usize) -> EditCommand {
    let n = i as u128;
    match kind % 5 {
        0 => EditCommand::AddVariable {
            variable: Variable {
                id: vid(0x100 + n),
                name: format!("v{i}"),
                value: s(i as f64),
                unit: Unit::Mm,
            },
        },
        1 => EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(sid(0x100 + n), format!("S{i}"), WorldPlane::XY),
        },
        2 => EditCommand::AddOperation {
            record: extrude_newbody(rid(0x100 + n), i as f64, bid(0x100 + n)),
            at_cursor: true,
        },
        3 => EditCommand::RenameBody {
            body: B0(),
            name: format!("b{i}"),
        },
        _ => EditCommand::SetVisibility {
            target: VisibilityTarget::Body(B0()),
            visible: i.is_multiple_of(2),
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Apply a random valid sequence, then undo everything: the document JSON
    /// returns exactly to its initial value, and the stack never exceeds 200.
    #[test]
    fn apply_all_then_undo_all_restores_initial(actions in prop::collection::vec(0u8..5, 0..80)) {
        let base = base_document();
        let initial = json(&base);
        let mut sess = DocumentSession::new(base);

        for (i, &k) in actions.iter().enumerate() {
            sess.apply(action_command(k, i)).unwrap();
            prop_assert!(sess.undo_depth() <= 200, "stack cap holds");
        }
        while sess.can_undo() {
            prop_assert!(sess.undo().is_some());
        }
        prop_assert_eq!(json(sess.document()), initial);
    }
}

#[test]
fn undo_stack_caps_at_200() {
    let base = base_document();
    let mut sess = DocumentSession::new(base);
    for i in 0..250 {
        sess.apply(action_command(0, i)).unwrap(); // AddVariable, unique
    }
    assert_eq!(sess.undo_depth(), 200, "capped at 200, oldest evicted");
}

// ── (g) session snapshot ─────────────────────────────────────────────────────

#[test]
fn session_snapshot_after_scripted_sequence() {
    let mut sess = DocumentSession::new(base_document());
    sess.apply(EditCommand::AddVariable {
        variable: Variable {
            id: vid(2),
            name: "height".into(),
            value: s(20.0),
            unit: Unit::Mm,
        },
    })
    .unwrap();
    sess.apply(EditCommand::AddOperation {
        record: extrude_newbody(rid(0x30), 12.5, bid(0x30)),
        at_cursor: true,
    })
    .unwrap();
    sess.apply(EditCommand::RenameBody {
        body: B0(),
        name: "Base Plate".into(),
    })
    .unwrap();
    sess.apply(EditCommand::SetRollback { cursor: 3 }).unwrap();
    sess.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(did(2), "Datum 2", "XZ", 5.0),
    })
    .unwrap();

    insta::assert_json_snapshot!("session_after_sequence", sess.document());
}

// ── Frontend wire pin: the TS `buildAddSketch` JSON must deserialize ─────────
//
// The frontend creates new sketches (plane-picker flow) by POSTing exactly this
// JSON shape through `apply_edit_command`. `SketchData.plane` has NO serde
// default — a payload without it fails before the command handler runs (that
// gap shipped once; this pin keeps the TS shape and the Rust serde in lockstep).
// Mirror of src/ipc/sketchWireMap.ts `buildAddSketch` — change both together.

#[test]
fn frontend_build_add_sketch_wire_deserializes_and_applies() {
    let wire = r#"{
        "cmd": "addSketch",
        "sketch": {
            "id": "8f9e0d1c-2b3a-4c5d-8e7f-0a1b2c3d4e5f",
            "name": "Sketch 1",
            "plane": {
                "origin": [0, 0, 0],
                "xAxis": [0, 1, 0],
                "yAxis": [0, 0, 1],
                "normal": [1, 0, 0]
            },
            "attachment": { "kind": "world", "plane": "XZ" }
        }
    }"#;

    let cmd: EditCommand = serde_json::from_str(wire).expect("frontend AddSketch wire must parse");
    let EditCommand::AddSketch { ref sketch } = cmd else {
        panic!("expected AddSketch, got {cmd:?}");
    };
    assert_eq!(sketch.plane, onecad_core::sketch::SketchPlane::xz());

    let mut sess = DocumentSession::new(base_document());
    sess.apply(cmd).unwrap();
    let sid: SketchId = "8f9e0d1c-2b3a-4c5d-8e7f-0a1b2c3d4e5f".parse().unwrap();
    assert!(
        sess.document().sketches.contains_key(&sid),
        "sketch must land in the document"
    );

    // The plane field is REQUIRED: the same payload without it must NOT parse.
    let no_plane = wire.replace(
        r#""plane": {
                "origin": [0, 0, 0],
                "xAxis": [0, 1, 0],
                "yAxis": [0, 0, 1],
                "normal": [1, 0, 0]
            },
            "#,
        "",
    );
    assert!(
        no_plane.len() < wire.len(),
        "replace must have removed the plane field"
    );
    assert!(
        serde_json::from_str::<EditCommand>(&no_plane).is_err(),
        "AddSketch without plane must fail (frontend MUST send it)"
    );
}

// ── HISTORY-HARDEN H9: sketch REATTACH ───────────────────────────────────────

/// Re-attaching a sketch must move the TIMELINE RECORD's plane, not just the
/// document sketch's.
///
/// The regen planner lowers the RECORD's `SketchOpParams` — the record is
/// otherwise refreshed only when a sketch session finishes
/// (`document_runtime::upsert_sketch_record`). Before H9,
/// `UpdateSketchAttachment` restamped only `host_face`, so a reattach updated the
/// tree and the drawn sketch while every regen kept replaying the sketch on its
/// OLD plane: the feature moved on screen and no downstream extrude followed it.
#[test]
fn reattaching_a_sketch_moves_the_timeline_record_plane() {
    let mut doc = base_document();
    // The base document's sketch S1 is world-XY with a producing Sketch record.
    let record_index = doc
        .timeline
        .records()
        .iter()
        .position(
            |r| matches!(&r.op, Operation::Known(KnownOperation::Sketch(p)) if p.sketch == S1()),
        )
        .expect("base document has a Sketch record for S1");
    let plane_kind_of = |doc: &Document| {
        let Operation::Known(KnownOperation::Sketch(p)) =
            &doc.timeline.record(record_index).unwrap().op
        else {
            panic!("Sketch record")
        };
        (p.plane.kind, p.plane.normal)
    };
    assert_eq!(plane_kind_of(&doc).0, PlaneKind::Xy, "starts on world XY");
    doc.sketches
        .insert(S1(), Sketch::on_world_plane(S1(), "S1", WorldPlane::XY));
    let mut sess = DocumentSession::new(doc);

    // Re-attach to world XZ — a NAMED plane, so the record keeps a named kind.
    let outcome = sess
        .apply(EditCommand::UpdateSketchAttachment {
            sketch: S1(),
            plane: onecad_core::sketch::SketchPlane::xz(),
            attachment: SketchAttachment::World {
                plane: WorldPlane::XZ,
            },
        })
        .expect("reattach to world XZ");
    let (kind, normal) = plane_kind_of(sess.document());
    assert_eq!(kind, PlaneKind::Xz, "the RECORD's plane kind follows");
    assert_eq!(
        normal,
        onecad_core::sketch::SketchPlane::xz().normal,
        "…and so does its basis — this is what the planner lowers"
    );
    // The reattach dirties from the producing Sketch step, so downstream regens.
    assert_eq!(outcome.regen, RegenHint::ToEnd);

    // Undo restores BOTH the sketch and the record's plane.
    assert!(sess.undo().is_some());
    assert_eq!(plane_kind_of(sess.document()).0, PlaneKind::Xy);
}

/// Re-attaching to a DATUM takes the datum's backend-resolved frame, never the
/// caller's — the same rule `add_sketch` enforces via `stamp_datum_plane`. A
/// frontend that sent a stale/identity basis must not be able to plant it.
#[test]
fn reattaching_to_a_datum_stamps_the_resolved_frame_on_the_record() {
    let mut doc = base_document();
    doc.sketches
        .insert(S1(), Sketch::on_world_plane(S1(), "S1", WorldPlane::XY));
    let mut sess = DocumentSession::new(doc);
    sess.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(did(7), "Datum 1", "XZ", 10.0),
    })
    .expect("add the datum");
    let resolved = sess.document().datum_planes.get(&did(7)).unwrap().clone();
    assert!(resolved.resolved_valid);

    // Deliberately send a WRONG basis with the command.
    sess.apply(EditCommand::UpdateSketchAttachment {
        sketch: S1(),
        plane: onecad_core::sketch::SketchPlane::xy(),
        attachment: SketchAttachment::Datum { datum: did(7) },
    })
    .expect("reattach to the datum");

    assert_eq!(
        sess.document().sketches[&S1()].plane,
        resolved.resolved_plane,
        "the core overwrites the caller's basis with the datum's resolved frame"
    );
    let Operation::Known(KnownOperation::Sketch(p)) = &sess
        .document()
        .timeline
        .records()
        .iter()
        .find(|r| matches!(&r.op, Operation::Known(KnownOperation::Sketch(q)) if q.sketch == S1()))
        .expect("Sketch record")
        .op
    else {
        panic!("Sketch record")
    };
    assert_eq!(p.plane.kind, PlaneKind::Custom, "a datum frame is custom");
    assert_eq!(p.plane.origin, resolved.resolved_plane.origin);
    assert_eq!(p.plane.normal, resolved.resolved_plane.normal);

    // An UNRESOLVED / unknown datum is refused rather than silently leaving the
    // sketch on a placeholder frame.
    let err = sess
        .apply(EditCommand::UpdateSketchAttachment {
            sketch: S1(),
            plane: onecad_core::sketch::SketchPlane::xy(),
            attachment: SketchAttachment::Datum { datum: did(99) },
        })
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

// ── Frontend wire pin #2: `sketch_upsert` ops (TS `marshalUpsert` output) ────
//
// Same contract class as the AddSketch pin above: the TS mapper authors these
// internally-tagged `op`/`kind` payloads and Tauri deserializes them STRAIGHT
// into `Vec<SketchEditOp>` (api::sketch_upsert). Mirror of the op/entity/
// constraint unions in src/ipc/sketchWireMap.ts — change both together.

#[test]
fn frontend_sketch_upsert_ops_wire_deserializes() {
    let uid = |n: u8| format!("00000000-0000-4000-8000-0000000000{n:02x}");
    let wire = serde_json::json!([
        { "op": "addEntity", "entity": { "kind": "point", "id": uid(1), "at": [0.0, 0.0] } },
        { "op": "addEntity", "entity": { "kind": "point", "id": uid(2), "at": [60.0, 0.0], "construction": false } },
        { "op": "addEntity", "entity": { "kind": "line", "id": uid(3), "start": uid(1), "end": uid(2) } },
        { "op": "addEntity", "entity": { "kind": "circle", "id": uid(4), "center": uid(1), "radius": 12.5 } },
        { "op": "addEntity", "entity": { "kind": "arc", "id": uid(5), "center": uid(2), "radius": 8.0,
                                          "startAngle": 0.0, "endAngle": 1.25, "construction": true } },
        { "op": "addConstraint", "constraint": { "kind": "coincident", "id": uid(6), "point1": uid(1), "point2": uid(2) } },
        { "op": "addConstraint", "constraint": { "kind": "horizontal", "id": uid(7), "line": uid(3) } },
        { "op": "addConstraint", "constraint": { "kind": "distance", "id": uid(8), "entity1": uid(1), "entity2": uid(2),
                                                  "value": { "value": 60.0 } } },
        { "op": "addConstraint", "constraint": { "kind": "radius", "id": uid(9), "entity": uid(4),
                                                  "value": { "value": 12.5 } } },
        { "op": "setDimension", "constraint": uid(8), "value": { "value": 42.0 } },
        { "op": "setEntityPositions", "positions": [[uid(1), [1.0, 2.0]], [uid(2), [3.0, 4.0]]] },
        { "op": "removeConstraint", "constraint": uid(7) },
        { "op": "removeEntity", "entity": uid(5) }
    ]);

    let ops: Vec<SketchEditOp> =
        serde_json::from_value(wire).expect("frontend sketch_upsert ops must parse");
    assert_eq!(ops.len(), 13);
    assert!(matches!(ops[0], SketchEditOp::AddEntity { .. }));
    assert!(matches!(ops[9], SketchEditOp::SetDimension { .. }));
    assert!(matches!(ops[10], SketchEditOp::SetEntityPositions { .. }));
    assert!(matches!(ops[12], SketchEditOp::RemoveEntity { .. }));
}

// ─────────────────────────────────────────────────────────────────────────────
// Datum planes (DATUM W1) — resolution is a CREATION-TIME, core-owned act.
//
// The FE authors only the parametric definition; `add_datum` resolves it and
// overwrites the frame, `add_sketch` stamps a datum-hosted sketch from that
// frame, and `DeleteDatum` refuses while a sketch still points at it.
// ─────────────────────────────────────────────────────────────────────────────

/// A session over a blank document (no seeded datum) — the datum tests below
/// author everything they need through commands, like the frontend does.
fn blank_session() -> DocumentSession {
    DocumentSession::new(Document::new(DocumentId(u(0xDA))))
}

fn add_offset_datum(
    sess: &mut DocumentSession,
    id: DatumPlaneId,
    base: &str,
    offset: f64,
) -> Result<(), String> {
    sess.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(id, "Datum", base, offset),
    })
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[test]
fn add_datum_resolves_at_creation_and_overwrites_the_client_frame() {
    let mut sess = blank_session();
    // A client that sent a bogus resolved frame + `resolvedValid: true` must not
    // be able to dictate the basis — Rust re-derives it, unconditionally.
    let mut d = DatumPlane::offset_from_plane(did(5), "Datum 1", "XY", 10.0);
    d.resolved_plane = onecad_core::sketch::SketchPlane {
        origin: Vec3::new_unchecked(999.0, 999.0, 999.0),
        ..onecad_core::sketch::SketchPlane::yz()
    };
    d.resolved_valid = true;
    sess.apply(EditCommand::AddDatumPlane { datum: d }).unwrap();

    let stored = sess.document().datum(did(5)).expect("datum stored");
    assert!(stored.resolved_valid);
    // XY's normal is +Z ⇒ a 10mm offset lands at z = 10, with XY's axes verbatim.
    assert_eq!(
        stored.resolved_plane.origin,
        Vec3::new_unchecked(0.0, 0.0, 10.0)
    );
    assert_eq!(
        stored.resolved_plane.x_axis,
        onecad_core::sketch::SketchPlane::xy().x_axis
    );
    assert_eq!(
        stored.resolved_plane.normal,
        onecad_core::sketch::SketchPlane::xy().normal
    );
}

/// LEGACY-SWAPPED BASES PIN (`sketch/plane.rs`): the repo's "XZ" plane has world
/// normal **+X**, so "XZ offset 10" slides along world +X — NOT along −Y as the
/// name suggests. Likewise "YZ" has normal +Y. If this ever fails, someone
/// "fixed" the non-standard bases and every datum in every stored file moved.
#[test]
fn offset_from_xz_slides_along_world_x() {
    let mut sess = blank_session();
    add_offset_datum(&mut sess, did(6), "XZ", 10.0).unwrap();
    assert_eq!(
        sess.document().datum(did(6)).unwrap().resolved_plane.origin,
        Vec3::new_unchecked(10.0, 0.0, 0.0),
        "XZ's normal is +X (Sketch.h), so the offset moves along world +X"
    );

    add_offset_datum(&mut sess, did(7), "YZ", -4.0).unwrap();
    assert_eq!(
        sess.document().datum(did(7)).unwrap().resolved_plane.origin,
        Vec3::new_unchecked(0.0, -4.0, 0.0),
        "YZ's normal is +Y (Sketch.h)"
    );
}

#[test]
fn a_datum_chains_off_another_resolved_datum() {
    let mut sess = blank_session();
    add_offset_datum(&mut sess, did(8), "XY", 10.0).unwrap();
    // Base = the FIRST datum's id (a heterogeneous `basePlaneId`, per the C++ model).
    add_offset_datum(&mut sess, did(9), &did(8).to_string(), 5.0).unwrap();

    let chained = sess.document().datum(did(9)).unwrap();
    assert!(chained.resolved_valid);
    assert_eq!(
        chained.resolved_plane.origin,
        Vec3::new_unchecked(0.0, 0.0, 15.0),
        "10 + 5 along the shared +Z normal"
    );
}

#[test]
fn an_unresolvable_base_inserts_lenient_but_invalid() {
    let mut sess = blank_session();
    // Unknown base name.
    add_offset_datum(&mut sess, did(10), "NOPE", 3.0).unwrap();
    assert!(!sess.document().datum(did(10)).unwrap().resolved_valid);

    // A base datum id that does not exist.
    add_offset_datum(&mut sess, did(11), &did(0xFFF).to_string(), 3.0).unwrap();
    assert!(!sess.document().datum(did(11)).unwrap().resolved_valid);

    // Chaining off an INVALID datum does not resolve either (no silent guess).
    add_offset_datum(&mut sess, did(12), &did(10).to_string(), 1.0).unwrap();
    assert!(!sess.document().datum(did(12)).unwrap().resolved_valid);

    // A kind the core cannot resolve without kernel context stays invalid too.
    let mut face_datum = DatumPlane::offset_from_plane(did(13), "FromFace", "XY", 2.0);
    face_datum.kind = onecad_core::document::datum::DatumKind::OffsetFromFace;
    sess.apply(EditCommand::AddDatumPlane { datum: face_datum })
        .unwrap();
    assert!(!sess.document().datum(did(13)).unwrap().resolved_valid);
}

#[test]
fn a_datum_hosted_sketch_is_stamped_with_the_core_resolved_frame() {
    let mut sess = blank_session();
    add_offset_datum(&mut sess, did(14), "XY", 10.0).unwrap();

    // The frontend's sketch carries the XY placeholder frame (`Sketch::new`), and
    // a hostile one could carry anything — the core overwrites it either way.
    let mut sk = Sketch::new(
        sid(9),
        "Sketch on datum",
        SketchAttachment::Datum { datum: did(14) },
    );
    sk.plane = onecad_core::sketch::SketchPlane::yz();
    sess.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    let stored = &sess.document().sketches[&sid(9)];
    assert_eq!(stored.plane.origin, Vec3::new_unchecked(0.0, 0.0, 10.0));
    assert_eq!(
        stored.plane.x_axis,
        onecad_core::sketch::SketchPlane::xy().x_axis,
        "the datum carried XY's axes verbatim, so the sketch reads like a sketch on XY"
    );
}

#[test]
fn a_sketch_on_a_missing_or_unresolved_datum_is_rejected_loudly() {
    let mut sess = blank_session();
    let missing = Sketch::new(sid(10), "S", SketchAttachment::Datum { datum: did(99) });
    let err = sess
        .apply(EditCommand::AddSketch { sketch: missing })
        .expect_err("a dangling datum ref must not be accepted");
    assert!(err.to_string().contains("does not exist"), "{err}");

    add_offset_datum(&mut sess, did(15), "NOPE", 1.0).unwrap(); // invalid
    let unresolved = Sketch::new(sid(11), "S", SketchAttachment::Datum { datum: did(15) });
    let err = sess
        .apply(EditCommand::AddSketch { sketch: unresolved })
        .expect_err("an unresolved datum must not host a sketch");
    assert!(err.to_string().contains("unresolved"), "{err}");
    assert!(
        sess.document().sketches.is_empty(),
        "a rejected AddSketch must not partially apply"
    );
}

#[test]
fn delete_datum_is_refused_while_a_sketch_references_it_and_names_the_blockers() {
    let mut sess = blank_session();
    add_offset_datum(&mut sess, did(16), "XY", 10.0).unwrap();
    sess.apply(EditCommand::AddSketch {
        sketch: Sketch::new(
            sid(12),
            "Sketch 1",
            SketchAttachment::Datum { datum: did(16) },
        ),
    })
    .unwrap();

    let err = sess
        .apply(EditCommand::DeleteDatum { datum: did(16) })
        .expect_err("a referenced datum must not be deletable");
    let msg = err.to_string();
    assert!(
        msg.contains("Sketch 1"),
        "message must NAME the blocker: {msg}"
    );
    assert!(msg.contains(&sid(12).to_string()), "and its id: {msg}");
    assert!(sess.document().datum(did(16)).is_some(), "nothing removed");

    // Drop the sketch and the same delete now succeeds.
    sess.apply(EditCommand::DeleteSketch { sketch: sid(12) })
        .unwrap();
    sess.apply(EditCommand::DeleteDatum { datum: did(16) })
        .unwrap();
    assert!(sess.document().datum(did(16)).is_none());
}

#[test]
fn delete_datum_round_trips_through_undo_redo() {
    let mut sess = blank_session();
    add_offset_datum(&mut sess, did(17), "XY", 7.0).unwrap();
    let before = json(sess.document());

    sess.apply(EditCommand::DeleteDatum { datum: did(17) })
        .unwrap();
    assert!(sess.document().datum(did(17)).is_none());

    assert!(sess.undo().is_some(), "undo available");
    assert_eq!(
        json(sess.document()),
        before,
        "undo restores the exact datum"
    );
    // Specifically: the RESOLVED frame comes back, not a re-defaulted one.
    let d = sess.document().datum(did(17)).unwrap();
    assert!(d.resolved_valid);
    assert_eq!(d.resolved_plane.origin, Vec3::new_unchecked(0.0, 0.0, 7.0));

    assert!(sess.redo().unwrap().is_some(), "redo available");
    assert!(sess.document().datum(did(17)).is_none());
}

#[test]
fn delete_datum_rejects_an_unknown_id() {
    let mut sess = blank_session();
    assert!(sess
        .apply(EditCommand::DeleteDatum { datum: did(0xABC) })
        .is_err());
}

// ── Frontend wire pin #3: the datum commands ─────────────────────────────────
//
// Same contract class as the AddSketch pin above. `AddDatumPlane` carries the RAW
// core `DatumPlane` struct, which has NO serde defaults for kind/offset/angleDeg/
// resolvedPlane/resolvedValid — so a TS builder that omits any of them breaks the
// whole command at deserialize, silently, with no compile-time signal on either
// side. These payloads are copied VERBATIM from `buildAddDatumPlane` /
// `deleteDatumCommand` in `src/ipc/tauriCommandMap.ts`; change both together.

#[test]
fn frontend_add_datum_plane_wire_parses_and_applies() {
    let wire = r#"{
        "cmd": "addDatumPlane",
        "datum": {
            "id": "a1b2c3d4-0000-4000-8000-000000000001",
            "name": "Datum 1",
            "kind": "OffsetFromPlane",
            "basePlaneId": "XY",
            "offset": 10,
            "angleDeg": 0,
            "resolvedPlane": {
                "origin": [0, 0, 0],
                "xAxis": [0, 1, 0],
                "yAxis": [-1, 0, 0],
                "normal": [0, 0, 1]
            },
            "resolvedValid": false
        }
    }"#;

    let cmd: EditCommand =
        serde_json::from_str(wire).expect("frontend AddDatumPlane wire must parse");
    let mut sess = DocumentSession::new(base_document());
    sess.apply(cmd).unwrap();

    let id: DatumPlaneId = "a1b2c3d4-0000-4000-8000-000000000001".parse().unwrap();
    let stored = sess
        .document()
        .datum(id)
        .expect("datum lands in the document");
    // The frontend sent `resolvedValid: false` + an origin frame; the core
    // resolved it anyway. This is the "Rust is the basis authority" contract seen
    // from the wire.
    assert!(stored.resolved_valid);
    assert_eq!(
        stored.resolved_plane.origin,
        Vec3::new_unchecked(0.0, 0.0, 10.0)
    );

    // `kind` is REQUIRED: the same payload without it must NOT parse.
    let no_kind = wire.replace("\"kind\": \"OffsetFromPlane\",\n", "");
    assert!(
        no_kind.len() < wire.len(),
        "replace must have removed the kind field"
    );
    assert!(
        serde_json::from_str::<EditCommand>(&no_kind).is_err(),
        "AddDatumPlane without kind must fail (frontend MUST send it)"
    );
}

#[test]
fn frontend_delete_datum_wire_parses_and_applies() {
    let wire = r#"{ "cmd": "deleteDatum", "datum": "00000000-0000-0000-0000-00000000d001" }"#;
    let cmd: EditCommand =
        serde_json::from_str(wire).expect("frontend DeleteDatum wire must parse");
    assert!(matches!(cmd, EditCommand::DeleteDatum { datum } if datum == did(1)));

    let mut sess = DocumentSession::new(base_document());
    sess.apply(cmd).unwrap();
    assert!(sess.document().datum(did(1)).is_none());
}

// ── Hole: the session is the gate on the SCHEMA §7.3 conditional blocks ──────
//
// `HoleParams::validate` owns the rule matrix (unit-tested in `record.rs`); what
// these pin is that the SINGLE WRITER actually calls it, on BOTH authoring entry
// points. Without that wiring an invalid hole reaches `document.json` and the
// worker refuses it every regen thereafter — an un-openable feature the user
// cannot fix from the UI.

fn hole_op(hole_type: HoleType, diameter: f64, cb: Option<(f64, f64)>) -> Operation {
    Operation::Known(KnownOperation::Hole(HoleParams {
        target_body: BX(),
        face: ElementRef {
            primary: Some(PrimaryRef {
                body: BX(),
                element: ElementId::new("el_face"),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        point: Vec3::new_unchecked(1.0, 2.0, 3.0),
        hole_type,
        diameter: Scalar::new(diameter),
        depth: None,
        cb_diameter: cb.map(|(d, _)| Scalar::new(d)),
        cb_depth: cb.map(|(_, t)| Scalar::new(t)),
        cs_diameter: None,
        cs_angle_deg: None,
        thread: None,
        result_policy_version: Some(HOLE_RESULT_POLICY_VERSION),
        extra: Default::default(),
    }))
}

fn hole_doc() -> DocumentSession {
    let mut doc = Document::new(DocumentId(u(0x5E)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    DocumentSession::new(doc)
}

#[test]
fn add_operation_rejects_a_hole_whose_conditional_block_is_wrong() {
    let mut sess = hole_doc();
    // A counterbore MISSING its cb pair.
    let err = sess
        .apply(EditCommand::AddOperation {
            record: record(
                rid(1),
                "Hole",
                hole_op(HoleType::Counterbore, 6.0, None),
                vec![BX()],
            ),
            at_cursor: false,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("required for a counterbore hole"),
        "the session must NAME the missing block: {err}"
    );

    // A SIMPLE hole carrying one — the direction only this layer can catch (the
    // worker never reads a param it does not use, so the stale block would ride
    // the record forever and resurrect on the next profile flip).
    let err = sess
        .apply(EditCommand::AddOperation {
            record: record(
                rid(2),
                "Hole",
                hole_op(HoleType::Simple, 6.0, Some((11.0, 6.8))),
                vec![BX()],
            ),
            at_cursor: false,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("counterbore-only"),
        "a simple hole may not carry cb*: {err}"
    );
    assert_eq!(sess.document().timeline.len(), 0, "neither record landed");
}

#[test]
fn update_operation_params_rejects_an_invalid_hole_edit() {
    let mut sess = hole_doc();
    sess.apply(EditCommand::AddOperation {
        record: record(
            rid(1),
            "Hole",
            hole_op(HoleType::Counterbore, 6.0, Some((11.0, 6.8))),
            vec![BX()],
        ),
        at_cursor: false,
    })
    .expect("a well-formed counterbore is accepted");

    // cbDiameter must EXCEED the drill — an edit that shrinks it is refused, and
    // the STORED record is untouched.
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: hole_op(HoleType::Counterbore, 6.0, Some((6.0, 6.8))),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("cbDiameter must exceed diameter"),
        "{err}"
    );
    let Operation::Known(KnownOperation::Hole(p)) = &sess.document().timeline.record(0).unwrap().op
    else {
        panic!("expected the stored Hole");
    };
    assert_eq!(
        p.cb_diameter.as_ref().map(|s| s.value),
        Some(11.0),
        "the refused edit left the record alone"
    );
}

#[test]
fn a_hole_derives_its_host_body_and_face_as_inputs() {
    let mut sess = hole_doc();
    sess.apply(EditCommand::AddOperation {
        record: record(
            rid(1),
            "Hole",
            hole_op(HoleType::Simple, 6.0, None),
            vec![BX()],
        ),
        at_cursor: false,
    })
    .expect("add");
    // F8 self-healing: the session re-derives `inputs` from the op rather than
    // trusting whatever the caller supplied.
    let inputs = &sess.document().timeline.record(0).unwrap().inputs;
    assert_eq!(inputs.bodies, vec![BX()]);
    assert_eq!(inputs.elements, vec![ElementId::new("el_face")]);
}

// ── (g2) HISTORY-HARDEN H9: the Shell + Hole InputPath arms ──────────────────
//
// Before H9 the ONLY element-bearing `InputPath` was `FilletEdges`, so the repair
// panel sent every rebind as a fillet-edge edit and the core rejected it for any
// other op — repair was fillet/chamfer-only in practice. These two arms complete
// the element slots the wire `inputs[]` array actually exposes
// (`worker::wire::wire_op_inputs`; the ordered table is pinned there by
// `wire_op_inputs_slot_order_is_the_repair_slot_table`).

fn shell_op(open_faces: &[&str]) -> Operation {
    Operation::Known(KnownOperation::Shell(ShellParams {
        thickness: s(1.5),
        open_faces: open_faces.iter().map(|f| ElementId::new(*f)).collect(),
        faces: open_faces.iter().map(|f| face_ref_at(BX(), f)).collect(),
        target_body: Some(BX()),
        extra: Default::default(),
    }))
}

/// A ref with anchor evidence but NO `primary` — what an un-promoted pick looks
/// like. Both new arms must refuse it (see their doc comments).
fn evidence_only_ref() -> ElementRef {
    ElementRef {
        primary: None,
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: Vec3::new_unchecked(1.0, 2.0, 3.0),
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

fn face_ref_at(body: BodyId, el: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(el),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: Vec3::new_unchecked(1.0, 2.0, 3.0),
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

fn shell_doc(open_faces: &[&str]) -> DocumentSession {
    let mut doc = Document::new(DocumentId(u(0x5F)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Shell",
        shell_op(open_faces),
        vec![BX()],
    )]);
    DocumentSession::new(doc)
}

fn stored_shell_faces(sess: &DocumentSession) -> Vec<String> {
    let Operation::Known(KnownOperation::Shell(p)) =
        &sess.document().timeline.record(0).unwrap().op
    else {
        panic!("expected the stored Shell");
    };
    p.open_faces.iter().map(|f| f.to_string()).collect()
}

/// `ShellOpenFaces{k}` updates the bare id and typed evidence in lockstep, with
/// the same overwrite/append bounds discipline as `set_fillet_edge`.
#[test]
fn shell_open_face_rebind_preserves_typed_evidence() {
    let mut sess = shell_doc(&["el_a", "el_b"]);
    let before = json(sess.document());
    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::ShellOpenFaces { index: 1 },
        reference: InputRef::Element(face_ref_at(BX(), "el_fresh")),
    })
    .expect("shell open-face rebind");
    assert_eq!(stored_shell_faces(&sess), vec!["el_a", "el_fresh"]);
    let v = json(sess.document());
    assert_eq!(
        v["timeline"]["records"][0]["params"]["openFaces"],
        serde_json::json!(["el_a", "el_fresh"])
    );
    assert_eq!(
        v["timeline"]["records"][0]["params"]["faces"][1]["anchor"]["worldPoint"],
        serde_json::json!([1.0, 2.0, 3.0]),
        "the re-picked face keeps its anchor evidence"
    );
    // …and the derived inputs follow the rewritten slot.
    assert_eq!(
        sess.document().timeline.record(0).unwrap().inputs.elements,
        vec![ElementId::new("el_a"), ElementId::new("el_fresh")]
    );
    assert!(sess.undo().is_some());
    assert_eq!(json(sess.document()), before, "undo restores the shell");

    // Append exactly at the end grows the list (mirrors the fillet rule).
    let mut sess = shell_doc(&["el_a"]);
    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::ShellOpenFaces { index: 1 },
        reference: InputRef::Element(face_ref_at(BX(), "el_new")),
    })
    .expect("append at the end");
    assert_eq!(stored_shell_faces(&sess), vec!["el_a", "el_new"]);
}

#[test]
fn legacy_single_shell_repair_adopts_face_body_as_target() {
    let mut doc = Document::new(DocumentId(u(0x60)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    let op = Operation::Known(KnownOperation::Shell(ShellParams {
        thickness: s(1.5),
        open_faces: vec![ElementId::new("el_legacy")],
        faces: Vec::new(),
        target_body: None,
        extra: Default::default(),
    }));
    doc.timeline = Timeline::from_records(vec![record(rid(1), "Shell", op, vec![BX()])]);
    let mut session = DocumentSession::new(doc);

    session
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::ShellOpenFaces { index: 0 },
            reference: InputRef::Element(face_ref_at(BX(), "el_fresh")),
        })
        .expect("legacy Shell repair");

    let Operation::Known(KnownOperation::Shell(params)) =
        &session.document().timeline.record(0).unwrap().op
    else {
        panic!("expected Shell");
    };
    assert_eq!(params.target_body, Some(BX()));
    assert_eq!(params.faces[0].primary.as_ref().unwrap().body, BX());
}

#[test]
fn shell_open_face_rebind_refuses_out_of_range_and_evidence_only_refs() {
    let mut sess = shell_doc(&["el_a"]);
    // A gap PAST the end is out of range — appending at 2 into a 1-length list
    // would leave a hole the worker reads as a face it was never given.
    let err = sess
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::ShellOpenFaces { index: 2 },
            reference: InputRef::Element(face_ref_at(BX(), "el_new")),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("out of range"),
        "expected an out-of-range rejection, got: {err}"
    );
    assert_eq!(stored_shell_faces(&sess), vec!["el_a"], "record untouched");

    // An intent-only ref has no id for the lockstep bare-id list.
    let err = sess
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::ShellOpenFaces { index: 0 },
            reference: InputRef::Element(evidence_only_ref()),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("primary element id"),
        "expected a primary-id rejection, got: {err}"
    );
    assert_eq!(stored_shell_faces(&sess), vec!["el_a"], "record untouched");
}

/// `HoleFace` writes the WHOLE typed ref into `params.face` — evidence included,
/// unlike shell — and re-derives the record's inputs from it.
#[test]
fn hole_face_rebind_writes_the_whole_typed_ref() {
    let mut sess = hole_doc();
    sess.apply(EditCommand::AddOperation {
        record: record(
            rid(1),
            "Hole",
            hole_op(HoleType::Simple, 6.0, None),
            vec![BX()],
        ),
        at_cursor: false,
    })
    .expect("add the hole");
    let before = json(sess.document());

    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::HoleFace,
        reference: InputRef::Element(face_ref_at(BX(), "el_repicked")),
    })
    .expect("hole host-face rebind");

    let Operation::Known(KnownOperation::Hole(p)) = &sess.document().timeline.record(0).unwrap().op
    else {
        panic!("expected the stored Hole");
    };
    assert_eq!(
        p.face.primary.as_ref().map(|x| x.element.to_string()),
        Some("el_repicked".to_string())
    );
    assert!(
        p.face.anchor.is_some(),
        "the hole face slot is typed, so anchor evidence must survive"
    );
    assert_eq!(
        sess.document().timeline.record(0).unwrap().inputs.elements,
        vec![ElementId::new("el_repicked")],
        "the derived inputs follow the rebind"
    );
    assert!(sess.undo().is_some());
    assert_eq!(json(sess.document()), before, "undo restores the hole");
}

#[test]
fn hole_face_rebind_refuses_an_evidence_only_ref_and_a_mismatched_op() {
    let mut sess = hole_doc();
    sess.apply(EditCommand::AddOperation {
        record: record(
            rid(1),
            "Hole",
            hole_op(HoleType::Simple, 6.0, None),
            vec![BX()],
        ),
        at_cursor: false,
    })
    .expect("add the hole");

    // No `primary` ⇒ the user's explicit re-pick would silently fall through to
    // the ladder on the very slot they just picked by hand.
    let err = sess
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::HoleFace,
            reference: InputRef::Element(evidence_only_ref()),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("primary element id"),
        "expected a primary-id rejection, got: {err}"
    );

    // The path/op pairing is still checked: `HoleFace` on a Shell is a mis-repair.
    let mut sess = shell_doc(&["el_a"]);
    let err = sess
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::HoleFace,
            reference: InputRef::Element(face_ref_at(BX(), "el_x")),
        })
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("does not match the operation type"),
        "expected a path/op mismatch rejection, got: {err}"
    );
    assert_eq!(stored_shell_faces(&sess), vec!["el_a"], "record untouched");
}

// ── OffsetFace repair slots (SCHEMA §7.3, 2026-08-06) ───────────────────────

/// An `Offset` push-pull over `faces`, or a `Total` one when `opposite` is set.
fn offset_face_op(faces: &[&str], opposite: Option<&str>) -> Operation {
    offset_face_op_with_primary(faces, faces, opposite)
}

/// As [`offset_face_op`], but with the V2 design-face intent stated separately from
/// the frozen G1 closure. Seeding the two identically — which every other fixture
/// here does — cannot exercise the closure-only rebind path at all.
fn offset_face_op_with_primary(
    faces: &[&str],
    primary: &[&str],
    opposite: Option<&str>,
) -> Operation {
    Operation::Known(KnownOperation::OffsetFace(OffsetFaceParams {
        face_ids: faces.iter().map(|f| ElementId::new(*f)).collect(),
        primary_face_ids: primary.iter().map(|f| ElementId::new(*f)).collect(),
        faces: faces.iter().map(|f| face_ref_at(BX(), f)).collect(),
        distance: Scalar::new(2.5),
        distance_type: match opposite {
            Some(_) => OffsetDistanceType::Total,
            None => OffsetDistanceType::Offset,
        },
        chain_tangent_faces: opposite.is_none(),
        opposite_face_id: opposite.map(ElementId::new),
        opposite_face: opposite.map(|o| face_ref_at(BX(), o)),
        target_body: BX(),
        result_policy_version: Some(2),
        extra: Default::default(),
    }))
}

fn offset_face_doc(faces: &[&str], opposite: Option<&str>) -> DocumentSession {
    let mut doc = Document::new(DocumentId(u(0x60)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Offset face",
        offset_face_op(faces, opposite),
        vec![BX()],
    )]);
    DocumentSession::new(doc)
}

fn stored_offset(sess: &DocumentSession) -> OffsetFaceParams {
    let Operation::Known(KnownOperation::OffsetFace(p)) =
        &sess.document().timeline.record(0).unwrap().op
    else {
        panic!("expected the stored OffsetFace");
    };
    p.clone()
}

/// Rebinding a face that is in the frozen G1 closure but NOT in the design intent
/// must leave `primary_face_ids` byte-identical. The complement of
/// `offset_face_rebind_writes_the_typed_ref_and_mirrors_the_bare_id`: together they
/// pin both branches of the `*primary == old_id` guard, which nothing else reaches
/// because every other fixture seeds the two vectors identically.
#[test]
fn offset_face_rebind_of_a_closure_face_leaves_primary_intent_unchanged() {
    let mut doc = Document::new(DocumentId(u(0x60)));
    doc.bodies.register(BodyMeta::new(BX(), "b", rid(0)));
    doc.timeline = Timeline::from_records(vec![record(
        rid(1),
        "Offset face",
        offset_face_op_with_primary(&["el_design", "el_closure"], &["el_design"], None),
        vec![BX()],
    )]);
    let mut sess = DocumentSession::new(doc);

    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::OffsetFaceFace { index: 1 },
        reference: InputRef::Element(face_ref_at(BX(), "el_fresh")),
    })
    .expect("closure-face rebind");

    let p = stored_offset(&sess);
    assert_eq!(
        p.face_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["el_design", "el_fresh"],
        "the rebound closure member follows the repaired reference"
    );
    assert_eq!(
        p.primary_face_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["el_design"],
        "design intent is untouched: the user never picked the closure face"
    );
    assert!(
        p.validate().is_ok(),
        "primary intent stays a subset of the closure after the rebind"
    );
}

/// `OffsetFaceFace{k}` writes the whole typed ref and keeps its bare mirror in
/// lockstep, matching the Shell/Fillet dual-field discipline.
#[test]
fn offset_face_rebind_writes_the_typed_ref_and_mirrors_the_bare_id() {
    let mut sess = offset_face_doc(&["el_a", "el_b"], None);
    let before = json(sess.document());

    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::OffsetFaceFace { index: 1 },
        reference: InputRef::Element(face_ref_at(BX(), "el_fresh")),
    })
    .expect("offset operative-face rebind");

    let p = stored_offset(&sess);
    assert_eq!(
        p.face_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["el_a", "el_fresh"],
        "the bare id mirror follows the typed ref"
    );
    assert_eq!(
        p.primary_face_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["el_a", "el_fresh"],
        "V2 primary intent follows an explicit repair rebind"
    );
    assert_eq!(
        p.faces[1].primary.as_ref().map(|x| x.element.to_string()),
        Some("el_fresh".to_string())
    );
    assert!(
        p.faces[1].anchor.is_some(),
        "an OffsetFace slot is a typed ElementRef, so the anchor evidence SURVIVES"
    );
    assert!(sess.undo().is_some());
    assert_eq!(json(sess.document()), before, "undo restores the offset");

    // Append exactly at the end grows BOTH lists (the fillet rule).
    let mut sess = offset_face_doc(&["el_a"], None);
    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::OffsetFaceFace { index: 1 },
        reference: InputRef::Element(face_ref_at(BX(), "el_new")),
    })
    .expect("append at the end");
    let p = stored_offset(&sess);
    assert_eq!(p.faces.len(), 2);
    assert_eq!(p.face_ids.len(), 2);
}

/// `OffsetFaceOpposite` addresses the LAST slot — the `Total` opposite face — and
/// keeps `oppositeFaceId` in lockstep with the typed ref.
#[test]
fn offset_face_opposite_rebind_targets_the_last_slot() {
    let mut sess = offset_face_doc(&["el_a"], Some("el_under"));
    let before = json(sess.document());

    sess.apply(EditCommand::EditOperationInput {
        record: rid(1),
        path: InputPath::OffsetFaceOpposite,
        reference: InputRef::Element(face_ref_at(BX(), "el_under2")),
    })
    .expect("offset opposite-face rebind");

    let p = stored_offset(&sess);
    assert_eq!(
        p.opposite_face_id.as_ref().map(ToString::to_string),
        Some("el_under2".to_string())
    );
    assert_eq!(
        p.opposite_face
            .as_ref()
            .and_then(|r| r.primary.as_ref())
            .map(|x| x.element.to_string()),
        Some("el_under2".to_string())
    );
    assert_eq!(
        p.face_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["el_a"],
        "the operative set is untouched"
    );
    assert!(sess.undo().is_some());
    assert_eq!(json(sess.document()), before, "undo restores the offset");
}

#[test]
fn offset_face_rebind_refuses_out_of_range_evidence_only_and_mismatched_ops() {
    let mut sess = offset_face_doc(&["el_a"], None);
    // A gap PAST the end would leave a hole the worker reads as a face it was
    // never given.
    let err = sess
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::OffsetFaceFace { index: 2 },
            reference: InputRef::Element(face_ref_at(BX(), "el_new")),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("out of range"),
        "expected an out-of-range rejection, got: {err}"
    );
    assert_eq!(stored_offset(&sess).faces.len(), 1, "record untouched");

    // An intent-only ref would silently fall back to the ladder on the very slot
    // the user just re-picked BY HAND.
    for path in [
        InputPath::OffsetFaceFace { index: 0 },
        InputPath::OffsetFaceOpposite,
    ] {
        let err = sess
            .apply(EditCommand::EditOperationInput {
                record: rid(1),
                path,
                reference: InputRef::Element(evidence_only_ref()),
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("primary element id"),
            "expected a primary-id rejection, got: {err}"
        );
    }

    // The path/op pairing is checked: an offset path on a Shell is a mis-repair.
    let mut shell = shell_doc(&["el_a"]);
    let err = shell
        .apply(EditCommand::EditOperationInput {
            record: rid(1),
            path: InputPath::OffsetFaceFace { index: 0 },
            reference: InputRef::Element(face_ref_at(BX(), "el_x")),
        })
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("does not match the operation type"),
        "expected a path/op mismatch rejection, got: {err}"
    );
    assert_eq!(stored_shell_faces(&shell), vec!["el_a"], "record untouched");
}

/// The SCHEMA §7.3 conditionals are enforced on BOTH authoring entry points —
/// `AddOperation` and `UpdateOperationParams` — so a params edit cannot smuggle in
/// a shape the add path rejects.
#[test]
fn offset_face_validation_runs_on_add_and_update() {
    let mut sess = offset_face_doc(&["el_a"], None);

    // add: multi-face is Offset-only.
    let mut bad = offset_face_op(&["el_a", "el_b"], None);
    if let Operation::Known(KnownOperation::OffsetFace(p)) = &mut bad {
        p.distance_type = OffsetDistanceType::Radius;
    }
    let err = sess
        .apply(EditCommand::AddOperation {
            record: record(rid(2), "Offset face", bad, vec![BX()]),
            at_cursor: false,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("exactly one face"),
        "expected a face-count rejection, got: {err}"
    );

    // update: a `Total` opposite face stranded by a distance-type switch.
    let mut stale = offset_face_op(&["el_a"], Some("el_under"));
    if let Operation::Known(KnownOperation::OffsetFace(p)) = &mut stale {
        p.distance_type = OffsetDistanceType::Offset;
    }
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: stale,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("Total-only"),
        "expected a stale-opposite rejection, got: {err}"
    );

    // update: the faces/faceIds lockstep.
    let mut broken = offset_face_op(&["el_a"], None);
    if let Operation::Known(KnownOperation::OffsetFace(p)) = &mut broken {
        p.face_ids.push(ElementId::new("el_orphan"));
    }
    let err = sess
        .apply(EditCommand::UpdateOperationParams {
            record: rid(1),
            op: broken,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("length mismatch"),
        "expected a lockstep rejection, got: {err}"
    );
}

// ── (h) HISTORY-HARDEN H8: undo/redo thread a real dirty floor ───────────────

/// Every structural undo reports the exact step it moved, so the caller replays
/// `[floor, end]` instead of the whole history.
#[test]
fn undo_reports_the_step_it_moved() {
    let base = base_document();
    let len = base.timeline.len(); // 5
    let mut sess = DocumentSession::new(base);

    // Frontier append → undo floors at the appended index.
    sess.apply(EditCommand::AddOperation {
        record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
        at_cursor: false,
    })
    .unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(len),
        "undo of an append floors at the appended index"
    );

    // A params edit at step 2 → undo floors at 2.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: extrude_newbody(rid(2), 99.0, BY()).op,
    })
    .unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(2),
        "undo of a params edit floors at the edited step"
    );

    // A removal at step 3 → undo (the re-insert) floors at 3.
    sess.apply(EditCommand::RemoveOperation { record: rid(3) })
        .unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(3),
        "undo of a removal floors at the re-inserted index"
    );

    // A rollback → undo (the cursor restore) floors at the lower cursor.
    sess.apply(EditCommand::SetRollback { cursor: 2 }).unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(2),
        "undo of a roll-back floors at the lower of the two cursors"
    );
}

/// A sketch edit's undo floors at the PRODUCING `Sketch` op, not at the first
/// consumer — that op's own regen re-runs region detection worker-side. This is
/// the floor that closes the sketch-geometry checkpoint hole.
#[test]
fn undo_of_a_sketch_edit_floors_at_its_producing_op() {
    let mut sess = DocumentSession::new(base_document());
    sess.apply(EditCommand::SketchEdit {
        sketch: S1(),
        ops: vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(eid(77), Vec2::new_unchecked(1.0, 2.0), false, false),
        }],
    })
    .unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(0),
        "the Sketch op producing S1 sits at step 0"
    );
}

/// A metadata-only revert claims NO floor: nothing the timeline regenerates moved,
/// so the caller must schedule no regen (and evict no checkpoint).
#[test]
fn undo_of_a_display_only_edit_claims_no_floor() {
    let mut sess = DocumentSession::new(base_document());
    sess.apply(EditCommand::SetVisibility {
        target: VisibilityTarget::Sketch(S1()),
        visible: false,
    })
    .unwrap();
    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        None,
        "sketch visibility is display-only"
    );
}

/// A transaction undoes/redoes as one step, and both directions fold to the LOWEST
/// floor across its edits — never to the last one applied.
#[test]
fn a_transaction_folds_to_its_lowest_floor_in_both_directions() {
    let mut sess = DocumentSession::new(base_document());
    sess.begin_transaction("batch");
    // Deliberately highest-first, so a "last wins" fold would report 3.
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(3),
        op: boolean(rid(3), BX(), BY(), BX()).op,
    })
    .unwrap();
    sess.apply(EditCommand::UpdateOperationParams {
        record: rid(1),
        op: extrude_newbody(rid(1), 77.0, BX()).op,
    })
    .unwrap();
    sess.end_transaction().expect("one committed step");

    assert_eq!(
        sess.undo().expect("undo available").dirty_floor,
        Some(1),
        "the batch's lowest edited step wins"
    );
    assert_eq!(
        sess.redo().unwrap().expect("redo available").dirty_floor,
        Some(1),
        "redo folds the RE-EXECUTED commands' dirty spans (they used to be discarded)"
    );
}

/// Redo derives its floor from the forward commands it replays, so it matches the
/// span the original commit scheduled.
#[test]
fn redo_reports_the_span_the_original_commit_scheduled() {
    let base = base_document();
    let len = base.timeline.len();
    let mut sess = DocumentSession::new(base);
    let outcome = sess
        .apply(EditCommand::AddOperation {
            record: extrude_newbody(rid(0x99), 3.0, bid(0x99)),
            at_cursor: false,
        })
        .unwrap();
    assert_eq!(outcome.dirty.map(|d| d.from), Some(len));
    sess.undo().expect("undo available");
    assert_eq!(
        sess.redo().unwrap().expect("redo available").dirty_floor,
        Some(len),
        "redo replays from the same floor the commit did"
    );
}

/// Nothing to revert ⇒ `None` (the signal the caller uses to skip the regen).
#[test]
fn an_empty_stack_reverts_nothing() {
    let mut sess = DocumentSession::new(base_document());
    assert!(sess.undo().is_none(), "nothing committed yet");
    assert!(sess.redo().unwrap().is_none(), "nothing undone yet");
    // An open transaction blocks both (C++ `CommandProcessor` parity).
    sess.apply(EditCommand::SetRollback { cursor: 2 }).unwrap();
    sess.begin_transaction("open");
    assert!(sess.undo().is_none(), "undo is ignored mid-transaction");
    assert!(
        sess.redo().unwrap().is_none(),
        "redo is ignored mid-transaction"
    );
}
