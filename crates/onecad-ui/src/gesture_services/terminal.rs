//! One observer prevents document commands from consuming a gesture's exact completion.
use super::*;
use crate::event_mailbox::{EventMailbox, Expected};

pub(super) struct Reservation {
    slot: Arc<std::sync::Mutex<Option<Arc<EventMailbox>>>>,
    pub mailbox: Arc<EventMailbox>,
}
impl Reservation {
    pub fn new(
        slot: Arc<std::sync::Mutex<Option<Arc<EventMailbox>>>>,
        expected: &api::PreviewContext,
    ) -> Result<Self, String> {
        let mailbox = Arc::new(EventMailbox::default());
        mailbox.begin(Expected::Known {
            session: expected.runtime_session.clone(),
            revision: expected.revision.0.saturating_add(1),
        });
        {
            let mut owner = slot
                .lock()
                .map_err(|_| "Gesture completion observer poisoned")?;
            if owner.is_some() {
                return Err("Another gesture owns completion observation".into());
            }
            *owner = Some(mailbox.clone());
        }
        Ok(Self { slot, mailbox })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock()
            && slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.mailbox))
        {
            slot.take();
        }
    }
}
