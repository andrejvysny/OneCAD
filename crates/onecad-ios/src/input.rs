//! UIKit samples to egui events, queued until the next frame.
//!
//! First version of the routing contract (`docs/ios/INPUT.md` owns the full
//! router): the first contact that begins while none is active owns egui's
//! primary pointer, and a Pencil that begins while a finger owns it takes over;
//! other contacts are left to the native gesture recognizers (`navigation`).
//! Actual and coalesced samples are replayed in chronological order; predicted
//! samples are kept for the owner as a transient overlay and never become events.
use crate::ffi::{self, TouchSample};
use egui::{Event, PointerButton, Pos2};
use std::collections::{HashMap, HashSet};

#[path = "input_navigation.rs"]
mod navigation;

pub const BATCH_MAX: usize = 4096;
pub const PENDING_MAX: usize = 16_384;

struct Item {
    event: Event,
    /// Intermediate movement that may be coalesced away under pressure.
    disposable: bool,
    /// The owner contact whose pointer stream produced this event.
    contact: Option<u64>,
}

#[derive(Clone, Copy)]
struct Owner {
    contact: u64,
    kind: u32,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prediction {
    pub position: Pos2,
    pub timestamp: f64,
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
}

pub struct Input {
    pending: Vec<Item>,
    owner: Option<Owner>,
    hovering: bool,
    gone: Gone,
    forget_pointer: bool,
    navigating: bool,
    /// The current navigation gesture is ignored (a Pencil or pointer owns the
    /// canvas, so the camera is held stable).
    navigation_ignored: bool,
    /// Start of the trailing queued navigation `Move` group, which later moves
    /// of the same gesture merge into.
    navigation_tail: Option<usize>,
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
            forget_pointer: false,
            navigating: false,
            navigation_ignored: false,
            navigation_tail: None,
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

type SampleKey = (u64, u64, u64, u64);

fn key(sample: &TouchSample) -> SampleKey {
    (
        sample.contact_id,
        sample.timestamp_s.to_bits(),
        sample.x_pt.to_bits(),
        sample.y_pt.to_bits(),
    )
}

/// Drops prediction/estimation records and the coalesced copy of each actual
/// sample, then orders the rest by timestamp (stable for equal times).
fn chronological(samples: &[TouchSample]) -> Vec<TouchSample> {
    let actual: HashSet<SampleKey> = samples
        .iter()
        .filter(|sample| sample.provenance == ffi::SAMPLE_ACTUAL)
        .map(key)
        .collect();
    let mut ordered: Vec<TouchSample> = samples
        .iter()
        .filter(|sample| match sample.provenance {
            ffi::SAMPLE_ACTUAL => true,
            ffi::SAMPLE_COALESCED => !actual.contains(&key(sample)),
            _ => false,
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
        for sample in chronological(samples) {
            self.sample(&sample, stale);
        }
        // After the batch's phases: only a still-live owner keeps a prediction.
        let owner = self.owner.map(|owner| owner.contact);
        for sample in samples.iter().filter(|sample| {
            sample.provenance == ffi::SAMPLE_PREDICTED && !stale && Some(sample.contact_id) == owner
        }) {
            self.predictions.insert(
                sample.contact_id,
                Prediction {
                    position: position(sample),
                    timestamp: sample.timestamp_s,
                },
            );
        }
        self.enforce_bound()
    }

    fn sample(&mut self, sample: &TouchSample, stale: bool) {
        let finite = sample.x_pt.is_finite() && sample.y_pt.is_finite();
        let owned = self.owner.is_some_and(|o| o.contact == sample.contact_id);
        let usable = finite && !stale;
        match sample.phase {
            ffi::PHASE_BEGAN if usable && self.pencil_takes_over(sample) => {
                self.cancel_owner();
                self.press(sample);
            }
            ffi::PHASE_BEGAN if self.owner.is_none() && usable => self.press(sample),
            ffi::PHASE_MOVED | ffi::PHASE_STATIONARY if owned && usable => self.drag(sample),
            ffi::PHASE_ENDED if owned && usable => self.release(sample),
            // A stale-coordinate end cannot commit at a meaningful point.
            ffi::PHASE_ENDED | ffi::PHASE_CANCELLED if owned => self.cancel_owner(),
            ffi::PHASE_ENDED | ffi::PHASE_CANCELLED => {
                self.predictions.remove(&sample.contact_id);
            }
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

    /// Pencil contacts own authoring: a Pencil landing while a finger (or palm)
    /// owns the pointer cancels that finger instead of being ignored.
    fn pencil_takes_over(&self, sample: &TouchSample) -> bool {
        sample.kind == ffi::CONTACT_PENCIL
            && self
                .owner
                .is_some_and(|owner| owner.kind == ffi::CONTACT_FINGER)
    }

    fn press(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        self.owner = Some(Owner {
            contact: sample.contact_id,
            kind: sample.kind,
            last: pos,
            delivered: false,
        });
        self.gone = Gone::Idle;
        self.push_owned(Event::PointerMoved(pos), false);
        self.push_owned(self.button(pos, true), false);
    }

    fn drag(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        if let Some(owner) = &mut self.owner
            && owner.last != pos
        {
            owner.last = pos;
            self.push_owned(Event::PointerMoved(pos), true);
        }
    }

    fn release(&mut self, sample: &TouchSample) {
        let pos = position(sample);
        let finger = self.owner.is_some_and(|o| o.kind == ffi::CONTACT_FINGER);
        self.push_owned(Event::PointerMoved(pos), false);
        self.push_owned(self.button(pos, false), false);
        self.predictions.remove(&sample.contact_id);
        self.owner = None;
        // A finger cannot hover after lifting; Pencil and pointers report hover exit.
        if finger {
            self.gone = Gone::Armed;
        }
    }

    /// Cancellation never delivers a release, so it can neither click nor commit a
    /// drag: the contact's queued pointer events are dropped, and a press egui has
    /// already seen is forgotten before the next frame, which carries
    /// `PointerGone` so the editor's canvas gestures cancel.
    fn cancel_owner(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        self.predictions.remove(&owner.contact);
        self.pending
            .retain(|item| item.contact != Some(owner.contact));
        self.navigation_tail = None;
        if owner.delivered {
            self.forget_pointer = true;
            self.push(Event::PointerGone, false);
        }
        self.gone = Gone::Idle;
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
        self.pending.push(Item {
            event,
            disposable,
            contact: None,
        });
    }

    fn push_owned(&mut self, event: Event, disposable: bool) {
        self.navigation_tail = None;
        let contact = self.owner.map(|owner| owner.contact);
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
        true
    }

    /// System gesture or interruption: every live contact is cancelled.
    pub fn cancel_all(&mut self) {
        self.cancel_owner();
        self.cancel_navigation();
        self.predictions.clear();
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
        self.cancel_owner();
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
            if self.owner.is_none() {
                events.push(Event::PointerGone);
            }
        }
        events.extend(self.pending.drain(..).map(|item| item.event));
        self.navigation_tail = None;
        if let Some(owner) = &mut self.owner {
            owner.delivered = true;
        }
        let again = self.gone == Gone::Armed;
        if again {
            self.gone = Gone::Due;
        }
        Drained {
            events,
            again,
            cancelled: std::mem::take(&mut self.forget_pointer),
        }
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
