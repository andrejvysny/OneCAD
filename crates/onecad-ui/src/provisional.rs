//! Only a failed fresh provisional operation may consume its backend rollback receipt.
use crate::{
    bridge::{Backend, Message},
    event_mailbox::Expected,
    outcome::{CommandOutcome, RollbackOutcome},
    services,
};
use onecad_app::{
    api::{self, OperationEffects},
    dto::RegenFinished,
};
use onecad_core::{edit::EditCommand, ids::RecordId};
use onecad_interaction::RequestTag;
use std::sync::mpsc;

pub async fn apply(
    backend: &mut Backend,
    tag: RequestTag,
    command: EditCommand,
    hide_sketch: Option<String>,
    sender: &mpsc::Sender<Message>,
) -> Result<CommandOutcome, String> {
    services::check_tag(&backend.state, &tag).await?;
    let fresh = match &command {
        EditCommand::AddOperation { record, .. } => Some(record.record_id),
        _ => None,
    };
    let result = api::apply_operation(
        &backend.state,
        backend.host.clone(),
        vec![command],
        Some(OperationEffects {
            hide_sketch_ids: hide_sketch.into_iter().collect(),
        }),
    )
    .await
    .map_err(|e| e.to_string())?;
    let revision = result.projection.revision;
    let session = result.projection.runtime_session.clone();
    let receipt = result.rollback_token.clone();
    let _ = sender.send(Message::Accepted {
        tag,
        revision,
        rollback_token: result.rollback_token,
    });
    backend.shared.wake();
    backend.events.retarget(&session, revision);
    let completion = services::wait_regen(backend, &session, revision).await?;
    let rollback = if should_rollback(fresh, &session, revision, &completion) {
        Some(rollback(backend, receipt, &session).await)
    } else {
        None
    };
    Ok(CommandOutcome {
        completion: Some(Box::new(completion)),
        rollback,
    })
}

async fn rollback(
    backend: &mut Backend,
    receipt: Option<String>,
    session: &str,
) -> RollbackOutcome {
    let Some(receipt) = receipt else {
        return RollbackOutcome::Refused("backend returned no receipt".into());
    };
    let before = match api::get_projection(&backend.state).await {
        Ok(projection) => projection,
        Err(error) => return RollbackOutcome::Failed(error.to_string()),
    };
    backend.events.begin(Expected::Known {
        session: session.into(),
        revision: before.revision + 1,
    });
    let response =
        match api::rollback_failed_operation(&backend.state, backend.host.clone(), receipt).await {
            Ok(response) => response,
            Err(error) => return RollbackOutcome::Failed(error.to_string()),
        };
    if !response.rolled_back {
        return RollbackOutcome::Refused(response.reason.into());
    }
    if !response.regen_expected {
        return RollbackOutcome::RolledBack { completion: None };
    }
    backend.events.retarget(
        &response.projection.runtime_session,
        response.projection.revision,
    );
    match services::wait_regen(
        backend,
        &response.projection.runtime_session,
        response.projection.revision,
    )
    .await
    {
        Ok(completion) => RollbackOutcome::RolledBack {
            completion: Some(Box::new(completion)),
        },
        Err(error) => RollbackOutcome::AppliedCompletionUnavailable(error),
    }
}

fn should_rollback(
    fresh: Option<RecordId>,
    session: &str,
    accepted_revision: u64,
    completion: &RegenFinished,
) -> bool {
    let Some(record) = fresh else {
        return false;
    };
    if completion.runtime_session != session
        || completion.source_revision != accepted_revision
        || completion.outcome == "superseded"
        || completion
            .repair_steps
            .iter()
            .any(|id| *id == record.to_string())
    {
        return false;
    }
    matches!(completion.outcome.as_str(), "failed" | "cancelled")
        || completion
            .failed_steps
            .iter()
            .any(|step| step.record_id == record.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn completion(outcome: &str) -> RegenFinished {
        RegenFinished {
            runtime_session: "current".into(),
            revision: 10,
            source_revision: 9,
            outcome: outcome.into(),
            message: Some("original failure".into()),
            diagnostics: vec![],
            failed_steps: vec![],
            affected_bodies: Default::default(),
            repair_steps: vec![],
        }
    }
    #[test]
    fn only_fresh_matching_authoritative_failures_allow_receipt_rollback() {
        let record = RecordId::new();
        assert!(should_rollback(
            Some(record),
            "current",
            9,
            &completion("failed")
        ));
        assert!(should_rollback(
            Some(record),
            "current",
            9,
            &completion("cancelled")
        ));
        assert!(!should_rollback(None, "current", 9, &completion("failed")));
        assert!(!should_rollback(
            Some(record),
            "old",
            9,
            &completion("failed")
        ));
        assert!(!should_rollback(
            Some(record),
            "current",
            8,
            &completion("failed")
        ));
        assert!(!should_rollback(
            Some(record),
            "current",
            11,
            &completion("failed")
        ));
        assert!(!should_rollback(
            Some(record),
            "current",
            9,
            &completion("superseded")
        ));
        assert!(!should_rollback(
            Some(record),
            "current",
            9,
            &completion("published")
        ));
    }
    #[test]
    fn repair_is_retained_and_sibling_failure_does_not_remove_fresh_success() {
        let record = RecordId::new();
        let mut result = completion("published");
        result.repair_steps.push(record.to_string());
        assert!(!should_rollback(Some(record), "current", 9, &result));
        result.repair_steps.clear();
        result.failed_steps.push(onecad_app::dto::FailedStep {
            record_id: RecordId::new().to_string(),
            message: "sibling failed".into(),
            diagnostics: vec![],
        });
        assert!(!should_rollback(Some(record), "current", 9, &result));
        result.failed_steps[0].record_id = record.to_string();
        assert!(should_rollback(Some(record), "current", 9, &result));
    }
}
