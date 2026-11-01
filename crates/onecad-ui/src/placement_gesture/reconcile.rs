//! Turns hover knowledge into the snap the reducer previews.
use super::{
    ComponentPlacement, Knowledge, Phase, autosize, mate,
    snap::{self, Candidate, IntentKey, Sent, SnapIntent},
};
use crate::{app::App, library_catalog::Placed};
use onecad_core::{
    document::record::{ComponentParamValue as Value, TransformRotation},
    ids::BodyId,
};
use onecad_interaction::{ModelingOptions, SessionPhase};
use std::time::Instant;

/// The element a mate seats on.
struct MateTarget {
    body: BodyId,
    element: String,
    topology: String,
}

/// Applies the reply of the one classification in flight, then asks for the next target.
/// A reply from another editor context, or one that arrives after the timeout, is dropped.
pub(super) fn take_classification(app: &mut App, p: &mut ComponentPlacement, now: Instant) {
    if let Some(flight) = p.hover.in_flight.clone() {
        if let Some(reply) = app.ui.library.catalog.take_placement_reply(flight.sequence) {
            p.hover.in_flight = None;
            let fresh = app.doc.interaction.context.as_ref() == Some(&flight.context);
            let current = fresh
                && p.hover
                    .target
                    .as_ref()
                    .is_some_and(|t| (t.body, t.topology.clone()) == flight.key);
            if current {
                accept(p, reply);
            }
        } else if now.duration_since(flight.since) >= super::REQUEST_TIMEOUT {
            p.hover.in_flight = None;
            p.hover.target = None;
            p.hover.class = Knowledge::None;
            p.notice = Some("Snap target lookup timed out; move the pointer to retry".into());
            app.ui.library.catalog.drop_placement();
        }
    }
    super::hover::pump_classify(app, p);
}

fn accept(p: &mut ComponentPlacement, reply: Result<Placed, String>) {
    let none = || crate::library_catalog::Classified {
        class: None,
        element_id: None,
    };
    let (known, notice) = match reply {
        Ok(Placed::Classified(known)) => (*known, None),
        Ok(_) => (
            none(),
            Some("Snap target unavailable: unexpected reply".into()),
        ),
        Err(e) => (none(), Some(format!("Snap target unavailable: {e}"))),
    };
    p.notice = notice;
    p.hover.class = Knowledge::Known(Box::new(known));
}

fn editing(app: &App) -> bool {
    super::placement_tool(app).is_some_and(|t| t.phase == SessionPhase::Editing)
}

/// Re-derives the snap from the hover and sends it when it changed enough.
pub(super) fn reconcile(app: &mut App) {
    let Some(mut p) = app.doc.placement.take() else {
        return;
    };
    if matches!(p.phase, Phase::Armed { .. }) && editing(app) {
        apply(app, &mut p);
    }
    app.doc.placement = Some(p);
}

fn resolve(p: &mut ComponentPlacement) -> Option<(SnapIntent, Option<MateTarget>)> {
    let ground = |p: &ComponentPlacement| {
        let (origin, direction) = p.hover.ray?;
        snap::ground_point(origin, direction).map(|point| SnapIntent::Ground { point })
    };
    let (target, class) = match (&p.hover.target, &p.hover.class) {
        (None, _) => {
            p.compatible = 0;
            return ground(p).map(|intent| (intent, None));
        }
        (Some(_), Knowledge::None | Knowledge::Pending) => return None,
        (Some(target), Knowledge::Known(class)) => (target.clone(), class.clone()),
    };
    let candidates: Vec<Candidate> = match (&class.class, &class.element_id) {
        (Some(info), Some(_)) => snap::candidates(&p.component.attachments, info, target.point),
        _ => Vec::new(),
    };
    p.compatible = candidates.len();
    let Some(chosen) = candidates.get(p.attachment_cursor % candidates.len().max(1)) else {
        return ground(p).map(|intent| (intent, None));
    };
    let mate_target = MateTarget {
        body: target.body,
        element: class.element_id.clone()?,
        topology: target.topology,
    };
    Some((SnapIntent::Mate(chosen.clone()), Some(mate_target)))
}

fn key_of(intent: &SnapIntent, target: Option<&MateTarget>) -> IntentKey {
    match (intent, target) {
        (SnapIntent::Mate(c), Some(t)) => IntentKey::Mate {
            body: t.body,
            topology: t.topology.clone(),
            attachment: c.attachment.clone(),
        },
        _ => IntentKey::Ground,
    }
}

fn apply(app: &mut App, p: &mut ComponentPlacement) {
    let Some((intent, target)) = resolve(p) else {
        return;
    };
    let key = key_of(&intent, target.as_ref());
    if p.flip_key.as_ref() != Some(&key) {
        p.flipped = default_flip(p, &intent);
        p.flip_key = Some(key.clone());
    }
    let sent = Sent {
        key,
        flipped: p.flipped,
        anchor: intent.anchor(),
        follows_pointer: snap::follows_pointer(&intent),
    };
    if snap::should_dispatch(p.sent.as_ref(), &sent) {
        send_snap(app, p, &intent, target.as_ref());
        p.sent = Some(sent);
        p.snap = intent.clone();
    }
    if let SnapIntent::Mate(candidate) = &intent {
        resize(app, p, candidate);
    }
}

fn default_flip(p: &ComponentPlacement, intent: &SnapIntent) -> bool {
    match (intent, p.hover.ray) {
        (SnapIntent::Mate(c), Some((_, direction))) => snap::default_flip(c, direction.map(|v| -v)),
        _ => false,
    }
}

fn send_snap(
    app: &mut App,
    p: &ComponentPlacement,
    intent: &SnapIntent,
    target: Option<&MateTarget>,
) {
    let mate = match (intent, target) {
        (SnapIntent::Mate(c), Some(t)) => Some(Box::new(mate::build(
            &p.component.attachments,
            c,
            (t.body, &t.element),
            p.flipped,
        ))),
        _ => None,
    };
    super::send(
        app,
        ModelingOptions::ComponentSnap {
            translate: intent.anchor(),
            rotate: TransformRotation::default(),
            mate,
        },
    );
}

/// A hole, rim or bore picks its thread unless the user typed one.
fn resize(app: &mut App, p: &mut ComponentPlacement, candidate: &Candidate) {
    let thread = p
        .draft
        .fields()
        .iter()
        .find(|f| f.key == autosize::THREAD)
        .cloned();
    let Some(map) = autosize::resized(thread.as_ref(), &p.params, &p.manual_params, candidate)
    else {
        return;
    };
    if let Some(Value::Text(fit)) = map.get(autosize::THREAD) {
        p.draft.set_text(autosize::THREAD, fit.clone());
        p.auto_note = Some(format!("auto {fit}"));
    }
    p.params = map.clone();
    super::send(app, ModelingOptions::ComponentParams(map));
}
