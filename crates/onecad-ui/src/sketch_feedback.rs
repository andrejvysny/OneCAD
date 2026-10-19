//! Present solver evidence without deriving degrees of freedom in the host.
use onecad_app::dto::{EntityConstrainedState, SketchSolveStatus, SketchTruthDto};
use onecad_core::ids::EntityId;
use onecad_interaction::sketch::SketchStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Assessment {
    Healthy,
    UnderConstrained,
    Conflicting,
    Unavailable,
}

pub(crate) struct Feedback {
    pub label: String,
    pub assessment: Assessment,
}

fn assessed(status: SketchSolveStatus, dof: u32, truth: &SketchTruthDto) -> Feedback {
    let (label, assessment) = if !truth.unassessed.is_empty() {
        (
            "Solver assessment incomplete".into(),
            Assessment::Unavailable,
        )
    } else if !truth.violations.is_empty() || !truth.reasons.is_empty() {
        ("Conflicting constraints".into(), Assessment::Conflicting)
    } else {
        match status {
            SketchSolveStatus::FullyConstrained if dof == 0 => {
                ("Fully constrained".into(), Assessment::Healthy)
            }
            SketchSolveStatus::FullyConstrained => (
                "Inconsistent solver assessment".into(),
                Assessment::Unavailable,
            ),
            SketchSolveStatus::UnderConstrained => (
                format!("Underconstrained · {dof} DOF"),
                Assessment::UnderConstrained,
            ),
            SketchSolveStatus::OverConstrained => {
                ("Redundant constraints".into(), Assessment::Conflicting)
            }
            SketchSolveStatus::Conflicting => {
                ("Conflicting constraints".into(), Assessment::Conflicting)
            }
        }
    };
    Feedback { label, assessment }
}

pub(crate) fn summary(status: &SketchStatus) -> Feedback {
    let label = match status {
        SketchStatus::Entered(solve) if solve.solve_current => {
            return assessed(solve.status, solve.dof, &solve.truth);
        }
        SketchStatus::Entered(_) => "Solver assessment unavailable (cached)".into(),
        SketchStatus::Solved(solve) => return assessed(solve.status, solve.dof, &solve.truth),
        SketchStatus::Starting => "Opening sketch…".into(),
        SketchStatus::Pending | SketchStatus::Accepted => "Solving sketch…".into(),
        SketchStatus::Refused(message) => format!("Edit rejected: {message}"),
        SketchStatus::Failed { message, .. } => format!("Solver unavailable: {message}"),
        SketchStatus::Finished { .. } => "Sketch finished".into(),
        SketchStatus::Discarded(_) => "Sketch edits discarded".into(),
    };
    Feedback {
        label,
        assessment: Assessment::Unavailable,
    }
}

pub(crate) fn entity(status: &SketchStatus, id: EntityId) -> Option<EntityConstrainedState> {
    let states = match status {
        SketchStatus::Entered(solve)
            if solve.solve_current && solve.truth.unassessed.is_empty() =>
        {
            &solve.entity_states
        }
        SketchStatus::Solved(solve) if solve.truth.unassessed.is_empty() => &solve.entity_states,
        _ => return None,
    };
    states.get(&id.to_string()).copied()
}

pub(crate) fn color(assessment: Assessment, theme: &crate::theme::Theme) -> egui::Color32 {
    match assessment {
        Assessment::Healthy => theme.dof_ok,
        Assessment::UnderConstrained => theme.created_text,
        Assessment::Conflicting => theme.danger_strong,
        Assessment::Unavailable => theme.dof_neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_dof_does_not_override_missing_failed_or_conflicting_truth() {
        let mut truth = SketchTruthDto::default();
        assert_eq!(
            assessed(SketchSolveStatus::FullyConstrained, 0, &truth).assessment,
            Assessment::Healthy
        );
        assert_eq!(
            assessed(SketchSolveStatus::FullyConstrained, 1, &truth).assessment,
            Assessment::Unavailable
        );
        for status in [
            SketchSolveStatus::Conflicting,
            SketchSolveStatus::OverConstrained,
        ] {
            assert_eq!(
                assessed(status, 0, &truth).assessment,
                Assessment::Conflicting
            );
        }
        truth.unassessed.push("unreadable constraint".into());
        assert_eq!(
            assessed(SketchSolveStatus::FullyConstrained, 0, &truth).assessment,
            Assessment::Unavailable
        );
        assert_eq!(
            summary(&SketchStatus::Failed {
                accepted: None,
                message: "worker disconnected".into()
            })
            .assessment,
            Assessment::Unavailable
        );
    }
}
