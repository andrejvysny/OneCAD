//! UIKit samples to egui events, queued until the next frame.
//!
//! Samples go through `onecad_interaction::touch` (`docs/ios/INPUT.md`): the
//! normalizer orders, dedupes and bounds them and drops predictions from the
//! pointer stream; the router gives every contact one owner for its whole life
//! from the previous frame's route context. `input_routed.rs` maps the routed
//! events to egui pointer, trackpad-wheel and overlay input; keys, text and the
//! native navigation recognizers are queued here in arrival order.
use crate::ffi::{self, TouchSample};
use egui::{Event, PointerButton, Pos2};
use onecad_interaction::touch::{
    self, CancelReason, ContactKind, Normalizer, RawSample, RouteContext, Router,
};
use std::collections::HashMap;

#[path = "input_navigation.rs"]
mod navigation;
#[path = "input_routed.rs"]
mod routed;

pub const BATCH_MAX: usize = touch::MAX_BATCH;
pub const PENDING_MAX: usize = touch::MAX_PENDING;

struct Item {
    event: Event,
    /// Intermediate movement that may be coalesced away under pressure.
    disposable: bool,
    /// The pointer contact whose stream produced this event.
    contact: Option<u64>,
}

/// The contact holding egui's single pointer (an `Author`, `Control` or `Tap`
/// owner).
#[derive(Clone, Copy)]
struct Pointer {
    contact: u64,
    last: Pos2,
    /// Its press has reached egui (a frame drained it).
    delivered: bool,
}

/// A `PointerGone` that must not share a frame with the release or navigation
/// it follows: `sketch_gestures::route_raw` cancels on any `PointerGone` in a
/// frame, and egui would drop that frame's hover-gated navigation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gone {
    Idle,
    Armed,
    Due,
}

/// One frame's input.
#[derive(Debug, Default, PartialEq)]
pub struct Drained {
    pub events: Vec<Event>,
    /// Another frame is needed to deliver a deferred `PointerGone`.
    pub again: bool,
    /// A contact whose press egui has seen was cancelled: before this frame the
    /// editor must forget the press (`onecad_ui::host::cancel_contact`), so no
    /// release, click or drag commit follows from it.
    pub cancelled: bool,
    /// The input kind of the newest press or hover, for the pick aperture
    /// (`onecad_ui::host::set_pointer_kind`).
    pub pointer_kind: Option<onecad_ui::host::PointerKind>,
    /// Whether the newest press is a Pencil tip double tap
    /// (`onecad_ui::host::set_pencil_double_tap`); `None` without a press.
    pub pencil_double_tap: Option<bool>,
    /// Controls tapped while another contact held the pointer, in points: the
    /// frame clicks the accessibility node under each.
    pub activations: Vec<Pos2>,
}

pub struct Input {
    pending: Vec<Item>,
    normalizer: Normalizer,
    router: Router,
    pointer: Option<Pointer>,
    /// A Pencil or pointer hovers with no pointer held.
    hovering: bool,
    gone: Gone,
    forget_pointer: bool,
    navigating: bool,
    /// The current navigation gesture is ignored (a Pencil or pointer authors,
    /// so the camera is held stable).
    navigation_ignored: bool,
    /// Start of the trailing queued navigation `Move` group, which later moves
    /// of the same gesture merge into.
    navigation_tail: Option<usize>,
    /// Start of the trailing queued one-finger orbit move, merged the same way.
    orbit_tail: Option<usize>,
    composing: bool,
    modifiers: egui::Modifiers,
    /// Latest predicted points per authoring contact: overlay input only.
    predictions: HashMap<u64, Vec<Pos2>>,
    pointer_kind: Option<ContactKind>,
    pencil_double_tap: Option<bool>,
    activations: Vec<Pos2>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            normalizer: Normalizer::default(),
            router: Router::default(),
            pointer: None,
            hovering: false,
            gone: Gone::Idle,
            forget_pointer: false,
            navigating: false,
            navigation_ignored: false,
            navigation_tail: None,
            orbit_tail: None,
            composing: false,
            modifiers: egui::Modifiers::NONE,
            predictions: HashMap::new(),
            pointer_kind: None,
            pencil_double_tap: None,
            activations: Vec::new(),
        }
    }
}

fn raw(sample: &TouchSample) -> RawSample {
    RawSample {
        contact_id: sample.contact_id,
        kind: sample.kind,
        phase: sample.phase,
        provenance: sample.provenance,
        present: sample.present,
        estimated: sample.estimated,
        expects_update: sample.expects_update,
        timestamp_s: sample.timestamp_s,
        x_pt: sample.x_pt,
        y_pt: sample.y_pt,
        force: sample.force,
        max_force: sample.max_force,
        altitude_rad: sample.altitude_rad,
        azimuth_rad: sample.azimuth_rad,
        roll_rad: sample.roll_rad,
        hover_z: sample.hover_z,
        estimation_index: sample.estimation_index,
        tap_count: sample.tap_count,
    }
}

impl Input {
    /// Normalizes and routes one host batch with the route context of the
    /// previous frame. `Ok(true)` when queued movement had to be coalesced.
    pub fn submit(
        &mut self,
        samples: &[TouchSample],
        generation: u64,
        route: &RouteContext<'_>,
    ) -> Result<bool, touch::SubmitError> {
        let raw: Vec<RawSample> = samples.iter().map(raw).collect();
        let merged = self.normalizer.submit_raw(&raw, generation)?;
        let routed = self.router.route(self.normalizer.drain(), route);
        self.apply_all(routed);
        Ok(self.enforce_bound() || merged)
    }

    /// System gesture, interruption, text focus or document switch: every live
    /// contact is cancelled without a release.
    pub fn cancel_all(&mut self, reason: CancelReason) {
        self.normalizer.cancel_all(reason);
        let routed = self
            .router
            .route(self.normalizer.drain(), &RouteContext::CANVAS);
        self.apply_all(routed);
        // A synthesized tap holds no router contact; nothing may outlive this.
        self.cancel_pointer();
        self.cancel_navigation();
        self.predictions.clear();
    }

    fn button(&self, pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: self.modifiers,
        }
    }

    fn push(&mut self, event: Event, disposable: bool) {
        self.navigation_tail = None;
        self.orbit_tail = None;
        self.pending.push(Item {
            event,
            disposable,
            contact: None,
        });
    }

    fn push_owned(&mut self, event: Event, disposable: bool) {
        self.navigation_tail = None;
        self.orbit_tail = None;
        let contact = self.pointer.map(|pointer| pointer.contact);
        self.pending.push(Item {
            event,
            disposable,
            contact,
        });
    }

    /// Coalesces only intermediate movement, oldest first; terminal events stay.
    fn enforce_bound(&mut self) -> bool {
        let excess = self.pending.len().saturating_sub(PENDING_MAX);
        if excess == 0 {
            return false;
        }
        let mut dropped = 0;
        self.pending.retain(|item| {
            let drop = item.disposable && dropped < excess;
            dropped += usize::from(drop);
            !drop
        });
        self.navigation_tail = None;
        self.orbit_tail = None;
        true
    }

    pub fn key(&mut self, event: &ffi::KeyEvent) {
        let modifiers = crate::keys::modifiers(event.modifiers);
        if modifiers != self.modifiers {
            self.modifiers = modifiers;
            self.push(Event::ModifiersChanged(modifiers), false);
        }
        if let Some(key) = crate::keys::from_hid(event.hid_usage) {
            self.push(
                Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: event.pressed != 0,
                    repeat: event.repeat != 0,
                    modifiers,
                },
                false,
            );
        }
    }

    fn tap_key(&mut self, key: egui::Key) {
        for pressed in [true, false] {
            let modifiers = self.modifiers;
            self.push(
                Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                },
                false,
            );
        }
    }

    /// Committed text. A return from the software keyboard is egui's Enter key.
    pub fn insert_text(&mut self, text: &str) {
        if self.composing {
            self.composing = false;
            self.push(Event::Ime(egui::ImeEvent::Commit(text.to_owned())), false);
        } else if text == "\n" || text == "\r" {
            self.tap_key(egui::Key::Enter);
        } else if !text.is_empty() {
            self.push(Event::Text(text.to_owned()), false);
        }
    }

    pub fn delete_backward(&mut self) {
        self.tap_key(egui::Key::Backspace);
    }

    /// Marked (IME) text; empty ends composition without committing.
    pub fn set_marked(&mut self, text: &str) {
        if text.is_empty() && !self.composing {
            return;
        }
        self.composing = !text.is_empty();
        let preedit = egui::ImeEvent::Preedit {
            text: text.to_owned(),
            active_range_chars: None,
        };
        self.push(Event::Ime(preedit), false);
    }

    /// A tap synthesized for an accessibility activation without a click action.
    pub fn synthesized_tap(&mut self, pos: Pos2) {
        self.cancel_pointer();
        self.push(Event::PointerMoved(pos), false);
        self.push(self.button(pos, true), false);
        self.push(self.button(pos, false), false);
        self.gone = Gone::Armed;
    }

    pub fn push_event(&mut self, event: Event) {
        self.push(event, false);
    }

    /// The frame's events and what the frame must do around them.
    pub fn drain(&mut self) -> Drained {
        let mut events = Vec::with_capacity(self.pending.len() + 1);
        // A new press, hover or navigation since arming resets `gone` to Idle.
        if self.gone == Gone::Due {
            self.gone = Gone::Idle;
            if self.pointer.is_none() {
                events.push(Event::PointerGone);
            }
        }
        events.extend(self.pending.drain(..).map(|item| item.event));
        self.navigation_tail = None;
        self.orbit_tail = None;
        if let Some(pointer) = &mut self.pointer {
            pointer.delivered = true;
        }
        let again = self.gone == Gone::Armed;
        if again {
            self.gone = Gone::Due;
        }
        let pointer_kind = self.pointer_kind.take().map(|kind| match kind {
            ContactKind::Finger => onecad_ui::host::PointerKind::Finger,
            ContactKind::Pencil | ContactKind::IndirectPointer => {
                onecad_ui::host::PointerKind::Precise
            }
        });
        Drained {
            events,
            again,
            cancelled: std::mem::take(&mut self.forget_pointer),
            pointer_kind,
            pencil_double_tap: self.pencil_double_tap.take(),
            activations: std::mem::take(&mut self.activations),
        }
    }

    /// Newest predicted point of a live authoring contact: overlay input only.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn prediction(&self, contact: u64) -> Option<Pos2> {
        self.predictions.get(&contact)?.last().copied()
    }

    /// The newest Pencil position, kept after the Pencil lifts.
    pub fn pencil_position(&self) -> Option<Pos2> {
        self.router
            .pencil_position()
            .map(|point| Pos2::new(point.x as f32, point.y as f32))
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
