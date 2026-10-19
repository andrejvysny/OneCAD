//! One bounded drag lane. Worker results may change only transient preview geometry.
use super::{
    SketchInteraction, SketchPhase, SketchSession, SketchSessionKey, SketchTarget, SketchTool,
};
use crate::DocumentContext;
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{CurvePosition, Sketch, SketchEntity},
};
use onecad_scene::CaptureIdentity;

mod model;
mod preview;
mod receipts;
mod target;
pub use model::*;
pub use target::{classify_candidates, classify_target};

impl SketchGestureInteraction {
    pub fn dispatch(
        &mut self,
        action: GestureAction,
        sketch: &SketchInteraction,
    ) -> Vec<GestureEffect> {
        if self.gesture.as_ref().is_some_and(|gesture| {
            gesture.phase != GesturePhase::Cancelling && !current(gesture, sketch)
        }) {
            self.error = Some(
                "Gesture cancelled because its document, visit, capture or accepted sketch changed"
                    .into(),
            );
            return self.cancel();
        }
        match action {
            GestureAction::Begin {
                scope,
                target,
                grab,
            } => self.begin(scope, target, grab, sketch),
            GestureAction::Move { tag, point } => self.pointer(tag, point, false),
            GestureAction::End { tag, point } => self.pointer(tag, point, true),
            GestureAction::Cancel { tag } if self.owns(&tag) => self.cancel(),
            GestureAction::Ready {
                tag,
                worker_id,
                preview,
            } => self.ready(tag, worker_id, preview),
            GestureAction::Updated {
                tag,
                worker_id,
                worker_sequence,
                preview,
            } => self.updated(tag, worker_id, worker_sequence, preview),
            GestureAction::Completed {
                tag,
                worker_id,
                moved,
            } => {
                self.completed(tag, worker_id, moved);
                Vec::new()
            }
            GestureAction::Cancelled { tag } => {
                self.cancelled(tag);
                Vec::new()
            }
            GestureAction::Failed { tag, message } => self.failed(tag, &message),
            GestureAction::Sync | GestureAction::Cancel { .. } => Vec::new(),
        }
    }

    fn begin(
        &mut self,
        scope: GestureScope,
        target: SketchTarget,
        grab: Vec2,
        sketch: &SketchInteraction,
    ) -> Vec<GestureEffect> {
        if self.busy() {
            return self.refuse("Finish or cancel the active gesture first");
        }
        let Some(session) = sketch.session.as_ref() else {
            return self.refuse("Gesture requires an active sketch");
        };
        if sketch.context.as_ref() != Some(&scope.document)
            || GestureScope::from_session(session).as_ref() != Ok(&scope)
        {
            return self.refuse("Gesture input belongs to a stale sketch visit or captured view");
        }
        let intent = match classify_target(&session.authoritative, target, grab) {
            Ok(intent) => intent,
            Err(message) => return self.refuse(&message),
        };
        let Some(serial) = self.serial.checked_add(1) else {
            return self.refuse("Gesture identity limit exceeded");
        };
        self.serial = serial;
        self.error = None;
        self.moved = None;
        let tag = GestureTag {
            scope,
            gesture: serial,
            sequence: 0,
        };
        self.gesture = Some(Gesture {
            tag: tag.clone(),
            phase: GesturePhase::PendingBegin,
            intent: intent.clone(),
            baseline: session.authoritative.clone(),
            preview: None,
            worker_id: None,
            worker_sequence: 0,
            latest: None,
            end: None,
        });
        vec![GestureEffect::Begin { tag, intent }]
    }

    fn pointer(&mut self, tag: GestureTag, point: Vec2, end: bool) -> Vec<GestureEffect> {
        if !self.owns(&tag) {
            return Vec::new();
        }
        if !point.is_finite() {
            return self.refuse("Gesture target coordinates must be finite");
        }
        let gesture = self.gesture.as_mut().expect("scoped gesture");
        if !matches!(
            gesture.phase,
            GesturePhase::PendingBegin | GesturePhase::Ready | GesturePhase::PendingUpdate
        ) || gesture.end.is_some()
        {
            return Vec::new();
        }
        if end {
            gesture.end = Some(point);
            gesture.latest = None;
        } else {
            gesture.latest = Some(point);
        }
        self.flush()
    }

    fn flush(&mut self) -> Vec<GestureEffect> {
        let Some(gesture) = self
            .gesture
            .as_mut()
            .filter(|gesture| gesture.phase == GesturePhase::Ready)
        else {
            return Vec::new();
        };
        let point = gesture.end.or(gesture.latest);
        let Some(point) = point else {
            return Vec::new();
        };
        let Some(sequence) = gesture
            .tag
            .sequence
            .checked_add(1)
            .filter(|sequence| *sequence < u64::MAX)
        else {
            self.error = Some("Gesture sequence limit exceeded".into());
            return self.cancel();
        };
        gesture.tag.sequence = sequence;
        gesture.latest = None;
        let tag = gesture.tag.clone();
        let worker_id = gesture.worker_id.expect("ready worker gesture");
        if gesture.end.is_some() {
            gesture.phase = GesturePhase::PendingEnd;
            vec![GestureEffect::End {
                tag,
                worker_id,
                point,
            }]
        } else {
            gesture.phase = GesturePhase::PendingUpdate;
            vec![GestureEffect::Update {
                tag,
                worker_id,
                point,
            }]
        }
    }

    fn cancel(&mut self) -> Vec<GestureEffect> {
        let Some(gesture) = self
            .gesture
            .as_mut()
            .filter(|gesture| gesture.phase != GesturePhase::Cancelling)
        else {
            return Vec::new();
        };
        gesture.tag.sequence += 1;
        gesture.phase = GesturePhase::Cancelling;
        gesture.preview = None;
        gesture.latest = None;
        gesture.end = None;
        vec![GestureEffect::Cancel {
            tag: gesture.tag.clone(),
            worker_id: gesture.worker_id,
        }]
    }

    fn owns(&self, tag: &GestureTag) -> bool {
        self.gesture.as_ref().is_some_and(|gesture| {
            gesture.tag.gesture == tag.gesture
                && gesture.tag.scope == tag.scope
                && tag.sequence <= gesture.tag.sequence
        })
    }

    fn refuse(&mut self, message: &str) -> Vec<GestureEffect> {
        self.error = Some(message.chars().take(512).collect());
        Vec::new()
    }
}

fn current(gesture: &Gesture, sketch: &SketchInteraction) -> bool {
    sketch.context.as_ref() == Some(&gesture.tag.scope.document)
        && sketch.session.as_ref().is_some_and(|session| {
            GestureScope::from_session(session).as_ref() == Ok(&gesture.tag.scope)
                && session.authoritative == gesture.baseline
        })
}
