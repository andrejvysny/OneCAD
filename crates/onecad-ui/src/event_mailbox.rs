//! One admitted durable mutation owns the terminal reservation. Unawaited notifications
//! are state refreshes, so they coalesce; a reserved terminal is never overwritten.
use onecad_app::dto::RegenFinished;
use std::sync::Mutex;
use tokio::sync::Notify;

#[derive(Clone, Debug)]
pub enum BackendEvent {
    Regen(RegenFinished),
    WorkerFailed,
}
#[derive(Clone)]
pub enum Expected {
    Known { session: String, revision: u64 },
    Replacement { previous: String },
}
impl Expected {
    fn matches(&self, event: &BackendEvent) -> bool {
        match event {
            BackendEvent::WorkerFailed => true,
            BackendEvent::Regen(event) => match self {
                Self::Known { session, revision } => {
                    event.runtime_session == *session && event.source_revision >= *revision
                }
                Self::Replacement { .. } => false,
            },
        }
    }
}
#[derive(Default)]
struct Slots {
    expected: Option<Expected>,
    terminal: Option<BackendEvent>,
    background: Option<BackendEvent>,
    worker_failed: bool,
}
#[derive(Default)]
pub struct EventMailbox {
    slots: Mutex<Slots>,
    notify: Notify,
}
impl EventMailbox {
    pub fn begin(&self, expected: Expected) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.expected = Some(expected);
            slots.terminal = None;
        }
    }
    /// Open emits its authoritative projection before scheduling the initial regen.
    pub fn observe_projection(&self, session: &str, revision: u64) {
        if let Ok(mut slots) = self.slots.lock()
            && matches!(&slots.expected,Some(Expected::Replacement{previous}) if previous!=session)
        {
            slots.expected = Some(Expected::Known {
                session: session.into(),
                revision,
            });
        }
    }
    pub fn retarget(&self, session: &str, revision: u64) {
        if let Ok(mut slots) = self.slots.lock() {
            let expected = Expected::Known {
                session: session.into(),
                revision,
            };
            if slots
                .terminal
                .as_ref()
                .is_none_or(|event| !expected.matches(event))
            {
                let previous = slots.terminal.take();
                if slots
                    .background
                    .as_ref()
                    .is_some_and(|event| expected.matches(event))
                {
                    slots.terminal = slots.background.take();
                }
                if slots.background.is_none() {
                    slots.background = previous;
                }
            }
            slots.expected = Some(expected);
        }
    }
    pub fn finish(&self) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.expected = None;
            slots.terminal = None;
        }
    }
    pub fn emit(&self, event: BackendEvent) -> Result<(), String> {
        let mut slots = self.slots.lock().map_err(|_| "terminal mailbox poisoned")?;
        if matches!(event, BackendEvent::WorkerFailed) {
            slots.worker_failed = true;
            drop(slots);
            self.notify.notify_one();
            return Ok(());
        }
        if slots
            .expected
            .as_ref()
            .is_some_and(|expected| expected.matches(&event))
        {
            let superseded =
                matches!(&event,BackendEvent::Regen(value) if value.outcome=="superseded");
            if !superseded && slots.terminal.is_none() {
                slots.terminal = Some(event);
            } else {
                slots.background = Some(event);
            }
        } else {
            slots.background = Some(event);
        }
        drop(slots);
        self.notify.notify_one();
        Ok(())
    }
    pub async fn next(&self) -> BackendEvent {
        loop {
            let notified = self.notify.notified();
            if let Ok(mut slots) = self.slots.lock() {
                if let Some(event) = slots.terminal.take() {
                    return event;
                }
                if slots.worker_failed {
                    slots.worker_failed = false;
                    return BackendEvent::WorkerFailed;
                }
                if let Some(event) = slots.background.take() {
                    return event;
                }
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn terminal(session: &str, revision: u64, outcome: &str) -> BackendEvent {
        BackendEvent::Regen(RegenFinished {
            runtime_session: session.into(),
            revision,
            source_revision: revision,
            outcome: outcome.into(),
            message: None,
            diagnostics: vec![],
            failed_steps: vec![],
            affected_bodies: Default::default(),
            repair_steps: vec![],
        })
    }
    #[tokio::test]
    async fn reserved_completion_survives_background_storm_and_later_failure() {
        let mailbox = EventMailbox::default();
        mailbox.begin(Expected::Known {
            session: "current".into(),
            revision: 9,
        });
        for i in 0..1000 {
            mailbox.emit(terminal("old", i, "published")).unwrap();
        }
        mailbox.emit(terminal("current", 9, "published")).unwrap();
        mailbox.emit(BackendEvent::WorkerFailed).unwrap();
        for i in 0..1000 {
            mailbox.emit(terminal("old", i, "published")).unwrap();
        }
        let BackendEvent::Regen(event) = mailbox.next().await else {
            panic!("reserved completion lost")
        };
        assert_eq!(
            (event.runtime_session.as_str(), event.source_revision),
            ("current", 9)
        );
        assert!(matches!(mailbox.next().await, BackendEvent::WorkerFailed));
        assert!(matches!(mailbox.next().await, BackendEvent::Regen(_)));
    }
    #[tokio::test]
    async fn open_reservation_ignores_prior_runtime_and_retargets_before_waiting() {
        let mailbox = EventMailbox::default();
        mailbox.begin(Expected::Replacement {
            previous: "old".into(),
        });
        mailbox.observe_projection("new", 1);
        mailbox.emit(terminal("new", 1, "superseded")).unwrap();
        mailbox.emit(terminal("old", 999, "published")).unwrap();
        mailbox.emit(terminal("new", 1, "published")).unwrap();
        mailbox.emit(terminal("old", 999, "failed")).unwrap();
        mailbox.retarget("new", 1);
        let BackendEvent::Regen(event) = mailbox.next().await else {
            panic!("missing new runtime terminal")
        };
        assert_eq!(event.runtime_session, "new");
    }
}

#[cfg(test)]
mod early_tests {
    use super::*;
    #[tokio::test]
    async fn retarget_promotes_early_completion_even_without_existing_terminal() {
        let mailbox = EventMailbox::default();
        mailbox.begin(Expected::Known {
            session: "current".into(),
            revision: 10,
        });
        let result = RegenFinished {
            runtime_session: "current".into(),
            revision: 9,
            source_revision: 9,
            outcome: "published".into(),
            message: None,
            diagnostics: vec![],
            failed_steps: vec![],
            repair_steps: vec![],
            affected_bodies: Default::default(),
        };
        mailbox.emit(BackendEvent::Regen(result)).unwrap();
        mailbox.retarget("current", 9);
        assert!(mailbox.slots.lock().unwrap().terminal.is_some());
        let BackendEvent::Regen(event) = mailbox.next().await else {
            panic!("missing early terminal");
        };
        assert_eq!(event.source_revision, 9);
    }
}
