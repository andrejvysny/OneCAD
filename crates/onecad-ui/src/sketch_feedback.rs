//! Present solver evidence without deriving degrees of freedom in the host.
use crate::diagnostics::State;
use onecad_app::dto::{EntityConstrainedState, FinishSketchDto, SketchSolveStatus, SketchTruthDto};
use onecad_core::ids::EntityId;
use onecad_core::regen::Severity;
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
        SketchStatus::Finished {
            kept_reason,
            result,
        } => finished(kept_reason.as_deref(), result).0,
        SketchStatus::Discarded(_) => "Sketch edits discarded".into(),
    };
    Feedback {
        label,
        assessment: Assessment::Unavailable,
    }
}

/// Label and diagnostics state of a closed Finish, shared by the status line and
/// the inspector. A refused profile still closes the sketch (the runtime commits
/// the record before deriving regions), so it arrives as a region-less result
/// carrying the profile-stage error; success advisories are warnings and never
/// mark a refusal.
pub(crate) fn finished(kept_reason: Option<&str>, result: &FinishSketchDto) -> (String, State) {
    let refused = || {
        if !result.regions.is_empty() {
            return None;
        }
        result
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error && d.stage.as_deref() == Some("profile"))
            .map(|d| format!("Sketch finished without a profile: {}", d.message))
    };
    match kept_reason.map(str::to_owned).or_else(refused) {
        Some(reason) => (reason, State::Warning),
        None => ("Sketch finished".into(), State::Succeeded),
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

    fn diagnostic(severity: Severity, message: &str) -> onecad_core::regen::Diagnostic {
        onecad_core::regen::Diagnostic {
            severity,
            code: "SKETCH_PROFILE".into(),
            message: message.into(),
            stage: Some("profile".into()),
            reason_code: None,
            evidence: None,
        }
    }

    fn finish(regions: usize, diagnostics: Vec<onecad_core::regen::Diagnostic>) -> FinishSketchDto {
        let region = |i: usize| onecad_app::dto::SketchRegionDto {
            region_id: format!("r{i}"),
            outer_loop: Vec::new(),
            holes: Vec::new(),
            preview_triangles: None,
        };
        FinishSketchDto {
            region_identity_version: 2,
            regions: (0..regions).map(region).collect(),
            diagnostics,
        }
    }

    #[test]
    fn refused_profile_finish_is_a_warning_and_advisories_are_not() {
        let advisory = diagnostic(Severity::Warning, "tiny edge omitted");
        let refusal = diagnostic(Severity::Error, "open profile");
        let refused = finish(0, vec![advisory.clone(), refusal]);
        let expected = "Sketch finished without a profile: open profile";
        assert_eq!(finished(None, &refused), (expected.into(), State::Warning));
        let status = SketchStatus::Finished {
            result: Box::new(refused),
            kept_reason: None,
        };
        assert_eq!(summary(&status).label, expected, "inspector agrees");

        let profiled = finish(1, vec![advisory.clone()]);
        let success = ("Sketch finished".to_owned(), State::Succeeded);
        assert_eq!(finished(None, &profiled), success);
        let open_path = finish(0, vec![advisory]);
        assert_eq!(finished(None, &open_path), success, "advisory only");
        assert_eq!(finished(None, &finish(0, Vec::new())), success);
        assert_eq!(
            finished(Some("kept"), &finish(0, Vec::new())),
            ("kept".into(), State::Warning)
        );
    }
}
