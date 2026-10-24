//! Routed touch events (`onecad_interaction::touch::RoutedEvent`) to egui input.
//! `Author`, `Control` and `Tap` owners drive egui's primary pointer; a one
//! finger `Orbit` becomes a Shift trackpad wheel (the editor's orbit gesture);
//! `MultiTouch` fingers leave navigation to the native recognizers; a `Cancel`
//! never delivers a release; an `Activate` (a control tapped while the pointer
//! is held) is queued for an accessibility click at the next frame.
use super::{Event, Gone, Input, Pointer};
use egui::{Pos2, TouchPhase, Vec2};
use onecad_interaction::touch::{ContactKind, GesturePhase, Overlay, Owner, Point, RoutedEvent};

fn pos(point: Point) -> Pos2 {
    Pos2::new(point.x as f32, point.y as f32)
}

fn touch(contact: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(contact),
        phase,
        pos,
        force: None,
    }
}

impl Input {
    pub(super) fn apply_all(&mut self, events: Vec<RoutedEvent>) {
        for event in events {
            self.apply(event);
        }
    }

    fn apply(&mut self, event: RoutedEvent) {
        match event {
            RoutedEvent::Press {
                contact,
                kind,
                owner,
                position,
                tap_count,
                ..
            } => self.press(contact, kind, owner, pos(position), tap_count),
            RoutedEvent::Move {
                contact, position, ..
            } => self.drag(contact, pos(position)),
            RoutedEvent::Release {
                contact,
                owner,
                kind,
                position,
            } => self.release(contact, owner, kind, pos(position)),
            RoutedEvent::Hover { kind, position } => self.hover(kind, pos(position)),
            RoutedEvent::HoverExit { .. } => {
                if self.hovering && self.pointer.is_none() {
                    self.hovering = false;
                    self.gone = Gone::Armed;
                }
            }
            RoutedEvent::Orbit {
                phase,
                position,
                delta,
                ..
            } => self.orbit(
                phase,
                pos(position),
                egui::vec2(delta.x as f32, delta.y as f32),
            ),
            RoutedEvent::MultiTouch { .. } => {}
            RoutedEvent::Overlay(Overlay::Prediction { contact, points }) => {
                self.predict(contact, points);
            }
            RoutedEvent::Cancel { contact, .. } => self.cancel_contact(contact),
            RoutedEvent::Activate { position, .. } => self.activations.push(pos(position)),
        }
    }

    fn predict(&mut self, contact: u64, points: Vec<Point>) {
        if points.is_empty() {
            self.predictions.remove(&contact);
        } else {
            let points = points.into_iter().map(pos).collect();
            self.predictions.insert(contact, points);
        }
    }

    fn cancel_contact(&mut self, contact: u64) {
        if self
            .pointer
            .is_some_and(|pointer| pointer.contact == contact)
        {
            self.cancel_pointer();
        }
        self.predictions.remove(&contact);
    }

    /// A Pencil tip's second tap (`tap_count` 2) promotes like a double click
    /// even when egui's click timing disagrees (INPUT.md).
    fn press(&mut self, contact: u64, kind: ContactKind, owner: Owner, at: Pos2, tap_count: u32) {
        // The router hands out one pointer at a time; never leave one pressed.
        self.cancel_pointer();
        self.contact_epoch = self.contact_epoch.wrapping_add(1);
        self.pointer = Some(Pointer {
            contact,
            kind,
            control_touch: kind == ContactKind::Finger && owner == Owner::Control,
            last: at,
            delivered: false,
        });
        self.gone = Gone::Idle;
        self.hovering = false;
        self.mark_pointer_kind(kind);
        self.pencil_double_tap = Some(kind == ContactKind::Pencil && tap_count >= 2);
        if self.pointer.is_some_and(|pointer| pointer.control_touch) {
            self.push_owned(touch(contact, TouchPhase::Start, at), false);
        }
        self.push_owned(Event::PointerMoved(at), false);
        self.push_owned(self.button(at, true), false);
    }

    fn drag(&mut self, contact: u64, at: Pos2) {
        self.predictions.remove(&contact);
        if let Some(pointer) = &mut self.pointer
            && pointer.contact == contact
            && pointer.last != at
        {
            pointer.last = at;
            let kind = pointer.kind;
            let control_touch = pointer.control_touch;
            self.mark_pointer_kind(kind);
            if control_touch {
                self.push_owned(touch(contact, TouchPhase::Move, at), true);
            }
            self.push_owned(Event::PointerMoved(at), true);
        }
    }

    fn release(&mut self, contact: u64, owner: Owner, kind: ContactKind, at: Pos2) {
        if self
            .pointer
            .is_none_or(|pointer| pointer.contact != contact)
        {
            return;
        }
        self.mark_pointer_kind(kind);
        if self.pointer.is_some_and(|pointer| pointer.control_touch) {
            self.push_owned(touch(contact, TouchPhase::End, at), false);
        }
        self.push_owned(Event::PointerMoved(at), false);
        self.push_owned(self.button(at, false), false);
        self.pointer = None;
        self.predictions.remove(&contact);
        // A lifted finger cannot hover; Pencil and pointers report hover exit.
        if kind == ContactKind::Finger || owner == Owner::Tap {
            self.gone = Gone::Armed;
        }
    }

    fn hover(&mut self, kind: ContactKind, at: Pos2) {
        if self.pointer.is_some() {
            return;
        }
        self.hovering = true;
        self.gone = Gone::Idle;
        self.mark_pointer_kind(kind);
        self.push(Event::PointerMoved(at), true);
    }

    fn mark_pointer_kind(&mut self, kind: ContactKind) {
        self.pointer_kind_conflict |= self.pointer_kind.is_some_and(|prior| prior != kind);
        self.pointer_kind = Some(kind);
    }

    /// The editor orbits on a Shift trackpad wheel over the hovered canvas; a
    /// held pointer (a finger on a control) is never moved for it.
    fn orbit(&mut self, phase: GesturePhase, at: Pos2, delta: Vec2) {
        let phase = match phase {
            GesturePhase::Start => TouchPhase::Start,
            GesturePhase::Move => TouchPhase::Move,
            GesturePhase::End => TouchPhase::End,
            GesturePhase::Cancel => TouchPhase::Cancel,
        };
        if phase == TouchPhase::Move && self.merge_orbit(at, delta) {
            return;
        }
        let free = self.pointer.is_none();
        let start = self.pending.len();
        if free {
            self.gone = Gone::Idle;
            self.push(Event::PointerMoved(at), false);
        }
        let modifiers = egui::Modifiers {
            shift: true,
            ..self.modifiers
        };
        self.push(
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase,
                source: egui::MouseWheelSource::Trackpad,
                modifiers,
            },
            false,
        );
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) && free {
            self.gone = Gone::Armed;
        }
        self.orbit_tail = (phase == TouchPhase::Move).then_some(start);
    }

    /// Folds a move into the trailing queued orbit move: a burst between frames
    /// stays one anchor and one summed delta.
    fn merge_orbit(&mut self, at: Pos2, delta: Vec2) -> bool {
        let Some(start) = self.orbit_tail else {
            return false;
        };
        for item in &mut self.pending[start..] {
            match &mut item.event {
                Event::PointerMoved(position) => *position = at,
                Event::MouseWheel { delta: queued, .. } => *queued += delta,
                _ => {}
            }
        }
        true
    }

    /// Cancellation never delivers a release, so it can neither click nor commit
    /// a drag: the contact's queued pointer events are dropped, and a press egui
    /// has already seen is forgotten before the next frame, which carries
    /// `PointerGone` so the editor's canvas gestures cancel.
    pub(super) fn cancel_pointer(&mut self) {
        let Some(pointer) = self.pointer.take() else {
            return;
        };
        self.predictions.remove(&pointer.contact);
        self.pending
            .retain(|item| item.contact != Some(pointer.contact));
        if !self.pending.iter().any(|item| {
            matches!(
                item.event,
                Event::PointerMoved(_) | Event::PointerButton { .. }
            )
        }) {
            self.pointer_kind = None;
            self.pointer_kind_conflict = false;
        }
        self.navigation_tail = None;
        self.orbit_tail = None;
        if pointer.delivered {
            if pointer.control_touch {
                self.push(
                    touch(pointer.contact, TouchPhase::Cancel, pointer.last),
                    false,
                );
            }
            self.forget_pointer = true;
            self.push(Event::PointerGone, false);
        }
        self.gone = Gone::Idle;
    }
}
