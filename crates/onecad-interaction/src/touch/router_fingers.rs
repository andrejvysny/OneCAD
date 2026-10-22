//! Finger rows of the INPUT.md ownership table: controls, taps, slop exit to
//! authoring or orbit, and multi-touch hand-off to native navigation.
use super::router::{orbit, zero, RouteContext, Router};
use super::{
    CancelReason, ContactId, ContactKind, GesturePhase, Owner, Point, RoutedEvent, Sample,
    TAP_MAX_SECONDS, TAP_SLOP_PT,
};

impl Router {
    pub(super) fn finger_began(
        &mut self,
        sample: &Sample,
        position: Point,
        ctx: &RouteContext<'_>,
        out: &mut Vec<RoutedEvent>,
    ) {
        if (ctx.over_control)(position) {
            // Controls keep working beside a Pencil, but the host has one pointer.
            let owner = if self.pointer_live() {
                Owner::Ignored
            } else {
                Owner::Control
            };
            self.insert(sample, position, Some(owner), false);
            if owner == Owner::Control {
                out.push(press(sample, owner, position));
            }
            return;
        }
        if self.precise_author_live() {
            self.insert(sample, position, Some(Owner::Ignored), true);
            return;
        }
        // An ignored finger stays ignored until lifted, so it does not pair up.
        let other_finger = self.tracks.values().any(|track| {
            track.kind == ContactKind::Finger
                && track.canvas
                && !track.cancelled
                && track.owner != Some(Owner::Ignored)
        });
        self.insert(sample, position, None, true);
        if other_finger {
            self.join_multi_touch(CancelReason::MultiTouch, out);
        }
    }

    /// An undecided finger left the slop: it authors when a drag can use the
    /// pointer, otherwise it orbits from the exit point (no jump).
    pub(super) fn finger_slop(
        &mut self,
        contact: ContactId,
        position: Point,
        ctx: &RouteContext<'_>,
        out: &mut Vec<RoutedEvent>,
    ) {
        let Some(track) = self.tracks.get(&contact) else {
            return;
        };
        if track.begin.distance(position) <= TAP_SLOP_PT {
            return;
        }
        let begin = track.begin;
        let author = ctx.drag_authoring && !ctx.pencil_only_drawing;
        let pointer_free = !self.pointer_live();
        let Some(track) = self.tracks.get_mut(&contact) else {
            return;
        };
        if author && pointer_free {
            track.owner = Some(Owner::Author);
            out.push(RoutedEvent::Press {
                contact,
                owner: Owner::Author,
                kind: ContactKind::Finger,
                position: begin,
                tap_count: track.tap_count,
            });
            out.push(RoutedEvent::Move {
                contact,
                owner: Owner::Author,
                position,
            });
        } else if author {
            track.owner = Some(Owner::Ignored);
        } else {
            track.owner = Some(Owner::Orbit);
            out.push(orbit(contact, GesturePhase::Start, position, zero()));
        }
    }

    /// An undecided finger lifted: a quick one within the slop is a tap at its
    /// begin position; a long press does nothing.
    pub(super) fn finger_ended(
        &mut self,
        sample: &Sample,
        position: Point,
        ctx: &RouteContext<'_>,
        out: &mut Vec<RoutedEvent>,
    ) {
        let contact = sample.contact;
        self.finger_slop(contact, position, ctx, out);
        let Some(track) = self.tracks.get(&contact) else {
            return;
        };
        if track.owner.is_some() {
            // The end itself left the slop; finish as the decided owner.
            self.ended_decided(sample, position, out);
            return;
        }
        let quick = sample.timestamp - track.begin_time <= TAP_MAX_SECONDS;
        let begin = track.begin;
        let tap_count = track.tap_count;
        self.tracks.remove(&contact);
        if quick && !self.pointer_live() {
            out.push(RoutedEvent::Press {
                contact,
                owner: Owner::Tap,
                kind: ContactKind::Finger,
                position: begin,
                tap_count,
            });
            out.push(RoutedEvent::Release {
                contact,
                owner: Owner::Tap,
                kind: ContactKind::Finger,
                position: begin,
            });
        }
    }

    fn ended_decided(&mut self, sample: &Sample, position: Point, out: &mut Vec<RoutedEvent>) {
        let Some(track) = self.tracks.remove(&sample.contact) else {
            return;
        };
        match track.owner {
            Some(Owner::Author) => out.push(RoutedEvent::Release {
                contact: sample.contact,
                owner: Owner::Author,
                kind: track.kind,
                position,
            }),
            Some(Owner::Orbit) => {
                out.push(orbit(sample.contact, GesturePhase::End, position, zero()))
            }
            _ => {}
        }
        self.forget_prediction(sample.contact, out);
    }

    /// Two or more canvas fingers: all become `MultiTouch` (native pan/pinch
    /// navigates); an authoring finger is cancelled and an orbit ends in place.
    fn join_multi_touch(&mut self, reason: CancelReason, out: &mut Vec<RoutedEvent>) {
        for (contact, track) in self.tracks.iter_mut() {
            if track.kind != ContactKind::Finger || !track.canvas {
                continue;
            }
            match track.owner {
                Some(Owner::Author) if !track.cancelled => {
                    track.cancelled = true;
                    out.push(RoutedEvent::Cancel {
                        contact: *contact,
                        reason,
                    });
                    continue;
                }
                Some(Owner::Orbit) => {
                    out.push(orbit(*contact, GesturePhase::End, track.last, zero()));
                }
                Some(Owner::MultiTouch | Owner::Ignored) => continue,
                Some(Owner::Author | Owner::Control | Owner::Tap) => continue,
                None => {}
            }
            track.owner = Some(Owner::MultiTouch);
            out.push(RoutedEvent::MultiTouch {
                contact: *contact,
                phase: GesturePhase::Start,
            });
        }
    }

    /// A multi-touch finger left; a single remaining one is ignored until lifted.
    pub(super) fn multi_touch_left(
        &mut self,
        contact: ContactId,
        phase: GesturePhase,
        out: &mut Vec<RoutedEvent>,
    ) {
        out.push(RoutedEvent::MultiTouch { contact, phase });
        let remaining: Vec<ContactId> = self
            .tracks
            .iter()
            .filter(|(_, track)| track.owner == Some(Owner::MultiTouch))
            .map(|(id, _)| *id)
            .collect();
        if let [last] = remaining[..] {
            if let Some(track) = self.tracks.get_mut(&last) {
                track.owner = Some(Owner::Ignored);
            }
            out.push(RoutedEvent::MultiTouch {
                contact: last,
                phase: GesturePhase::End,
            });
        }
    }

    /// The host's native pan/pinch recognizer began. Returns whether it may move
    /// the camera: never while a Pencil or pointer authors. Otherwise canvas
    /// fingers hand over to it.
    pub fn native_navigation(&mut self) -> (bool, Vec<RoutedEvent>) {
        if self.precise_author_live() {
            return (false, Vec::new());
        }
        let mut out = Vec::new();
        self.join_multi_touch(CancelReason::MultiTouch, &mut out);
        (true, out)
    }
}

fn press(sample: &Sample, owner: Owner, position: Point) -> RoutedEvent {
    RoutedEvent::Press {
        contact: sample.contact,
        owner,
        kind: sample.kind,
        position,
        tap_count: sample.tap_count,
    }
}
