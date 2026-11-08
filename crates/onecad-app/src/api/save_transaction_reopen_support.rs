use super::*;
use crate::{
    dto::RegenFinished,
    host::{AppEvent, EventSink},
    worker::{wire::body_id_wire, ElementQuery},
};
use onecad_core::{
    ids::{BodyId, DocumentRevision, ElementId, SnapshotId},
    regen::GeometryEngine,
};
use tokio::sync::mpsc;

pub(super) struct RegenEvents(mpsc::UnboundedSender<RegenFinished>);

impl EventSink for RegenEvents {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        if let AppEvent::RegenFinished(report) = event {
            let _ = self.0.send(report);
        }
        Ok(())
    }
}

pub(super) fn capture(host: &mut HostContext) -> mpsc::UnboundedReceiver<RegenFinished> {
    let (sender, receiver) = mpsc::unbounded_channel();
    host.events = Arc::new(RegenEvents(sender));
    receiver
}

async fn published(events: &mut mpsc::UnboundedReceiver<RegenFinished>, session: &str) {
    let report = loop {
        let report = events.recv().await.expect("regen event channel closed");
        if report.runtime_session == session && report.source_revision == 0 {
            break report;
        }
    };
    assert_eq!(report.outcome, "published", "{report:?}");
    assert!(report.failed_steps.is_empty(), "{report:?}");
    assert!(report.repair_steps.is_empty(), "{report:?}");
}

async fn head(state: &AppState, session: &str) -> SnapshotId {
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(runtime.runtime_session(), session);
    assert_eq!(runtime.revision(), DocumentRevision(0));
    assert!(
        runtime.persisted_element_count() > 0,
        "reopened runtime must contain promoted evidence"
    );
    let snapshot = runtime.head_snapshot_id().expect("published snapshot");
    assert!(snapshot.0 > 0, "published snapshot must be positive");
    snapshot
}

pub(super) async fn scheduled_bound(
    state: &AppState,
    events: &mut mpsc::UnboundedReceiver<RegenFinished>,
    session: &str,
    body: BodyId,
    element: &ElementId,
) -> SnapshotId {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        published(events, session).await;
        let snapshot = head(state, session).await;
        let worker = state.live_worker().expect("reopened worker");
        let epoch = worker.current_epoch();
        loop {
            let resolved = worker
                .query_element(snapshot, body, element.as_str())
                .await
                .expect("QueryElement failed");
            if let Some(info) = resolved {
                assert_eq!(info.element_id, element.as_str());
                assert_eq!(info.body_id, body_id_wire(body));
                assert_eq!(info.kind, "face");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(worker.current_epoch(), epoch);
        assert_eq!(head(state, session).await, snapshot);
        snapshot
    })
    .await
    .expect("scheduled publish and persisted-element binding did not finish within 15 seconds")
}
