//! WP-FIX W4 — `TransformBody` edit-safety gate holes (VF-B5 a/b/c) and the
//! single-snapshot repair-inverse rule (VF-M8), at the pure-core level.
//!
//! Every test here is a RED-FIRST pin on a path that silently mis-bound geometry
//! before this wave:
//!
//! * **VF-M8** — a cascade (un)suppression across two `TransformBody` records
//!   wrapped one repair snapshot per transform, nested; undo applies a composite in
//!   REVERSE, so the LAST-taken snapshot (which already carried the first
//!   transform's seeds) won and the first gate survived undo forever.
//! * **VF-B5a** — a face-hosted sketch declared NO inputs, so the gate could not
//!   see that moving the host body invalidates it (and everything cut from it).
//! * **VF-B5b** — DELETING a `TransformBody` bypassed the gate that suppressing
//!   the same record triggers.
//! * **VF-B5c** — the seeded `step_index` is positional and was never remapped, so
//!   a timeline insert/delete silently re-pointed the regen ceiling at the wrong
//!   step (or left a gate no record could ever close).
//!
//! Geometry-bearing variants (real OCCT worker) live in
//! `src-tauri/tests/transform_body.rs`.

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef, TransformBodyParams,
    TransformRotation,
};
use onecad_core::document::refs::{ElementKind, ElementRef, PrimaryRef, SketchRegionRef};
use onecad_core::document::variables::Scalar;
use onecad_core::document::Document;
use onecad_core::edit::{DocumentSession, EditCommand};
use onecad_core::history::Timeline;
use onecad_core::ids::{BodyId, DocumentId, ElementId, RecordId, RegionId, SketchId};
use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
use onecad_core::math::Vec3;
use onecad_core::regen::history_prefix_hash;
use onecad_core::sketch::{Sketch, SketchAttachment, SketchPlane, WorldPlane};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn rid(n: u128) -> RecordId {
    RecordId(Uuid::from_u128(0x4A00 + n))
}
fn bid(n: u128) -> BodyId {
    BodyId(Uuid::from_u128(0x4B00 + n))
}
fn skid(n: u128) -> SketchId {
    SketchId(Uuid::from_u128(0x4C00 + n))
}

fn plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A face ref on `body` naming element `el`.
fn face_ref(body: BodyId, el: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(el),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}

fn sketch_record(id: RecordId, sketch: SketchId, host_face: Option<ElementRef>) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch,
        plane: plane_ref(),
        entities: vec![],
        constraints: vec![],
        host_face,
        extra: Default::default(),
    }));
    OperationRecord::new(id, 0, "Sketch", op)
}

fn extrude_record(id: RecordId, sketch: SketchId, out: BodyId) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            extra: Default::default(),
        }),
        distance: Scalar::new(10.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    let mut r = OperationRecord::new(id, 0, "Extrude", op);
    r.outputs = vec![out];
    r
}

fn transform_op(targets: Vec<BodyId>, dx: f64) -> Operation {
    Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: [Scalar::new(dx), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: TransformRotation::default(),
        copy: false,
        extra: Default::default(),
    }))
}

fn transform_record(id: RecordId, target: BodyId, dx: f64) -> OperationRecord {
    let mut r = OperationRecord::new(id, 0, "Move", transform_op(vec![target], dx));
    r.outputs = vec![target];
    r
}

fn fillet_record(id: RecordId, body: BodyId, el: &str) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Fillet(FilletParams {
        edge_ids: vec![ElementId::new(el)],
        edges: vec![ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(el),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        }],
        radius: Scalar::new(2.0),
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    let mut r = OperationRecord::new(id, 0, "Fillet", op);
    r.outputs = vec![body];
    r
}

fn session_over(records: Vec<OperationRecord>) -> DocumentSession {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    let len = records.len();
    doc.timeline = Timeline::from_records(records);
    doc.timeline.set_cursor(len);
    DocumentSession::new(doc)
}

fn seeded_steps(s: &DocumentSession) -> Vec<usize> {
    s.document().repair.seeded_steps()
}

// ─────────────────────────────────────────────────────────────────────────────
// (t1) VF-M8 — one command, one repair snapshot
// ─────────────────────────────────────────────────────────────────────────────

/// **The VF-M8 pin.** A cascade UN-suppression whose closure covers TWO
/// `TransformBody` records must undo the repair state to EXACTLY what it was at
/// command entry — including a gate that was already standing on an unrelated
/// lineage.
///
/// RED (pre-W4): `set_suppression` called `seed_transform_gate` once per affected
/// transform, and each call wrapped its OWN `RestoreRepair` around the growing
/// inverse. `Inverse::Composite` applies its members in REVERSE (`undo.rs`), so the
/// outermost (last-taken, already-seeded) snapshot was applied LAST and won — undo
/// left the first transform's seeds behind and the regen ceiling stayed truncated
/// until a bogus manual repair.
///
/// The un-suppress direction is used because a cascade SUPPRESS also suppresses
/// every downstream ref, and the gate deliberately skips suppressed records (they
/// never execute), so nothing would be seeded to expose the ordering.
#[test]
fn cascade_unsuppression_across_two_transforms_undoes_the_repair_state_exactly() {
    // One sketch feeding TWO bodies, so a single cascade closure covers two
    // transforms whose gate sets are DISJOINT (each gates only its own fillet) —
    // the shape that makes the per-transform snapshots observably differ:
    //   0 Sketch(sk1) · 1 Extrude→bA · 2 Move(bA) · 3 Fillet(bA)
    //                 · 4 Extrude→bB · 5 Move(bB) · 6 Fillet(bB)
    // plus an INDEPENDENT lineage C (7..10) that carries a pre-existing gate.
    let (ba, bb, bc) = (bid(1), bid(2), bid(3));
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), ba),
        transform_record(rid(2), ba, 10.0),
        fillet_record(rid(3), ba, "el_a"),
        extrude_record(rid(4), skid(1), bb),
        transform_record(rid(5), bb, 5.0),
        fillet_record(rid(6), bb, "el_b"),
        sketch_record(rid(7), skid(2), None),
        extrude_record(rid(8), skid(2), bc),
        transform_record(rid(9), bc, 3.0),
        fillet_record(rid(10), bc, "el_c"),
    ]);
    // Setup 1: park lineages A+B suppressed (no gate — every downstream ref is
    // suppressed too, and the gate skips those).
    s.apply(EditCommand::SetOperationSuppression {
        record: rid(0),
        suppressed: true,
        cascade: true,
    })
    .expect("suppress the shared sketch's whole closure");
    // Setup 2: plant a gate on the UNRELATED lineage C, so the command under test
    // is entered against a NON-EMPTY repair state (the sharpest form of the bug:
    // the wrong snapshot restores a state that is neither empty nor final).
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(9),
        op: transform_op(vec![bc], 12.0),
    })
    .expect("edit lineage C's Move");
    let at_entry = s.document().repair.clone();
    assert_eq!(
        at_entry.seeded_steps(),
        vec![10],
        "precondition: exactly lineage C's fillet is gated"
    );

    // ── The command under test: ONE cascade un-suppress, TWO transforms ──────
    s.apply(EditCommand::SetOperationSuppression {
        record: rid(0),
        suppressed: false,
        cascade: true,
    })
    .expect("cascade un-suppress");
    let after: Vec<usize> = seeded_steps(&s);
    assert!(
        after.contains(&3) && after.contains(&6),
        "BOTH transforms seeded their own (disjoint) gate, got {after:?}"
    );

    assert!(s.undo().is_some(), "undo the cascade un-suppression");
    assert_eq!(
        s.document().repair,
        at_entry,
        "ONE command ⇒ ONE repair snapshot, applied LAST under Composite's reverse \
         order: undo restores the command-entry state EXACTLY (VF-M8 — the FIRST \
         transform's seeds used to survive undo)"
    );
}

/// The inverse must also be exact for a params edit that both CLOSES a gate and
/// OPENS one (`edit_operation_input` clears the step's seeds, then re-seeds).
#[test]
fn a_single_command_emits_exactly_one_restore_repair() {
    let b = bid(1);
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        fillet_record(rid(3), b, "el_a"),
    ]);
    let pristine = s.document().repair.clone();
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 25.0),
    })
    .expect("params edit");
    assert_eq!(seeded_steps(&s), vec![3]);
    // A SECOND edit re-enters with gates already standing.
    let after_first = s.document().repair.clone();
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 40.0),
    })
    .expect("second params edit");
    assert!(s.undo().is_some(), "undo the second edit");
    assert_eq!(
        s.document().repair,
        after_first,
        "second edit undone exactly"
    );
    assert!(s.undo().is_some(), "undo the first edit");
    assert_eq!(s.document().repair, pristine, "first edit undone exactly");
}

// ─────────────────────────────────────────────────────────────────────────────
// (t4-core) VF-B5b — deleting a transform seeds the gate
// ─────────────────────────────────────────────────────────────────────────────

/// RED (pre-W4): `remove_operation` never touched the repair state, so DELETING a
/// `TransformBody` re-bound every downstream ref against un-moved geometry with
/// zero `NeedsRepair` — while merely SUPPRESSING the same record gated it.
#[test]
fn deleting_a_transform_seeds_the_same_gate_as_suppressing_it() {
    let b = bid(1);
    let records = vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        fillet_record(rid(3), b, "el_a"),
    ];

    // Reference behaviour: suppression gates the fillet at step 3.
    let mut suppressed = session_over(records.clone());
    suppressed
        .apply(EditCommand::SetOperationSuppression {
            record: rid(2),
            suppressed: true,
            cascade: false,
        })
        .expect("suppress");
    let suppress_refs: Vec<String> = suppressed
        .document()
        .repair
        .seeded_items()
        .iter()
        .map(|i| i.ref_id.clone())
        .collect();
    assert_eq!(seeded_steps(&suppressed), vec![3]);

    // The delete path must gate the SAME refs — at the POST-removal index (2).
    let mut s = session_over(records);
    let pristine = s.document().repair.clone();
    s.apply(EditCommand::RemoveOperation { record: rid(2) })
        .expect("remove the Move");
    let items = s.document().repair.seeded_items();
    assert!(
        !items.is_empty(),
        "deleting a TransformBody seeds the edit-safety gate (VF-B5b)"
    );
    assert_eq!(
        seeded_steps(&s),
        vec![2],
        "the fillet shifted down to index 2 when the Move was removed — the seeded \
         step_index must address the POST-removal timeline"
    );
    let delete_refs: Vec<String> = items.iter().map(|i| i.ref_id.clone()).collect();
    assert_eq!(
        delete_refs, suppress_refs,
        "delete gates the same refs as suppress"
    );
    assert_eq!(
        items[0].element_id.as_ref().map(ElementId::as_str),
        Some("el_a"),
        "the gate names the fillet's edge ref"
    );

    // Undo restores the timeline AND the repair state exactly.
    assert!(s.undo().is_some(), "undo the removal");
    assert_eq!(s.document().timeline.len(), 4, "the record is back");
    assert_eq!(s.document().timeline.index_of(rid(2)), Some(2));
    assert_eq!(
        s.document().repair,
        pristine,
        "the delete's seeds ride the delete's own undo entry"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (t5) VF-B5c — seeded step_index remap on insert / delete
// ─────────────────────────────────────────────────────────────────────────────

/// RED (pre-W4): `RepairItem::step_index` is positional and nothing remapped it, so
/// inserting a record below a gate left the ceiling pointing one step too low
/// (truncating an innocent op while the gated one ran free), and deleting one left
/// it a step too high.
#[test]
fn a_seeded_gate_tracks_its_record_across_inserts_and_deletes() {
    let b = bid(1);
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        fillet_record(rid(3), b, "el_a"),
    ]);
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 25.0),
    })
    .expect("params edit seeds the gate at step 3");
    assert_eq!(s.document().repair.first_seeded_step(), Some(3));
    let gated_ref = s.document().repair.seeded_items()[0].ref_id.clone();
    assert!(
        gated_ref.starts_with(&rid(3).to_string()),
        "the gate names the fillet record"
    );

    // ── INSERT below the gate ⇒ +1, still the same record ────────────────────
    s.apply(EditCommand::SetRollback { cursor: 1 })
        .expect("roll the cursor back to index 1");
    s.apply(EditCommand::AddOperation {
        record: sketch_record(rid(9), skid(2), None),
        at_cursor: true,
    })
    .expect("insert at index 1");
    assert_eq!(
        s.document().repair.first_seeded_step(),
        Some(4),
        "the gate shifted with its record"
    );
    assert_eq!(
        s.document().timeline.index_of(rid(3)),
        Some(4),
        "and it still names the fillet"
    );
    assert_eq!(
        s.document().repair.seeded_items()[0].ref_id,
        gated_ref,
        "ref_id is record-keyed and never moves"
    );

    // ── DELETE below the gate ⇒ −1 ───────────────────────────────────────────
    s.apply(EditCommand::RemoveOperation { record: rid(9) })
        .expect("remove the inserted record");
    assert_eq!(s.document().repair.first_seeded_step(), Some(3));
    assert_eq!(s.document().timeline.index_of(rid(3)), Some(3));

    // ── DELETE the gated record itself ⇒ the item is dropped ─────────────────
    s.apply(EditCommand::RemoveOperation { record: rid(3) })
        .expect("remove the gated fillet");
    assert_eq!(
        s.document().repair.first_seeded_step(),
        None,
        "a gate whose record is gone can never be closed — drop it rather than pin \
         the regen ceiling forever"
    );
}

/// A frontier append (`at_cursor == false`, cursor already at the end) inserts
/// ABOVE every gate and must shift nothing.
#[test]
fn a_frontier_append_does_not_shift_a_seeded_gate() {
    let b = bid(1);
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        fillet_record(rid(3), b, "el_a"),
    ]);
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 25.0),
    })
    .expect("params edit");
    assert_eq!(s.document().repair.first_seeded_step(), Some(3));
    s.apply(EditCommand::AddOperation {
        record: fillet_record(rid(8), b, "el_z"),
        at_cursor: false,
    })
    .expect("frontier append");
    assert_eq!(
        s.document().repair.first_seeded_step(),
        Some(3),
        "an append past the gate leaves it exactly where it was"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (t2/t3-core) VF-B5a — the host-face dependency edge + the legacy bridge
// ─────────────────────────────────────────────────────────────────────────────

/// A `Sketch` record carrying `host_face` derives the host body + face as inputs,
/// which is what makes it visible to the gate.
#[test]
fn a_host_face_sketch_record_derives_its_host_as_an_input() {
    let b = bid(1);
    let rec = sketch_record(rid(5), skid(2), Some(face_ref(b, "el_top")));
    let inputs = rec.op.derive_inputs();
    assert_eq!(inputs.bodies, vec![b], "the host body is a real dependency");
    assert_eq!(
        inputs.elements,
        vec![ElementId::new("el_top")],
        "so is the host face element"
    );
    // A world/datum sketch keeps declaring nothing.
    let plain = sketch_record(rid(6), skid(3), None);
    assert!(plain.op.derive_inputs().bodies.is_empty());
}

/// RED (pre-W4): editing the Move under a face-hosted sketch produced ZERO seeded
/// items — the sketch (and every cut taken from it) silently re-projected against
/// moved geometry.
#[test]
fn editing_a_move_gates_the_sketch_hosted_on_the_moved_body() {
    let b = bid(1);
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        // step 3: a sketch glued to a face of the MOVED body
        sketch_record(rid(3), skid(2), Some(face_ref(b, "el_top"))),
        // step 4: the cut taken from it
        extrude_record(rid(4), skid(2), bid(2)),
    ]);
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 25.0),
    })
    .expect("edit the Move");

    let items = s.document().repair.seeded_items();
    assert!(
        items.iter().any(|i| i.step_index == 3),
        "the HOSTED SKETCH is gated, got {:?}",
        items.iter().map(|i| i.step_index).collect::<Vec<_>>()
    );
    assert!(
        items
            .iter()
            .any(|i| i.element_id.as_ref().map(ElementId::as_str) == Some("el_top")),
        "and the gate names the host FACE ref"
    );
    assert_eq!(
        s.document().repair.first_seeded_step(),
        Some(3),
        "the regen ceiling stops BELOW the sketch step, so the downstream cut can \
         never execute against re-projected geometry"
    );
}

/// The LEGACY bridge: a `Sketch` record minted before `host_face` existed carries
/// no param at all, yet the gate must still cover it — via the document-level
/// attachment. Its record bytes and derived inputs stay untouched (the hash-freeze
/// constraint: `inputs` is inside the golden prefix hash).
#[test]
fn a_legacy_hosted_sketch_is_gated_through_the_attachment_bridge() {
    let b = bid(1);
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    // The record has NO host_face — exactly what a pre-W4 document holds.
    let legacy = sketch_record(rid(3), skid(2), None);
    assert!(
        legacy.op.derive_inputs().bodies.is_empty(),
        "precondition: the legacy record declares nothing"
    );
    doc.timeline = Timeline::from_records(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        transform_record(rid(2), b, 10.0),
        legacy,
        extrude_record(rid(4), skid(2), bid(2)),
    ]);
    doc.timeline.set_cursor(5);
    // …but the DOCUMENT knows the sketch is glued to a face of the moved body.
    let mut sk = Sketch::new(
        skid(2),
        "OnFace",
        SketchAttachment::HostFace {
            face: face_ref(b, "el_top"),
            projected_boundary_version: 1,
        },
    );
    sk.plane = SketchPlane::xy();
    doc.sketches.insert(skid(2), sk);
    let mut s = DocumentSession::new(doc);

    s.apply(EditCommand::UpdateOperationParams {
        record: rid(2),
        op: transform_op(vec![b], 25.0),
    })
    .expect("edit the Move");

    let items = s.document().repair.seeded_items();
    assert!(
        items
            .iter()
            .any(|i| i.step_index == 3
                && i.element_id.as_ref().map(ElementId::as_str) == Some("el_top")),
        "a legacy hosted sketch is gated through the attachment bridge, got {:?}",
        items
    );
    // The record itself was never rewritten (hash-freeze).
    let rec = s.document().timeline.record(3).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("still a Sketch record")
    };
    assert!(p.host_face.is_none(), "the legacy record is NOT backfilled");
    assert!(rec.inputs.bodies.is_empty(), "nor are its derived inputs");
}

// ─────────────────────────────────────────────────────────────────────────────
// (t7) `UpdateSketchAttachment` restamps the edge
// ─────────────────────────────────────────────────────────────────────────────

/// Re-picking a host face must restamp the record's `host_face`, re-derive its
/// inputs, close that step's gate (re-picking IS the repair action), and undo
/// exactly. Without the restamp the gate would clear while the dependency edge
/// kept naming the OLD face forever.
#[test]
fn update_sketch_attachment_restamps_the_host_face_edge_and_closes_the_gate() {
    let (b1, b2) = (bid(1), bid(2));
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    doc.timeline = Timeline::from_records(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b1),
        sketch_record(rid(2), skid(9), None),
        extrude_record(rid(3), skid(9), b2),
        transform_record(rid(4), b1, 10.0),
        sketch_record(rid(5), skid(2), Some(face_ref(b1, "el_old"))),
    ]);
    doc.timeline.set_cursor(6);
    doc.sketches.insert(
        skid(2),
        Sketch::new(
            skid(2),
            "OnFace",
            SketchAttachment::HostFace {
                face: face_ref(b1, "el_old"),
                projected_boundary_version: 1,
            },
        ),
    );
    let mut s = DocumentSession::new(doc);

    // Seed a gate on the sketch's step by editing the Move it stands on.
    s.apply(EditCommand::UpdateOperationParams {
        record: rid(4),
        op: transform_op(vec![b1], 25.0),
    })
    .expect("edit the Move");
    assert_eq!(
        s.document().repair.first_seeded_step(),
        Some(5),
        "the hosted sketch is gated"
    );
    let gated = s.document().repair.clone();

    // The user re-picks the host: a face of the OTHER body.
    s.apply(EditCommand::UpdateSketchAttachment {
        sketch: skid(2),
        plane: SketchPlane::xy(),
        attachment: SketchAttachment::HostFace {
            face: face_ref(b2, "el_new"),
            projected_boundary_version: 2,
        },
    })
    .expect("re-pick the host face");

    let rec = s.document().timeline.record(5).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("Sketch record")
    };
    assert_eq!(
        p.host_face
            .as_ref()
            .and_then(|f| f.primary.as_ref())
            .map(|x| x.element.as_str().to_owned()),
        Some("el_new".to_owned()),
        "the record's host_face param was restamped"
    );
    assert_eq!(
        rec.inputs.bodies,
        vec![b2],
        "and the derived inputs followed — the dependency edge moved with the pick"
    );
    assert_eq!(
        s.document().repair.first_seeded_step(),
        None,
        "re-picking the host IS the repair action, so the step's gate closes"
    );

    // Undo restores the record, the attachment AND the repair state exactly.
    assert!(s.undo().is_some(), "undo the re-pick");
    assert_eq!(s.document().repair, gated, "the gate is back");
    let rec = s.document().timeline.record(5).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("Sketch record")
    };
    assert_eq!(
        p.host_face
            .as_ref()
            .and_then(|f| f.primary.as_ref())
            .map(|x| x.element.as_str().to_owned()),
        Some("el_old".to_owned()),
        "the old host ref is restored"
    );
    assert_eq!(rec.inputs.bodies, vec![b1]);
}

/// The anti-time-travel guard: a legacy sketch record that sits AHEAD of its host
/// body's producer (what `backfill_missing_sketch_records` authors) must not be
/// stamped — the edge would name a later producer, which `validate_temporal`
/// rejects. Nothing is left ungated: such a record is not downstream of any
/// transform on that body in the first place.
#[test]
fn a_sketch_record_ahead_of_its_host_body_is_not_stamped() {
    let b = bid(1);
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    doc.timeline = Timeline::from_records(vec![
        // step 0: the (front-inserted) sketch record
        sketch_record(rid(0), skid(2), None),
        sketch_record(rid(1), skid(1), None),
        // step 2: the producer of the host body — AFTER the sketch record
        extrude_record(rid(2), skid(1), b),
    ]);
    doc.timeline.set_cursor(3);
    doc.sketches.insert(
        skid(2),
        Sketch::new(
            skid(2),
            "OnFace",
            SketchAttachment::World {
                plane: WorldPlane::XY,
            },
        ),
    );
    let mut s = DocumentSession::new(doc);

    s.apply(EditCommand::UpdateSketchAttachment {
        sketch: skid(2),
        plane: SketchPlane::xy(),
        attachment: SketchAttachment::HostFace {
            face: face_ref(b, "el_top"),
            projected_boundary_version: 1,
        },
    })
    .expect("re-pick must not be rejected");

    let rec = s.document().timeline.record(0).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("Sketch record")
    };
    assert!(
        p.host_face.is_none(),
        "no stamp when the host body is produced by a LATER op (anti-time-travel)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (t8) Cascade semantics — DELIBERATE change
// ─────────────────────────────────────────────────────────────────────────────

/// The new `host_face` input edge puts a hosted sketch inside
/// `graph.downstream(host body producer)`. Suppressing the producer therefore now
/// cascades to the hosted sketch (and to everything cut from it) — INTENDED: a
/// face-attached sketch whose host body no longer exists is not a valid feature.
#[test]
fn suppressing_the_host_bodys_producer_cascades_to_the_hosted_sketch() {
    let b = bid(1);
    let mut s = session_over(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        sketch_record(rid(2), skid(2), Some(face_ref(b, "el_top"))),
        extrude_record(rid(3), skid(2), bid(2)),
    ]);
    s.apply(EditCommand::SetOperationSuppression {
        record: rid(1),
        suppressed: true,
        cascade: true,
    })
    .expect("cascade suppress the producer");
    let suppressed: Vec<bool> = s
        .document()
        .timeline
        .records()
        .iter()
        .map(|r| r.suppressed)
        .collect();
    assert_eq!(
        suppressed,
        vec![false, true, true, true],
        "the hosted sketch (step 2) and its cut (step 3) now ride the cascade"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (t6) Byte-stability of a legacy document
// ─────────────────────────────────────────────────────────────────────────────

/// A legacy container — a `HostFace`-attached sketch whose `Sketch` record carries
/// NO `host_face` param — must open + re-save BYTE-IDENTICAL, with an unchanged
/// history-prefix hash. `inputs` is inside the golden prefix hash and is re-derived
/// on every deserialize, so ANY load-time backfill of this field would silently
/// move every existing document's hash and bytes; the stamp is confined to a live
/// re-finish / re-pick.
#[test]
fn a_legacy_host_face_document_reopens_and_resaves_byte_identical() {
    let dir = std::env::temp_dir().join(format!("onecad-w4-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("legacy.onecad");

    let b = bid(1);
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    doc.timeline = Timeline::from_records(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        // The legacy shape: attachment on the sketch, NOTHING on the record.
        sketch_record(rid(2), skid(2), None),
    ]);
    doc.timeline.set_cursor(3);
    doc.sketches.insert(
        skid(2),
        Sketch::new(
            skid(2),
            "OnFace",
            SketchAttachment::HostFace {
                face: face_ref(b, "el_top"),
                projected_boundary_version: 1,
            },
        ),
    );
    let hash_before = history_prefix_hash(doc.timeline.records());

    let meta = SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    };
    ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
    let first = std::fs::read(&path).unwrap();

    // Open → save again, touching nothing.
    let loaded = ContainerReader::open(&path).unwrap();
    let reopened = loaded.document().clone();
    let hash_after = history_prefix_hash(reopened.timeline.records());
    assert_eq!(
        hash_before, hash_after,
        "the golden prefix hash of a legacy document does not move"
    );
    let rec = reopened.timeline.record(2).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("Sketch record")
    };
    assert!(
        p.host_face.is_none(),
        "no load-time backfill — the record is exactly as authored"
    );
    assert!(
        rec.inputs.bodies.is_empty(),
        "nor are inputs re-derived onto it"
    );

    let path2 = dir.join("resaved.onecad");
    ContainerWriter::save(&path2, &reopened, &ContainerCaches::none(), &meta).unwrap();
    let second = std::fs::read(&path2).unwrap();
    assert_eq!(
        first, second,
        "a legacy document opened and re-saved is BYTE-IDENTICAL"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The complement: a record that DOES carry `host_face` round-trips it verbatim
/// (and re-derives the same inputs on load, since inputs are never trusted from
/// disk).
#[test]
fn a_stamped_host_face_round_trips_through_the_container() {
    let dir = std::env::temp_dir().join(format!("onecad-w4-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stamped.onecad");

    let b = bid(1);
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0x4A)));
    doc.timeline = Timeline::from_records(vec![
        sketch_record(rid(0), skid(1), None),
        extrude_record(rid(1), skid(1), b),
        sketch_record(rid(2), skid(2), Some(face_ref(b, "el_top"))),
    ]);
    doc.timeline.set_cursor(3);
    let meta = SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    };
    ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
    let reopened = ContainerReader::open(&path).unwrap().document().clone();
    let rec = reopened.timeline.record(2).unwrap();
    let Operation::Known(KnownOperation::Sketch(p)) = &rec.op else {
        panic!("Sketch record")
    };
    assert_eq!(
        p.host_face
            .as_ref()
            .and_then(|f| f.primary.as_ref())
            .map(|x| x.element.as_str().to_owned()),
        Some("el_top".to_owned())
    );
    assert_eq!(
        rec.inputs.bodies,
        vec![b],
        "inputs re-derive from the param"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `hostFace` is absent from the JSON when unset, so no pre-existing document's
/// `document.json` moves by a byte.
#[test]
fn host_face_is_absent_from_the_wire_when_unset() {
    let plain = sketch_record(rid(0), skid(1), None);
    let v = serde_json::to_value(&plain.op).unwrap();
    assert!(
        v["params"].get("hostFace").is_none(),
        "byte-stable for every pre-W4 record"
    );
    let stamped = sketch_record(rid(1), skid(2), Some(face_ref(bid(1), "el_top")));
    let v = serde_json::to_value(&stamped.op).unwrap();
    assert_eq!(v["params"]["hostFace"]["primary"]["elementId"], "el_top");
    let back: Operation = serde_json::from_value(v).unwrap();
    assert_eq!(back, stamped.op, "round-trips verbatim");
}
