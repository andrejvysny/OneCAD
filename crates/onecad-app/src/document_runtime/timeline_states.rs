//! Preserve only verdicts that a mutation cannot have invalidated.

use onecad_core::history::{StepState, Timeline};

pub(super) fn retain_prefix_states(
    previous: &Timeline,
    next: &mut Timeline,
    dirty_floor: Option<usize>,
) {
    let end = previous
        .cursor()
        .min(next.cursor())
        .min(dirty_floor.unwrap_or(usize::MAX));
    for step in 0..end {
        let old = &previous.records()[step];
        let new = &next.records()[step];
        // A mismatched prefix cannot certify any later verdict, even if a caller
        // accidentally supplied a dirty floor beyond the changed record.
        if old.record_id != new.record_id || old.suppressed != new.suppressed {
            break;
        }
        // Effective lowering already stamped variable/pattern failures and
        // suppression. An old verdict must never overwrite that new evidence.
        if next.state(step) == Some(&StepState::Dirty) {
            if let Some(state) = previous.state(step) {
                let _ = next.mark_state(step, state.clone());
            }
        }
    }
}

pub(super) fn earliest_dirty_floor(first: Option<usize>, next: Option<usize>) -> Option<usize> {
    match (first, next) {
        (Some(first), Some(next)) => Some(first.min(next)),
        (Some(floor), None) | (None, Some(floor)) => Some(floor),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::document::record::{Operation, OperationRecord};
    use onecad_core::ids::RecordId;
    use uuid::Uuid;

    fn timeline(ids: &[u128]) -> Timeline {
        Timeline::from_records(
            ids.iter()
                .enumerate()
                .map(|(step, id)| {
                    let op: Operation = serde_json::from_value(serde_json::json!({
                        "opType": "FutureShape", "params": {}
                    }))
                    .unwrap();
                    OperationRecord::new(RecordId(Uuid::from_u128(*id)), step as u32, "Op", op)
                })
                .collect(),
        )
    }

    fn valid(ids: &[u128]) -> Timeline {
        let mut timeline = timeline(ids);
        for step in 0..ids.len() {
            timeline.mark_state(step, StepState::Valid).unwrap();
        }
        timeline
    }

    #[test]
    fn retains_actual_prefix_verdicts_without_certifying_the_suffix() {
        let mut old = valid(&[1, 2, 3, 4]);
        old.mark_state(1, StepState::NeedsRepair).unwrap();
        old.mark_state(
            2,
            StepState::Error {
                reason: "failed".into(),
            },
        )
        .unwrap();
        old.mark_state(3, StepState::Dirty).unwrap();
        let mut next = timeline(&[1, 2, 3, 4, 5]);
        retain_prefix_states(&old, &mut next, Some(4));
        for step in 0..4 {
            assert_eq!(next.state(step), old.state(step));
        }
        assert_eq!(next.state(4), Some(&StepState::Dirty));
    }

    #[test]
    fn keeps_new_lowering_errors_and_refuses_mismatched_identity() {
        let old = valid(&[1, 2, 3]);
        let mut next = timeline(&[1, 2, 3]);
        let error = StepState::Error {
            reason: "missing variable".into(),
        };
        next.mark_state(0, error.clone()).unwrap();
        next.mark_state(1, StepState::NeedsRepair).unwrap();
        retain_prefix_states(&old, &mut next, None);
        assert_eq!(next.state(0), Some(&error));
        assert_eq!(next.state(1), Some(&StepState::NeedsRepair));
        assert_eq!(next.state(2), Some(&StepState::Valid));
        let mut mismatch = timeline(&[9, 2, 3]);
        retain_prefix_states(&old, &mut mismatch, None);
        assert!((0..3).all(|step| mismatch.state(step) == Some(&StepState::Dirty)));
    }

    #[test]
    fn respects_dirty_floor_and_both_applied_cursors() {
        let mut old = valid(&[1, 2, 3]);
        old.set_cursor(2);
        let mut next = timeline(&[1, 2, 3]);
        retain_prefix_states(&old, &mut next, None);
        assert_eq!(next.state(1), Some(&StepState::Valid));
        assert_eq!(next.state(2), Some(&StepState::Dirty));
        let mut next = timeline(&[1, 2, 3]);
        next.set_cursor(1);
        retain_prefix_states(&old, &mut next, None);
        assert_eq!(next.state(0), Some(&StepState::Valid));
        assert_eq!(next.state(1), Some(&StepState::Dirty));
        let mut next = timeline(&[1, 2, 3]);
        retain_prefix_states(&old, &mut next, Some(1));
        assert_eq!(next.state(1), Some(&StepState::Dirty));
    }

    #[test]
    fn suppression_changes_stop_retention_without_overwriting_new_flags() {
        let old = valid(&[1, 2, 3]);
        let mut records = old.records().to_vec();
        records[1].suppressed = true;
        let mut next = Timeline::from_records(records);
        retain_prefix_states(&old, &mut next, None);
        assert_eq!(next.state(0), Some(&StepState::Valid));
        assert_eq!(next.state(1), Some(&StepState::Suppressed));
        assert_eq!(next.state(2), Some(&StepState::Dirty));
        let mut unsuppressed = timeline(&[1, 2, 3]);
        retain_prefix_states(&next, &mut unsuppressed, None);
        assert_eq!(unsuppressed.state(1), Some(&StepState::Dirty));
    }

    #[test]
    fn multiple_undo_floors_keep_the_earliest_claim() {
        assert_eq!(earliest_dirty_floor(Some(5), Some(2)), Some(2));
        assert_eq!(earliest_dirty_floor(Some(2), Some(5)), Some(2));
        assert_eq!(earliest_dirty_floor(None, Some(2)), Some(2));
        assert_eq!(earliest_dirty_floor(Some(2), None), Some(2));
        assert_eq!(earliest_dirty_floor(None, None), None);
    }
}
