//! Authoring receipts and geometry completion remain distinct, including post-record failures.
use onecad_app::{api::CommandCompletion, dto::RegenFinished};
use std::future::Future;

pub async fn finish<T: serde::Serialize>(
    receipt: CommandCompletion<T>,
    wait: impl Future<Output = Result<RegenFinished, String>>,
) -> Result<T, String> {
    let original = receipt.result.as_ref().err().map(ToString::to_string);
    if receipt.regen_expected {
        let completion = wait.await.map_err(|error| {
            format!(
                "Sketch authoring may be accepted; completion unavailable: {error}{}",
                suffix(original.as_deref())
            )
        })?;
        if let Some(error) = completion_error(&completion) {
            return Err(format!(
                "Sketch authoring retained; {error}{}",
                suffix(original.as_deref())
            ));
        }
    }
    if let Ok(result) = &receipt.result {
        crate::record_budget::check_payload(result)?;
    }
    receipt.result.map_err(|error| {
        format!("Sketch service error; authoring was not automatically reverted: {error}")
    })
}
fn suffix(original: Option<&str>) -> String {
    original
        .map(|error| format!("; {error}"))
        .unwrap_or_default()
}
fn bounded(text: &str) -> String {
    text.chars().take(2048).collect()
}
pub(crate) fn completion_error(completion: &RegenFinished) -> Option<String> {
    let mut errors = Vec::new();
    if !matches!(completion.outcome.as_str(), "published" | "noop") {
        errors.push(format!(
            "regeneration {}: {}",
            bounded(&completion.outcome),
            bounded(completion.message.as_deref().unwrap_or_default())
        ));
    }
    if let Some(failure) = completion.failed_steps.first() {
        errors.push(format!(
            "{} failed feature(s); {}: {}",
            completion.failed_steps.len(),
            bounded(&failure.record_id),
            bounded(&failure.message)
        ));
    }
    if !completion.repair_steps.is_empty() {
        errors.push(format!(
            "{} feature(s) need repair: {}",
            completion.repair_steps.len(),
            completion
                .repair_steps
                .iter()
                .take(4)
                .map(|id| bounded(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_app::{
        dto::{DocumentProjection, FailedStep},
        error::ApiError,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    fn receipt(result: Result<u32, ApiError>, regen_expected: bool) -> CommandCompletion<u32> {
        CommandCompletion {
            result,
            projection: DocumentProjection::empty(),
            regen_expected,
        }
    }
    fn completion() -> RegenFinished {
        RegenFinished {
            runtime_session: "test".into(),
            revision: 3,
            source_revision: 3,
            outcome: "published".into(),
            message: None,
            diagnostics: vec![],
            failed_steps: vec![],
            repair_steps: vec![],
            affected_bodies: Default::default(),
        }
    }
    #[tokio::test]
    async fn no_regen_receipt_never_waits_and_post_record_error_still_waits() {
        assert_eq!(
            finish(receipt(Ok(7), false), std::future::pending())
                .await
                .unwrap(),
            7
        );
        let waited = AtomicBool::new(false);
        let error = finish(
            receipt(
                Err(ApiError::InvalidCommand("regions refusal".into())),
                true,
            ),
            async {
                waited.store(true, Ordering::Release);
                Ok(completion())
            },
        )
        .await
        .unwrap_err();
        assert!(waited.load(Ordering::Acquire));
        assert!(error.contains("regions refusal") && error.contains("not automatically reverted"));
    }
    #[tokio::test]
    async fn dependent_failure_preserves_exact_reason_repair_ids_and_original_error() {
        let mut event = completion();
        event.failed_steps.push(FailedStep {
            record_id: "fillet-1".into(),
            message: "radius exceeds face".into(),
            diagnostics: vec![],
        });
        event.repair_steps.push("shell-2".into());
        let error = finish(
            receipt(
                Err(ApiError::InvalidCommand("regions refusal".into())),
                true,
            ),
            async { Ok(event) },
        )
        .await
        .unwrap_err();
        for text in [
            "fillet-1",
            "radius exceeds face",
            "shell-2",
            "regions refusal",
            "authoring retained",
        ] {
            assert!(error.contains(text), "{error}");
        }
    }
    #[tokio::test]
    async fn unavailable_completion_preserves_authoring_uncertainty_without_retry() {
        let error = finish(receipt(Ok(7), true), async { Err("worker retired".into()) })
            .await
            .unwrap_err();
        assert!(error.contains("may be accepted") && error.contains("worker retired"));
    }
}
