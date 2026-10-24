//! Recovery outcomes remain separate from explicit Save and other operation evidence.
#[cfg(test)]
use crate::diagnostics::Diagnostics;
use crate::diagnostics::{Source, State};
use onecad_app::{autosave::AutosaveEvent, dto::DocumentState};

#[derive(Clone, Copy)]
struct Success {
    attempt_id: u64,
    at_ms: u64,
    revision: u64,
}

#[derive(Default)]
pub(crate) struct Slot {
    latest: Option<AutosaveEvent>,
    success: Option<Success>,
    serial: u64,
    delivered: u64,
}

impl Slot {
    pub(crate) fn record(&mut self, mut event: AutosaveEvent) {
        let same_session = self.latest.as_ref().is_some_and(|latest| {
            latest.document_id == event.document_id
                && latest.runtime_session == event.runtime_session
        });
        let newer = self
            .latest
            .as_ref()
            .is_none_or(|latest| event.attempt_id > latest.attempt_id);
        if !same_session && !newer {
            return;
        }
        if !same_session {
            self.success = None;
        }
        let success = event.error.is_none()
            && self
                .success
                .is_none_or(|success| event.attempt_id > success.attempt_id);
        if success {
            self.success = Some(Success {
                attempt_id: event.attempt_id,
                at_ms: event.at_ms,
                revision: event.document_revision,
            });
        }
        if newer {
            bound(&mut event.document_id, 128);
            bound(&mut event.runtime_session, 128);
            bound(&mut event.path, 32768);
            if let Some(error) = &mut event.error {
                bound(error, 4096);
            }
            self.latest = Some(event);
        }
        if newer || success {
            self.serial = self.serial.wrapping_add(1);
        }
    }

    fn take_current(&mut self, current: Option<&DocumentState>) -> Option<(State, String)> {
        let current = current?;
        let event = self.latest.as_ref()?;
        if self.serial == self.delivered
            || event.document_id != current.document_id
            || event.runtime_session != current.runtime_session
        {
            return None;
        }
        self.delivered = self.serial;
        let last = self.success.map_or_else(
            || "No successful recovery checkpoint observed in this session.".into(),
            |success| {
                format!(
                    "Last successful checkpoint: {} UTC, revision {}.",
                    time(success.at_ms),
                    success.revision
                )
            },
        );
        Some(match &event.error {
            Some(error) => (
                State::Failed,
                format!("Recovery checkpoint failed: {error}\n{last}\nSave remains available."),
            ),
            None => (
                State::Succeeded,
                format!("Recovery checkpoint updated. {last}"),
            ),
        })
    }
}

pub(crate) fn poll(app: &mut crate::app::App) {
    let update = app
        .doc
        .bridge
        .shared
        .autosave_status
        .lock()
        .ok()
        .and_then(|mut slot| slot.take_current(app.doc.document_state.as_ref()));
    if let Some((state, message)) = update {
        app.doc.diagnostics.record(Source::Autosave, state, message);
    }
}

#[cfg(test)]
fn deliver(slot: &mut Slot, current: &DocumentState, diagnostics: &mut Diagnostics) {
    if let Some((state, message)) = slot.take_current(Some(current)) {
        diagnostics.record(Source::Autosave, state, message);
    }
}

fn bound(value: &mut String, bytes: usize) {
    let mut end = value.len().min(bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

fn time(at_ms: u64) -> String {
    let seconds = at_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
#[path = "autosave_status_tests.rs"]
mod tests;
