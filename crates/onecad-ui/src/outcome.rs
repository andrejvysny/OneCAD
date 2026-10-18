use onecad_app::dto::RegenFinished;

#[derive(Default, Debug)]
pub struct CommandOutcome {
    pub completion: Option<Box<RegenFinished>>,
    pub rollback: Option<RollbackOutcome>,
}
#[derive(Debug)]
pub enum RollbackOutcome {
    RolledBack {
        completion: Option<Box<RegenFinished>>,
    },
    Refused(String),
    Failed(String),
    AppliedCompletionUnavailable(String),
}
impl RollbackOutcome {
    pub fn explanation(&self) -> String {
        match self {
            Self::RolledBack { completion } => {
                if let Some(result) = completion {
                    if let Some(failure) = result.failed_steps.first() {
                        return format!(
                            "New operation rolled back; restored geometry contains failed feature: {}",
                            failure.message
                        );
                    }
                    if !result.repair_steps.is_empty() {
                        return "New operation rolled back; restored geometry needs repair".into();
                    }
                    if !matches!(result.outcome.as_str(), "published" | "noop") {
                        return format!(
                            "New operation rolled back; rollback regeneration {}: {}",
                            result.outcome,
                            result.message.as_deref().unwrap_or("no detail provided")
                        );
                    }
                }
                "New provisional operation rolled back".into()
            }
            Self::Refused(reason) => {
                format!("Rollback refused: {reason}; authoring history preserved")
            }
            Self::Failed(reason) => format!("Rollback failed: {reason}"),
            Self::AppliedCompletionUnavailable(reason) => {
                format!("New operation rolled back; geometry completion unavailable: {reason}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_partial_failure_and_repair_remain_explicit() {
        let mut completion = RegenFinished {
            runtime_session: "current".into(),
            revision: 4,
            source_revision: 3,
            outcome: "published".into(),
            message: None,
            diagnostics: vec![],
            failed_steps: vec![onecad_app::dto::FailedStep {
                record_id: "older".into(),
                message: "restored feature failed".into(),
                diagnostics: vec![],
            }],
            repair_steps: vec![],
            affected_bodies: Default::default(),
        };
        let failed = RollbackOutcome::RolledBack {
            completion: Some(Box::new(completion.clone())),
        };
        assert!(failed.explanation().contains("restored feature failed"));
        completion.failed_steps.clear();
        completion.repair_steps.push("repair".into());
        let repair = RollbackOutcome::RolledBack {
            completion: Some(Box::new(completion)),
        };
        assert!(repair.explanation().contains("needs repair"));
    }
}
