//! `TransformBody` lineage queries (WP-B W0; SCHEMA §7.3 `TransformBody`).
//!
//! One query lives here, and it is deliberately a **core** query rather than a
//! frontend heuristic: the frontend has no lineage. It sees a picked body id and
//! a flat history list, not "which record last touched this body".
//!
//! ## The fold rule
//!
//! A `TransformBody` is a *cumulative placement intent*, not a nudge: dragging a
//! part twice must produce ONE record whose params were re-edited, never a stack
//! of two. [`can_fold_transform`] answers "may the next placement gesture on this
//! body fold into an existing record?" and is the single authority for that.
//!
//! **Lineage definition implemented here** (conservative, V1):
//!
//! * Walk the APPLIED prefix (`timeline.cursor()`) from the end towards 0,
//!   skipping suppressed records (a suppressed record never executes, so it is
//!   not part of the body's current geometry).
//! * The first record that **touches** the body — its derived
//!   [`inputs.bodies`](crate::document::record::OperationInputs) OR its
//!   `outputs` contains the id — is the body's LAST producer/consumer.
//! * Fold is allowed iff that record is a `TransformBody` with `copy: false`
//!   whose `targets` contain the body. Anything else (a fillet, a boolean, a
//!   `copy: true` transform, an untouched body) ⇒ `None`.
//!
//! Why `copy: false` only: a `copy: true` record does not move its sources at
//! all, so folding a new placement of a SOURCE into it would silently relocate
//! the copies instead. And why "last touching record": folding into a transform
//! that some later op already consumed would retroactively move the geometry that
//! op was built on — the H5-B failure mode, one layer up.
//!
//! `outputs` is regen-derived provenance (`DocumentSession::sync_record_outputs`)
//! and is empty until a regen has run; the `inputs` half of the test is therefore
//! the load-bearing one for a freshly loaded document, and `outputs` sharpens it
//! for bodies a record PRODUCED but does not list as an input (an extrude's own
//! body, a `copy: true` transform's copies).

use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::document::Document;
use crate::ids::{BodyId, RecordId};

/// `Some(record)` iff the next placement gesture on `body` may **fold into** that
/// existing `TransformBody` record instead of appending a new one. See the module
/// docs for the exact lineage rule.
#[must_use]
pub fn can_fold_transform(doc: &Document, body: BodyId) -> Option<RecordId> {
    let records = doc.timeline.records();
    let applied = doc.timeline.cursor().min(records.len());
    let last = records[..applied]
        .iter()
        .rev()
        .find(|rec| !rec.suppressed && touches_body(rec, body))?;
    match &last.op {
        Operation::Known(KnownOperation::TransformBody(p))
            if !p.copy && p.targets.contains(&body) =>
        {
            Some(last.record_id)
        }
        _ => None,
    }
}

/// Whether `rec` reads or produces `body` (its derived inputs or its regen-derived
/// `outputs`).
fn touches_body(rec: &OperationRecord, body: BodyId) -> bool {
    rec.inputs.bodies.contains(&body) || rec.outputs.contains(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{FilletParams, TransformBodyParams, TransformRotation};
    use crate::document::variables::Scalar;
    use crate::ids::{DocumentId, ElementId};
    use uuid::Uuid;

    fn body(n: u128) -> BodyId {
        BodyId(Uuid::from_u128(n))
    }
    fn rec_id(n: u128) -> RecordId {
        RecordId(Uuid::from_u128(n))
    }

    fn transform_op(targets: Vec<BodyId>, copy: bool) -> Operation {
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets,
            translate: [Scalar::new(10.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: TransformRotation::default(),
            copy,
            extra: Default::default(),
        }))
    }

    fn fillet_op() -> Operation {
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(1.0),
            edge_ids: vec![ElementId::new("el_1")],
            edges: vec![],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        }))
    }

    /// Builds a document whose timeline is `ops` (all applied), stamping each
    /// record's `outputs` from `outputs[i]`.
    fn doc_with(ops: Vec<(Operation, Vec<BodyId>)>) -> Document {
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        for (i, (op, outs)) in ops.into_iter().enumerate() {
            let mut rec = OperationRecord::new(rec_id(0x100 + i as u128), i as u32, "op", op);
            rec.outputs = outs;
            doc.timeline.insert_at_cursor(rec);
        }
        doc
    }

    #[test]
    fn last_touching_record_is_a_move_of_this_body() {
        let b = body(1);
        let doc = doc_with(vec![(transform_op(vec![b], false), vec![b])]);
        assert_eq!(can_fold_transform(&doc, b), Some(rec_id(0x100)));
    }

    #[test]
    fn a_later_op_on_the_same_body_blocks_the_fold() {
        let b = body(1);
        let mut doc = doc_with(vec![(transform_op(vec![b], false), vec![b])]);
        // Fillet AFTER the transform, standing on the moved body.
        let mut fillet = OperationRecord::new(rec_id(0x200), 1, "Fillet", fillet_op());
        fillet.outputs = vec![b];
        doc.timeline.insert_at_cursor(fillet);
        assert_eq!(
            can_fold_transform(&doc, b),
            None,
            "the fillet is later in the lineage — folding would move geometry it stands on"
        );
    }

    #[test]
    fn copy_transform_never_folds_its_source() {
        let b = body(1);
        let doc = doc_with(vec![(transform_op(vec![b], true), vec![body(9)])]);
        assert_eq!(can_fold_transform(&doc, b), None);
    }

    #[test]
    fn untouched_body_and_empty_timeline_fold_nowhere() {
        let doc = doc_with(vec![(transform_op(vec![body(1)], false), vec![body(1)])]);
        assert_eq!(can_fold_transform(&doc, body(7)), None);
        let blank = Document::new(DocumentId(Uuid::from_u128(2)));
        assert_eq!(can_fold_transform(&blank, body(1)), None);
    }

    #[test]
    fn a_multi_target_move_folds_for_every_target() {
        let (a, b) = (body(1), body(2));
        let doc = doc_with(vec![(transform_op(vec![a, b], false), vec![a, b])]);
        assert_eq!(can_fold_transform(&doc, a), Some(rec_id(0x100)));
        assert_eq!(can_fold_transform(&doc, b), Some(rec_id(0x100)));
    }

    #[test]
    fn suppressed_and_rolled_back_records_are_not_the_last_toucher() {
        let b = body(1);
        let mut doc = doc_with(vec![(transform_op(vec![b], false), vec![b])]);
        let mut fillet = OperationRecord::new(rec_id(0x200), 1, "Fillet", fillet_op());
        fillet.outputs = vec![b];
        fillet.suppressed = true;
        doc.timeline.insert_at_cursor(fillet);
        assert_eq!(
            can_fold_transform(&doc, b),
            Some(rec_id(0x100)),
            "a suppressed fillet never executes ⇒ the transform is still last"
        );
        // Roll the cursor back BEFORE the transform: nothing applied touches it.
        doc.timeline.set_cursor(0);
        assert_eq!(can_fold_transform(&doc, b), None);
    }
}
