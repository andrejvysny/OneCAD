//! UIKit samples to egui events, queued until the next frame.
//!
//! First version of the routing contract: the first Pencil, finger or pointer
//! contact that begins while none is active owns egui's primary pointer; others
//! are left to the native gesture recognizers (`navigation`). Actual and
//! coalesced samples are replayed in chronological order; predicted samples are
//! kept per contact for a transient overlay and never become events.
use crate::ffi::{self, TouchSample};
use egui::{Event, PointerButton, Pos2, TouchPhase};
use std::collections::HashMap;

pub const BATCH_MAX: usize = 4096;
pub const PENDING_MAX: usize = 16_384;

struct Item {
    event: Event,
    /// Intermediate movement that may be coalesced away under pressure.
    disposable: bool,
}

#[derive(Clone, Copy)]
struct Owner {
    contact: u64,
    kind: u32,
    last: Pos2,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prediction {
    pub position: Pos2,
    pub timestamp: f64,
}

pub struct Input {
    pending: Vec<Item>,
    owner: Option<Owner>,
    hovering: bool,
    gone: Gone,
    navigating: bool,
    composing: bool,
    modifiers: egui::Modifiers,
    newest_generation: u64,
    predictions: HashMap<u64, Prediction>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            owner: None,
            hovering: false,
            gone: Gone::Idle,
            navigating: false,
            composing: false,
            modifiers: egui::Modifiers::NONE,
            newest_generation: 0,
            predictions: HashMap::new(),
        }
    }
}

fn position(sample: &TouchSample) -> Pos2 {
    Pos2::new(sample.x_pt as f32, sample.y_pt as f32)
}

/// Drops prediction/estimation records and the coalesced copy of each actual
/// sample, then orders the rest by timestamp (stable for equal times).
fn chronological(samples: &[TouchSample]) -> Vec<TouchSample> {
    let duplicate = |sample: &TouchSample| {
        sample.provenance == ffi::SAMPLE_COALESCED
            && samples.iter().any(|actual| {
                actual.provenance == ffi::SAMPLE_ACTUAL
                    && actual.contact_id == sample.contact_id
                    && actual.timestamp_s == sample.timestamp_s
                    && actual.x_pt == sample.x_pt
                    && actual.y_pt == sample.y_pt
            })
    };
    let mut ordered: Vec<TouchSample> = samples
        .iter()
        .filter(|sample| {
            matches!(
                sample.provenance,
                ffi::SAMPLE_ACTUAL | ffi::SAMPLE_COALESCED
            ) && !duplicate(sample)
        })
        .copied()
        .collect();
    ordered.sort_by(|a, b| a.timestamp_s.total_cmp(&b.timestamp_s));
    ordered
}

impl Input {
    /// Returns `true` when queued movement had to be coalesced (overflow).
    pub fn submit(&mut self, samples: &[TouchSample], generation: u64) -> bool {
        let stale = generation < self.newest_generation;
        self.newest_generation = self.newest_generation.max(generation);
        for sample in samples
            .iter()
            .filter(|sample| sample.provenance == ffi::SAMPLE_PREDICTED)
        {
            self.predictions.insert(
                sample.contact_id,
                Prediction {
                    position: position(sample),
                    timestamp: sample.timestamp_s,
                },
            );
        }
        for sample in chronological(samples) {
            self.sample(&sample, stale);
        }
        self.enforce_bound()
    }

    fn sample(&mut self, sample: &TouchSample, stale: bool) {
        let finite = sample.x_pt.is_finite() && sample.y_pt.is_finite();
        let owned = self.owner.is_some_and(|o| o.contact == sample.contact_id);
        let usable = finite && !stale;
        match sample.phase {
            ffi::PHASE_BEGAN if self.owner.is_none() && usable => self.press(sample),
            ffi::PHASE_MOVED | ffi::PHASE_STATIONARY if owned && usable => self.drag(sample),
            ffi::PHASE_ENDED if owned && usable => self.release(sample),
            // A stale-coordinate end cannot commit at a meaningful point.
            ffi::PHASE_ENDED | ffi::PHASE_CANCELLED if owned => self.cancel_owner(),
            ffi::PHASE_HOVER_MOVED if self.owner.is_none() && usable => {
                self.hovering = true;
                self.gone = Gone::Idle;
                self.push(Event::PointerMoved(position(sample)), true);
            }
            ffi::PHASE_HOVER_EXITED if self.owner.is_none() && self.hovering => {
                self.hovering = false;
                self.gone = Gone::Armed;
            }
            _ => {}
        }
    }

    fn press(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        self.owner = Some(Owner {
            contact: sample.contact_id,
            kind: sample.kind,
            last: pos,
        });
        self.gone = Gone::Idle;
        self.push(Event::PointerMoved(pos), false);
        self.push(self.button(pos, true), false);
    }

    fn drag(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        if let Some(owner) = &mut self.owner
            && owner.last != pos
        {
            owner.last = pos;
            self.push(Event::PointerMoved(pos), true);
        }
    }

    fn release(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        let finger = self.owner.is_some_and(|o| o.kind == ffi::CONTACT_FINGER);
        self.push(Event::PointerMoved(pos), false);
        self.push(self.button(pos, false), false);
        self.predictions.remove(&sample.contact_id);
        self.owner = None;
        // A finger cannot hover after lifting; Pencil and pointers report hover exit.
        if finger {
            self.gone = Gone::Armed;
        }
    }

    /// Cancellation: `PointerGone` first so the frame's raw-input routing cancels
    /// sketch gestures, then the release that frees egui's pressed state.
    fn cancel_owner(&mut self) {
        if let Some(owner) = self.owner.take() {
            self.predictions.remove(&owner.contact);
            self.push(Event::PointerGone, false);
            self.push(self.button(owner.last, false), false);
            self.gone = Gone::Idle;
        }
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
        self.pending.push(Item { event, disposable });
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
        true
    }

    /// System gesture or interruption: every live contact is cancelled.
    pub fn cancel_all(&mut self) {
        self.cancel_owner();
        if self.navigating {
            self.navigation_wheel(egui::Vec2::ZERO, TouchPhase::Cancel);
        }
        self.predictions.clear();
    }

    pub fn navigation(&mut self, nav: &ffi::Navigation) -> bool {
        let phase = match nav.phase {
            ffi::PHASE_BEGAN => TouchPhase::Start,
            ffi::PHASE_MOVED => TouchPhase::Move,
            ffi::PHASE_ENDED => TouchPhase::End,
            ffi::PHASE_CANCELLED => TouchPhase::Cancel,
            _ => return false,
        };
        let values = [
            nav.anchor_x_pt,
            nav.anchor_y_pt,
            nav.pan_dx_pt,
            nav.pan_dy_pt,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return false;
        }
        let anchor = Pos2::new(nav.anchor_x_pt as f32, nav.anchor_y_pt as f32);
        let delta = egui::vec2(nav.pan_dx_pt as f32, nav.pan_dy_pt as f32);
        // Navigation reads the hovered viewport; a held contact keeps its own position.
        if self.owner.is_none() {
            self.gone = Gone::Idle;
            self.push(Event::PointerMoved(anchor), true);
        }
        if phase != TouchPhase::Move || delta != egui::Vec2::ZERO {
            self.navigation_wheel(delta, phase);
        }
        let scale = nav.scale_delta;
        if scale.is_finite() && scale > 0.0 && scale != 1.0 {
            self.push(Event::Zoom(scale as f32), false);
        }
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) && self.owner.is_none() {
            self.gone = Gone::Armed;
        }
        true
    }

    fn navigation_wheel(&mut self, delta: egui::Vec2, phase: TouchPhase) {
        self.navigating = matches!(phase, TouchPhase::Start | TouchPhase::Move);
        self.push(
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: self.modifiers,
            },
            false,
        );
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
        if self.owner.is_some() {
            self.cancel_owner();
        }
        self.push(Event::PointerMoved(pos), false);
        self.push(self.button(pos, true), false);
        self.push(self.button(pos, false), false);
        self.gone = Gone::Armed;
    }

    pub fn push_event(&mut self, event: Event) {
        self.push(event, false);
    }

    /// The frame's events. Returns whether another frame is needed to deliver a
    /// deferred `PointerGone`.
    pub fn drain(&mut self) -> (Vec<Event>, bool) {
        let mut events = Vec::with_capacity(self.pending.len() + 1);
        // A new press, hover or navigation since arming resets `gone` to Idle.
        if self.gone == Gone::Due {
            self.gone = Gone::Idle;
            if self.owner.is_none() {
                events.push(Event::PointerGone);
            }
        }
        events.extend(self.pending.drain(..).map(|item| item.event));
        let again = self.gone == Gone::Armed;
        if again {
            self.gone = Gone::Due;
        }
        (events, again)
    }

    /// Latest prediction for a live contact: transient overlay input only.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn prediction(&self, contact: u64) -> Option<Prediction> {
        self.predictions.get(&contact).copied()
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
