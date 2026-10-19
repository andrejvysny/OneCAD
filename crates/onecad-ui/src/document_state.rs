//! Lifecycle state remains available when scene admission or renderer preparation fails.
use onecad_app::dto::{DocumentProjection, DocumentState};

#[derive(Default)]
pub(crate) struct Slot {
    serial: u64,
    state: Option<DocumentState>,
}

impl Slot {
    pub(crate) fn publish(&mut self, mut state: DocumentState) {
        truncate(&mut state.title, 4096);
        if let Some(path) = &mut state.path {
            truncate(path, 32768);
        }
        if self.state.as_ref().is_some_and(|current| {
            current.runtime_session == state.runtime_session && current.revision > state.revision
        }) {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        self.state = Some(state);
    }

    pub(crate) fn observe(&mut self, projection: &DocumentProjection) {
        // Only a locked application snapshot can authorize replacing an existing session.
        if self.state.as_ref().is_some_and(|state| {
            state.runtime_session != projection.runtime_session
                || projection.revision <= state.revision
        }) {
            return;
        }
        let path = self.state.as_ref().and_then(|state| {
            (state.runtime_session == projection.runtime_session)
                .then(|| state.path.clone())
                .flatten()
        });
        self.publish(DocumentState {
            document_id: projection.document_id.clone(),
            runtime_session: projection.runtime_session.clone(),
            revision: projection.revision,
            dirty: projection.dirty,
            title: bounded(&projection.title, 4096),
            path,
        });
    }

    pub(crate) fn latest(&self) -> Option<(u64, DocumentState)> {
        self.state.clone().map(|state| (self.serial, state))
    }
}

fn bounded(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn truncate(value: &mut String, limit: usize) {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

pub(crate) async fn refresh(backend: &crate::bridge::Backend) {
    match onecad_app::api::get_document_state(&backend.state).await {
        Ok(state) => {
            if let Ok(mut slot) = backend.shared.document_state.lock() {
                slot.publish(state);
            }
            backend.shared.wake();
        }
        Err(error) => crate::services::invalidate(
            &backend.shared,
            format!("Document state unavailable: {error}"),
        ),
    }
}

impl crate::app::App {
    pub(crate) fn sync_document_state(&mut self) {
        let latest = self
            .bridge
            .shared
            .document_state
            .lock()
            .ok()
            .and_then(|slot| slot.latest());
        if let Some((serial, state)) = latest
            && serial != self.document_state_serial
        {
            self.document_state_serial = serial;
            self.diagnostics
                .runtime(&state.runtime_session, &self.status);
            self.document_state = Some(state);
            self.authoring_dirty = false;
        }
    }

    pub(crate) fn document_dirty(&self) -> bool {
        self.authoring_dirty
            || self
                .document_state
                .as_ref()
                .is_some_and(|state| state.dirty)
    }

    pub(crate) fn document_open(&self) -> bool {
        self.document_state
            .as_ref()
            .is_some_and(|state| !state.runtime_session.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_slot_bounds_snapshot_strings_on_utf8_boundaries() {
        let mut slot = Slot::default();
        slot.publish(DocumentState {
            title: "界".repeat(2000),
            path: Some("界".repeat(12000)),
            ..Default::default()
        });
        let (_, state) = slot.latest().unwrap();
        assert_eq!(state.title.len(), 4095);
        assert_eq!(state.path.unwrap().len(), 32766);
    }

    #[test]
    fn projection_updates_preserve_same_session_path_and_bound_title() {
        let mut slot = Slot::default();
        slot.publish(DocumentState {
            runtime_session: "saved".into(),
            path: Some("/saved.onecad".into()),
            revision: 8,
            ..Default::default()
        });
        let mut projection = DocumentProjection::empty();
        projection.runtime_session = "saved".into();
        projection.revision = 9;
        projection.dirty = true;
        projection.title = "🛠".repeat(4096);
        slot.observe(&projection);
        let (_, state) = slot.latest().unwrap();
        assert_eq!(state.path.as_deref(), Some("/saved.onecad"));
        assert!(state.dirty);
        assert_eq!(state.title.len(), 4096);
        projection.runtime_session = "replacement".into();
        slot.observe(&projection);
        assert_eq!(slot.latest().unwrap().1.runtime_session, "saved");
        slot.publish(DocumentState {
            runtime_session: "replacement".into(),
            ..Default::default()
        });
        slot.observe(&projection);
        assert!(slot.latest().unwrap().1.path.is_none());
    }

    #[test]
    fn stale_projection_cannot_restore_presave_or_replaced_metadata() {
        let mut slot = Slot::default();
        slot.publish(DocumentState {
            runtime_session: "current".into(),
            revision: 9,
            dirty: false,
            title: "Saved".into(),
            path: Some("/Saved.onecad".into()),
            ..Default::default()
        });
        let mut stale = DocumentProjection::empty();
        stale.runtime_session = "current".into();
        stale.revision = 9;
        stale.dirty = true;
        stale.title = "Before save".into();
        slot.observe(&stale);
        stale.runtime_session = "retired".into();
        stale.revision = 100;
        slot.observe(&stale);
        let (_, state) = slot.latest().unwrap();
        assert_eq!(state.runtime_session, "current");
        assert_eq!(state.title, "Saved");
        assert!(!state.dirty);
        assert_eq!(state.path.as_deref(), Some("/Saved.onecad"));
    }

    #[test]
    fn latest_slot_survives_capture_failure_and_rejects_old_same_session_state() {
        let mut slot = Slot::default();
        slot.publish(DocumentState {
            runtime_session: "recovered".into(),
            revision: 9,
            dirty: true,
            title: "Recovered".into(),
            ..Default::default()
        });
        let mut oversized = DocumentProjection::empty();
        oversized.title = "x".repeat(2 << 20);
        assert!(crate::projection_budget::check(&oversized).is_err());
        slot.publish(DocumentState {
            runtime_session: "recovered".into(),
            revision: 8,
            ..Default::default()
        });
        let (_, state) = slot.latest().unwrap();
        assert_eq!(state.revision, 9);
        assert!(state.dirty);
        assert_eq!(state.title, "Recovered");
    }
}
