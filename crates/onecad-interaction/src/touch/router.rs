//! INPUT.md ownership table: one owner per contact for its whole life, and at
//! most one pointer stream (`Author`, `Control`) at a time because hosts have a
//! single pointer. A control finger arriving while that pointer is held still
//! works: it activates the control on lift (`RoutedEvent::Activate`) without
//! the pointer. Finger arbitration lives in `router_fingers.rs`.
use super::{
    CancelReason, ContactId, ContactKind, GesturePhase, Normalized, Overlay, Owner, Point,
    RoutedEvent, Sample, TAP_SLOP_PT,
};
use std::collections::{BTreeMap, BTreeSet};

/// Per-frame routing input, computed from the previous host frame.
pub struct RouteContext<'a> {
    /// When true, canvas fingers never author or synthesize taps; drags orbit.
    /// Controls and precise Pencil/indirect-pointer contacts remain usable.
    pub pencil_only_drawing: bool,
    /// A sketch tool, handle or parameter drag can use a dragged pointer.
    pub drag_authoring: bool,
    /// The point is over a control rather than the CAD canvas.
    pub over_control: &'a dyn Fn(Point) -> bool,
}

impl RouteContext<'_> {
    /// Generic canvas routing: fingers orbit and tap. Hosts may require Pencil.
    pub const CANVAS: RouteContext<'static> = RouteContext {
        pencil_only_drawing: false,
        drag_authoring: false,
        over_control: &|_| false,
    };
}

pub(super) struct Track {
    pub kind: ContactKind,
    /// `None` while a canvas finger is still undecided (tap, author or orbit).
    pub owner: Option<Owner>,
    /// A pressed contact was cancelled; it emits nothing more.
    pub cancelled: bool,
    /// A `Control` finger that arrived while another contact held the pointer:
    /// it never presses and activates its control on lift instead.
    pub pointerless: bool,
    pub canvas: bool,
    pub begin: Point,
    pub begin_time: f64,
    pub last: Point,
    pub tap_count: u32,
}

impl Track {
    /// Owns the host's single pointer: pressed, not cancelled.
    fn holds_pointer(&self) -> bool {
        matches!(self.owner, Some(Owner::Author | Owner::Control))
            && !self.cancelled
            && !self.pointerless
    }
}

#[derive(Default)]
pub struct Router {
    pub(super) tracks: BTreeMap<ContactId, Track>,
    hovering: Option<ContactKind>,
    predicted: BTreeSet<ContactId>,
    /// Newest Pencil position (contact or hover), kept after it lifts.
    last_pencil: Option<Point>,
}

impl Router {
    pub fn route(&mut self, events: Vec<Normalized>, ctx: &RouteContext<'_>) -> Vec<RoutedEvent> {
        let mut out = Vec::new();
        for event in events {
            match event {
                Normalized::Contact(sample) => self.contact(&sample, ctx, &mut out),
                Normalized::Hover(sample) => self.hover(&sample, &mut out),
                Normalized::Prediction { contact, points } => {
                    self.prediction(contact, points, &mut out);
                }
                Normalized::Cancel { contact, reason } => self.cancel(contact, reason, &mut out),
            }
        }
        out
    }

    /// The owner of a live contact (`None` while undecided or unknown).
    pub fn owner(&self, contact: ContactId) -> Option<Owner> {
        self.tracks.get(&contact).and_then(|track| track.owner)
    }

    /// A Pencil or pointer is authoring: the camera must hold still.
    pub fn precise_author_live(&self) -> bool {
        self.tracks.values().any(|track| {
            track.kind.precise() && track.owner == Some(Owner::Author) && !track.cancelled
        })
    }

    /// The newest Pencil position seen (contact or hover), for accessory placement.
    pub fn pencil_position(&self) -> Option<Point> {
        self.last_pencil
    }

    pub(super) fn pointer_live(&self) -> bool {
        self.tracks.values().any(Track::holds_pointer)
    }

    fn contact(&mut self, sample: &Sample, ctx: &RouteContext<'_>, out: &mut Vec<RoutedEvent>) {
        use super::Phase;
        let Some(position) = sample.position else {
            return;
        };
        if sample.kind == ContactKind::Pencil {
            self.last_pencil = Some(position);
        }
        match sample.phase {
            Phase::Began if sample.kind.precise() => self.precise_began(sample, position, out),
            Phase::Began => self.finger_began(sample, position, ctx, out),
            Phase::Moved | Phase::Stationary => self.moved(sample.contact, position, ctx, out),
            Phase::Ended => self.ended(sample, position, ctx, out),
            Phase::Cancelled => self.cancel(sample.contact, CancelReason::System, out),
            Phase::HoverMoved | Phase::HoverExited => {}
        }
    }

    /// Pencil and pointer contacts author anywhere and take the pointer from a finger.
    fn precise_began(&mut self, sample: &Sample, position: Point, out: &mut Vec<RoutedEvent>) {
        let precise_live = self.tracks.values().any(|track| {
            track.kind.precise()
                && matches!(track.owner, Some(Owner::Author | Owner::Control))
                && !track.cancelled
        });
        let owner = if precise_live {
            Owner::Ignored
        } else {
            self.yield_fingers(out);
            Owner::Author
        };
        self.insert(sample, position, Some(owner), false);
        if owner == Owner::Author {
            self.hovering = None;
            out.push(RoutedEvent::Press {
                contact: sample.contact,
                owner,
                kind: sample.kind,
                position,
                tap_count: sample.tap_count,
            });
        }
    }

    /// A Pencil landed: finger pointers are cancelled, canvas fingers stop
    /// navigating and are ignored until lifted (the camera holds still).
    fn yield_fingers(&mut self, out: &mut Vec<RoutedEvent>) {
        for (contact, track) in self.tracks.iter_mut() {
            if track.kind != ContactKind::Finger || track.cancelled || track.pointerless {
                continue;
            }
            match track.owner {
                Some(Owner::Author | Owner::Control) => {
                    track.cancelled = true;
                    out.push(RoutedEvent::Cancel {
                        contact: *contact,
                        reason: CancelReason::PencilPrecedence,
                    });
                }
                Some(Owner::Orbit) => {
                    track.owner = Some(Owner::Ignored);
                    out.push(orbit(
                        *contact,
                        GesturePhase::End,
                        track.last,
                        Point::new(0.0, 0.0),
                    ));
                }
                Some(Owner::MultiTouch) => {
                    track.owner = Some(Owner::Ignored);
                    out.push(RoutedEvent::MultiTouch {
                        contact: *contact,
                        phase: GesturePhase::End,
                    });
                }
                None => track.owner = Some(Owner::Ignored),
                Some(Owner::Tap | Owner::Ignored) => {}
            }
        }
    }

    pub(super) fn insert(
        &mut self,
        sample: &Sample,
        position: Point,
        owner: Option<Owner>,
        canvas: bool,
    ) {
        self.tracks.insert(
            sample.contact,
            Track {
                kind: sample.kind,
                owner,
                cancelled: false,
                pointerless: false,
                canvas,
                begin: position,
                begin_time: sample.timestamp,
                last: position,
                tap_count: sample.tap_count,
            },
        );
    }

    fn moved(
        &mut self,
        contact: ContactId,
        position: Point,
        ctx: &RouteContext<'_>,
        out: &mut Vec<RoutedEvent>,
    ) {
        let Some(track) = self.tracks.get_mut(&contact) else {
            return;
        };
        let previous = std::mem::replace(&mut track.last, position);
        match track.owner {
            Some(owner @ (Owner::Author | Owner::Control)) if track.holds_pointer() => {
                out.push(RoutedEvent::Move {
                    contact,
                    owner,
                    position,
                });
            }
            Some(Owner::Orbit) if previous != position => {
                let delta = Point::new(position.x - previous.x, position.y - previous.y);
                out.push(orbit(contact, GesturePhase::Move, position, delta));
            }
            None => self.finger_slop(contact, position, ctx, out),
            _ => {}
        }
    }

    fn ended(
        &mut self,
        sample: &Sample,
        position: Point,
        ctx: &RouteContext<'_>,
        out: &mut Vec<RoutedEvent>,
    ) {
        let contact = sample.contact;
        if self.tracks.get(&contact).is_some_and(|t| t.owner.is_none()) {
            self.finger_ended(sample, position, ctx, out);
            return;
        }
        let Some(track) = self.tracks.remove(&contact) else {
            return;
        };
        match track.owner {
            Some(Owner::Control) if track.pointerless => {
                // Lifted within the slop: a tap on the control, not a drag off it.
                if track.begin.distance(position) <= TAP_SLOP_PT {
                    out.push(RoutedEvent::Activate {
                        contact,
                        position: track.begin,
                    });
                }
            }
            Some(owner @ (Owner::Author | Owner::Control)) if !track.cancelled => {
                out.push(RoutedEvent::Release {
                    contact,
                    owner,
                    kind: track.kind,
                    position,
                });
            }
            Some(Owner::Orbit) => out.push(orbit(contact, GesturePhase::End, position, zero())),
            Some(Owner::MultiTouch) => self.multi_touch_left(contact, GesturePhase::End, out),
            _ => {}
        }
        self.forget_prediction(contact, out);
    }

    /// Cancellation never releases: a pressed contact reports `Cancel`.
    fn cancel(&mut self, contact: ContactId, reason: CancelReason, out: &mut Vec<RoutedEvent>) {
        let Some(track) = self.tracks.remove(&contact) else {
            return;
        };
        match track.owner {
            Some(Owner::Author | Owner::Control) if track.holds_pointer() => {
                out.push(RoutedEvent::Cancel { contact, reason });
            }
            Some(Owner::Orbit) => {
                out.push(orbit(contact, GesturePhase::Cancel, track.last, zero()));
            }
            Some(Owner::MultiTouch) => self.multi_touch_left(contact, GesturePhase::Cancel, out),
            _ => {}
        }
        self.forget_prediction(contact, out);
    }

    /// Hover moves the pointer without a button while no pointer stream is live;
    /// exit clears hover only.
    fn hover(&mut self, sample: &Sample, out: &mut Vec<RoutedEvent>) {
        if !sample.kind.precise() || self.pointer_live() {
            return;
        }
        match (sample.phase, sample.position) {
            (super::Phase::HoverMoved, Some(position)) => {
                self.hovering = Some(sample.kind);
                if sample.kind == ContactKind::Pencil {
                    self.last_pencil = Some(position);
                }
                out.push(RoutedEvent::Hover {
                    kind: sample.kind,
                    position,
                });
            }
            (super::Phase::HoverExited, _) => {
                // Reports the kind that was hovering, not the exit sample's.
                if let Some(kind) = self.hovering.take() {
                    out.push(RoutedEvent::HoverExit { kind });
                }
            }
            _ => {}
        }
    }

    /// Predictions reach hosts only as overlays of an authoring contact.
    fn prediction(&mut self, contact: ContactId, points: Vec<Point>, out: &mut Vec<RoutedEvent>) {
        let authoring = self
            .tracks
            .get(&contact)
            .is_some_and(|t| t.owner == Some(Owner::Author) && !t.cancelled);
        if points.is_empty() {
            self.forget_prediction(contact, out);
        } else if authoring {
            self.predicted.insert(contact);
            out.push(RoutedEvent::Overlay(Overlay::Prediction {
                contact,
                points,
            }));
        }
    }

    pub(super) fn forget_prediction(&mut self, contact: ContactId, out: &mut Vec<RoutedEvent>) {
        if self.predicted.remove(&contact) {
            out.push(RoutedEvent::Overlay(Overlay::Prediction {
                contact,
                points: Vec::new(),
            }));
        }
    }
}

pub(super) fn orbit(
    contact: ContactId,
    phase: GesturePhase,
    position: Point,
    delta: Point,
) -> RoutedEvent {
    RoutedEvent::Orbit {
        contact,
        phase,
        position,
        delta,
    }
}

pub(super) fn zero() -> Point {
    Point::new(0.0, 0.0)
}
