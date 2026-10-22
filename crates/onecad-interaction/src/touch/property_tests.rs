//! Randomized interleavings (fixed seeds, no dependencies): every pressed
//! contact presses once and ends in exactly one release or cancellation, never
//! a release after a cancellation; predicted positions never become pointer
//! events; one pointer at a time; hover never moves a held pointer. Scripts mix
//! hover, estimation updates, out-of-order timestamps, duplicate begins, id
//! reuse after cancel-all, generation changes and pending overflow.
use super::test_support::*;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn coin(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Predicted samples sit far outside the actual coordinate range.
const PREDICTED_X: f64 = 10_000.0;

struct Script {
    rng: Rng,
    t: f64,
    next_contact: ContactId,
    live: BTreeMap<ContactId, (ContactKind, Point)>,
    /// Ids whose contact the script still thinks live after a cancel-all; a
    /// later `began` may reuse one.
    retired: Vec<ContactId>,
    generation: u64,
}

impl Script {
    fn batch(&mut self) -> Vec<Sample> {
        let mut out = Vec::new();
        for _ in 0..=self.rng.below(6) {
            self.t += 0.01 * (1 + self.rng.below(20)) as f64;
            match self.rng.below(20) {
                0 => self.hover(&mut out),
                1 => self.noise(&mut out),
                _ => self.step(&mut out),
            }
        }
        out
    }

    /// Pencil or pointer hover with no contact down, sometimes exiting.
    fn hover(&mut self, out: &mut Vec<Sample>) {
        let kind = if self.rng.coin(50) {
            ContactKind::Pencil
        } else {
            ContactKind::IndirectPointer
        };
        let phase = if self.rng.coin(20) {
            Phase::HoverExited
        } else {
            Phase::HoverMoved
        };
        let (x, y) = (self.rng.below(400) as f64, self.rng.below(400) as f64);
        out.push(sample(0, kind, phase, Provenance::Actual, self.t, x, y));
    }

    /// Estimation updates, stale timestamps and duplicate begins of live ids.
    fn noise(&mut self, out: &mut Vec<Sample>) {
        let Some((&contact, &(kind, at))) = self.live.iter().next() else {
            return;
        };
        let (phase, provenance, t) = match self.rng.below(3) {
            0 => (Phase::Moved, Provenance::EstimationUpdate, self.t),
            1 => (Phase::Moved, Provenance::Actual, self.t - 1.0),
            _ => (Phase::Began, Provenance::Actual, self.t),
        };
        let mut noisy = sample(contact, kind, phase, provenance, t, at.x + 3.0, at.y);
        noisy.estimation_index = Some(self.rng.below(8));
        out.push(noisy);
    }

    fn step(&mut self, out: &mut Vec<Sample>) {
        let begin = self.live.is_empty() || (self.live.len() < 4 && self.rng.coin(25));
        if begin {
            let kind = match self.rng.below(5) {
                0 | 1 => ContactKind::Pencil,
                2 => ContactKind::IndirectPointer,
                _ => ContactKind::Finger,
            };
            let at = p(self.rng.below(400) as f64, self.rng.below(400) as f64);
            let contact = if !self.retired.is_empty() && self.rng.coin(30) {
                self.retired.remove(0)
            } else {
                self.next_contact += 1;
                self.next_contact - 1
            };
            self.live.insert(contact, (kind, at));
            out.push(sample(
                contact,
                kind,
                Phase::Began,
                Provenance::Actual,
                self.t,
                at.x,
                at.y,
            ));
            return;
        }
        let index = self.rng.below(self.live.len() as u64) as usize;
        let contact = *self.live.keys().nth(index).expect("live contact");
        let (kind, at) = self.live[&contact];
        let next = p(
            at.x + self.rng.below(41) as f64 - 20.0,
            at.y + self.rng.below(41) as f64 - 20.0,
        );
        let phase = match self.rng.below(10) {
            0 => Phase::Ended,
            1 => Phase::Cancelled,
            2 => Phase::Stationary,
            _ => Phase::Moved,
        };
        let provenance = if phase == Phase::Moved && self.rng.coin(30) {
            Provenance::Coalesced
        } else {
            Provenance::Actual
        };
        out.push(sample(
            contact, kind, phase, provenance, self.t, next.x, next.y,
        ));
        if phase.terminal() {
            self.live.remove(&contact);
        } else {
            self.live.insert(contact, (kind, next));
            if self.rng.coin(40) {
                let t = self.t + 0.016;
                out.push(sample(
                    contact,
                    kind,
                    Phase::Moved,
                    Provenance::Predicted,
                    t,
                    PREDICTED_X,
                    next.y,
                ));
            }
        }
    }
}

/// Pointer lives per contact id; a reused id starts a new life only after the
/// previous one ended.
#[derive(Default)]
struct Ledger {
    /// Pressed contacts whose life has not ended.
    open: BTreeSet<ContactId>,
    /// How each id's latest life ended (`true`: cancelled).
    ended: BTreeMap<ContactId, bool>,
    /// The contact currently holding the host pointer.
    pointer: Option<ContactId>,
}

impl Ledger {
    fn check(&mut self, event: &RoutedEvent) {
        match event {
            RoutedEvent::Press {
                contact, position, ..
            } => {
                assert_ne!(position.x, PREDICTED_X, "prediction became a press");
                assert!(self.open.insert(*contact), "c{contact} pressed twice");
                self.ended.remove(contact);
                assert!(
                    self.pointer.is_none(),
                    "c{contact} pressed while {:?} holds the pointer",
                    self.pointer
                );
                self.pointer = Some(*contact);
            }
            RoutedEvent::Move {
                contact, position, ..
            } => {
                assert_ne!(position.x, PREDICTED_X, "prediction became a move");
                assert_eq!(self.pointer, Some(*contact), "move without the pointer");
            }
            RoutedEvent::Release {
                contact, position, ..
            } => {
                assert_ne!(position.x, PREDICTED_X, "prediction became a release");
                self.end(*contact, false);
            }
            RoutedEvent::Hover { .. } => {
                assert!(self.pointer.is_none(), "hover moved a held pointer");
            }
            RoutedEvent::Cancel { contact, .. } => self.end(*contact, true),
            RoutedEvent::Activate { contact, position } => {
                assert_ne!(position.x, PREDICTED_X, "prediction became an activation");
                assert!(
                    !self.open.contains(contact),
                    "c{contact} activated while it holds a press"
                );
            }
            _ => {}
        }
    }

    /// A life ends once: no release after a cancel, no second end.
    fn end(&mut self, contact: ContactId, cancelled: bool) {
        assert!(
            self.open.remove(&contact),
            "c{contact} ended without a live press (previous end: {:?})",
            self.ended.get(&contact)
        );
        self.ended.insert(contact, cancelled);
        assert_eq!(self.pointer.take(), Some(contact));
    }

    /// After every contact ended: each life ended exactly once, no pointer.
    fn settled(&self) {
        assert!(self.open.is_empty(), "{:?} never ended", self.open);
        assert!(
            self.pointer.is_none(),
            "{:?} still holds the pointer",
            self.pointer
        );
    }
}

fn context(rng: &mut Rng) -> RouteContext<'static> {
    let over_control: &'static dyn Fn(Point) -> bool = if rng.coin(50) {
        &|point| point.x < 80.0
    } else {
        &|_| false
    };
    RouteContext {
        pencil_only_drawing: rng.coin(30),
        drag_authoring: rng.coin(50),
        over_control,
    }
}

fn run(seed: u64) {
    let mut script = Script {
        rng: Rng(seed),
        t: 0.0,
        next_contact: 1,
        live: BTreeMap::new(),
        retired: Vec::new(),
        generation: 0,
    };
    let mut normalizer = Normalizer::default();
    let mut router = Router::default();
    let mut ledger = Ledger::default();
    for _ in 0..200 {
        let ctx = context(&mut script.rng);
        let mut events = Vec::new();
        match script.rng.below(40) {
            0 => {
                normalizer.cancel_all(CancelReason::SystemGesture);
                // The host may never report these ends; their ids get reused.
                let ids: Vec<_> = std::mem::take(&mut script.live).into_keys().collect();
                script.retired.extend(ids);
            }
            1 => {
                script.generation += 1;
                script.live.clear();
            }
            2 => events.extend(router.native_navigation().1),
            _ => {}
        }
        let batch = script.batch();
        normalizer
            .submit(&batch, script.generation)
            .expect("small batch");
        // Now and then the host skips draining, so the pending bound merges.
        if script.rng.coin(90) {
            events.extend(router.route(normalizer.drain(), &ctx));
        }
        for event in &events {
            ledger.check(event);
        }
    }
    let ends: Vec<Sample> = std::mem::take(&mut script.live)
        .into_iter()
        .map(|(contact, (kind, at))| {
            let phase = Phase::Ended;
            sample(
                contact,
                kind,
                phase,
                Provenance::Actual,
                script.t + 1.0,
                at.x,
                at.y,
            )
        })
        .collect();
    normalizer.submit(&ends, script.generation).expect("ends");
    normalizer.cancel_all(CancelReason::SceneInactive);
    for event in router.route(normalizer.drain(), &RouteContext::CANVAS) {
        ledger.check(&event);
    }
    ledger.settled();
}

/// A host that stops draining overflows the pending bound: intermediate moves
/// merge, while every began, end and cancellation survives.
#[test]
fn overflow_merges_moves_but_keeps_every_terminal_event() {
    let mut normalizer = Normalizer::default();
    let mut batches = 0;
    let mut t = 0.0;
    for contact in 0..4 {
        normalizer
            .submit(&[pencil(contact, Phase::Began, t, 0.0, 0.0)], 0)
            .expect("began");
    }
    while normalizer.stats().merged_moves == 0 {
        let batch: Vec<Sample> = (0..MAX_BATCH)
            .map(|i| {
                t += 0.001;
                pencil(i as u64 % 4, Phase::Moved, t, i as f64, 0.0)
            })
            .collect();
        normalizer.submit(&batch, 0).expect("batch");
        batches += 1;
        assert!(batches < 16, "the bound must merge before 16 batches");
    }
    for contact in 0..4 {
        normalizer
            .submit(&[pencil(contact, Phase::Ended, t + 1.0, 0.0, 0.0)], 0)
            .expect("end");
    }
    let pending = normalizer.drain();
    assert!(pending.len() <= MAX_PENDING + 8);
    let terminal = |phase: Phase| {
        pending
            .iter()
            .filter(|e| matches!(e, Normalized::Contact(s) if s.phase == phase))
            .count()
    };
    assert_eq!((terminal(Phase::Began), terminal(Phase::Ended)), (4, 4));
}

#[test]
fn random_interleavings_keep_the_pointer_invariants() {
    for seed in 1..=300u64 {
        run(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    }
}
