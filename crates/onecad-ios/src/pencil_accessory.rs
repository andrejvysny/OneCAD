//! Exact callback identity, bounded independently of callback timing.
use onecad_interaction::touch::accessory::AccessoryAction;
use onecad_ui::host::{OwnerIdentity, PresentationContext};
use std::collections::VecDeque;

type ContactContext = (u64, u64, Option<u64>);

#[derive(PartialEq)]
struct End {
    action: u32,
    timestamp: u64,
    owner: OwnerIdentity,
    runtime: Option<String>,
    contact: ContactContext,
}

pub struct Pending {
    pub action: AccessoryAction,
    context: PresentationContext,
    contact: ContactContext,
}

#[derive(Default)]
pub struct State {
    pending: Option<Pending>,
    ended: VecDeque<End>,
}

impl State {
    pub fn submit(
        &mut self,
        event: &crate::ffi::PencilEvent,
        action: AccessoryAction,
        context: PresentationContext,
        contact: ContactContext,
    ) {
        if event.phase != crate::ffi::PHASE_ENDED || !context.owner.editor_visible {
            return;
        }
        let end = End {
            action: event.action,
            timestamp: event.timestamp_s.to_bits(),
            owner: context.owner,
            runtime: context.runtime.clone(),
            contact,
        };
        if self.ended.contains(&end) {
            return;
        }
        if self.ended.len() == 64 {
            self.ended.pop_front();
        }
        self.ended.push_back(end);
        if !matches!(
            action,
            AccessoryAction::Nothing | AccessoryAction::SystemShortcut
        ) {
            self.pending = Some(Pending {
                action,
                context,
                contact,
            });
        }
    }

    pub fn take(
        &mut self,
        context: &PresentationContext,
        contact: ContactContext,
    ) -> Option<AccessoryAction> {
        self.pending
            .take()
            .filter(|pending| &pending.context == context && pending.contact == contact)
            .map(|pending| pending.action)
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

pub fn feedback(changed: bool, flags: u32) -> Option<crate::host::Request> {
    (changed && flags & crate::ffi::CAP_HAPTICS != 0).then(|| crate::host::Request {
        kind: crate::ffi::REQ_HAPTIC,
        text: "pencil-tool".into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PresentationContext {
        let mut context = PresentationContext::default();
        context.owner.editor_visible = true;
        context.runtime = Some("runtime".into());
        context
    }

    #[test]
    fn repeated_end_does_not_toggle_twice_and_distinct_timestamp_does() {
        let mut state = State::default();
        let mut event = crate::ffi::PencilEvent {
            phase: crate::ffi::PHASE_ENDED,
            timestamp_s: 2.0,
            ..Default::default()
        };
        let contact = (1, 3, Some(7));
        state.submit(
            &event,
            AccessoryAction::TogglePreviousTool,
            context(),
            contact,
        );
        assert_eq!(
            state.take(&context(), contact),
            Some(AccessoryAction::TogglePreviousTool)
        );
        state.submit(
            &event,
            AccessoryAction::TogglePreviousTool,
            context(),
            contact,
        );
        assert!(state.take(&context(), contact).is_none());
        event.timestamp_s += f64::EPSILON * 2.0;
        state.submit(
            &event,
            AccessoryAction::TogglePreviousTool,
            context(),
            contact,
        );
        assert!(state.take(&context(), contact).is_some());
    }

    #[test]
    fn stale_context_contact_cancel_and_home_never_apply() {
        for changed in 0..4 {
            let mut state = State::default();
            let event = crate::ffi::PencilEvent {
                phase: crate::ffi::PHASE_ENDED,
                timestamp_s: 2.0,
                ..Default::default()
            };
            let mut context = context();
            state.submit(
                &event,
                AccessoryAction::TogglePreviousTool,
                context.clone(),
                (1, 0, None),
            );
            let contact = if changed == 0 {
                (2, 0, None)
            } else {
                (1, 0, None)
            };
            match changed {
                1 => context.owner.editor_visible = false,
                2 => context.runtime = Some("new".into()),
                3 => state.clear(),
                _ => {}
            }
            assert!(state.take(&context, contact).is_none());
        }
    }

    #[test]
    fn feedback_requires_success_and_real_haptics_capability() {
        assert!(feedback(false, crate::ffi::CAP_HAPTICS).is_none());
        assert!(feedback(true, crate::ffi::CAP_SQUEEZE).is_none());
        let request = feedback(true, crate::ffi::CAP_HAPTICS).unwrap();
        assert_eq!(request.kind, crate::ffi::REQ_HAPTIC);
        assert_eq!(request.text, "pencil-tool");
    }
}
