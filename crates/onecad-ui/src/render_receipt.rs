//! Acknowledgements describe completed GPU work for one captured publication, not presentation.
use onecad_app::host::DocumentChangedEvent;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct RenderReceipt {
    expectation_id: u64,
    document_id: String,
    runtime_session: String,
    revision: u64,
    snapshot_id: u64,
    bodies: Vec<String>,
    queued: Arc<AtomicBool>,
}

impl RenderReceipt {
    pub fn from_event(event: DocumentChangedEvent) -> Option<Self> {
        Some(Self {
            expectation_id: event.render_expectation_id?,
            document_id: event.change.document_id,
            runtime_session: event.change.runtime_session,
            revision: event.change.revision,
            snapshot_id: event.change.snapshot_id,
            bodies: event
                .change
                .changed_bodies
                .into_iter()
                .map(|body| body.body_id)
                .collect(),
            queued: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn capture(
        &self,
        document_id: &str,
        runtime_session: &str,
        revision: u64,
        snapshot_id: u64,
        scene_bodies: impl IntoIterator<Item = String>,
    ) -> Option<Self> {
        if self.document_id != document_id
            || self.runtime_session != runtime_session
            || self.revision != revision
            || self.snapshot_id != snapshot_id
        {
            return None;
        }
        let mut captured = self.clone();
        captured.bodies = scene_bodies
            .into_iter()
            .filter(|body| self.bodies.contains(body))
            .collect();
        (!captured.bodies.is_empty()).then_some(captured)
    }

    fn claim(&self) -> bool {
        self.queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn submitted(&self, queue: &wgpu::Queue) {
        if !self.claim() {
            return;
        }
        let receipt = self.clone();
        queue.on_submitted_work_done(move || {
            for body in receipt.bodies {
                onecad_app::api::mesh_render_completed(
                    receipt.expectation_id,
                    receipt.document_id.clone(),
                    receipt.revision,
                    receipt.snapshot_id,
                    body,
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> RenderReceipt {
        RenderReceipt {
            expectation_id: 23,
            document_id: "document".into(),
            runtime_session: "session".into(),
            revision: 4,
            snapshot_id: 8,
            bodies: vec!["body-a".into(), "body-b".into()],
            queued: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn capture_requires_every_publication_identity_component() {
        let receipt = receipt();
        for (document, session, revision, snapshot) in [
            ("other", "session", 4, 8),
            ("document", "other", 4, 8),
            ("document", "session", 5, 8),
            ("document", "session", 4, 9),
        ] {
            assert!(
                receipt
                    .capture(document, session, revision, snapshot, ["body-a".into()])
                    .is_none()
            );
        }
        assert!(
            receipt
                .capture("document", "session", 4, 8, ["body-a".into()])
                .is_some()
        );
    }

    #[test]
    fn capture_never_acknowledges_missing_or_unpublished_bodies() {
        let receipt = receipt();
        let captured = receipt
            .capture(
                "document",
                "session",
                4,
                8,
                ["body-b".into(), "unpublished".into()],
            )
            .unwrap();
        assert_eq!(captured.bodies, ["body-b"]);
        assert!(
            receipt
                .capture("document", "session", 4, 8, ["unpublished".into()])
                .is_none()
        );
    }

    #[test]
    fn repeated_scene_captures_share_exactly_one_submission_claim() {
        let receipt = receipt();
        let first = receipt
            .capture("document", "session", 4, 8, ["body-a".into()])
            .unwrap();
        let second = receipt
            .capture("document", "session", 4, 8, ["body-a".into()])
            .unwrap();
        assert!(!receipt.queued.load(Ordering::Acquire));
        assert!(first.claim());
        assert!(!first.claim());
        assert!(!second.claim());
        assert!(!receipt.claim());
    }
}
