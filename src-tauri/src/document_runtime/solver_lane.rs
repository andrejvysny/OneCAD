//! SP-0 D3 — per-sketch solver-lane claims, held by RAII.
//!
//! # The race
//!
//! Two app-layer paths write the worker's per-sketch `SketchStore` entry
//! (`worker/src/session/SketchStore.h`) **with the runtime lock released**:
//!
//! * [`prepare_sketch_regions`](super::DocumentRuntime::prepare_sketch_regions)
//!   clones the authoritative sketch under the lock, then
//!   [`drive`](super::PreparedSketchRegions::drive)s a `SketchUpsert` +
//!   `SketchRegions` round-trip **without** it;
//! * [`begin_gesture`](super::DocumentRuntime::begin_gesture) upserts the sketch
//!   and opens a worker `Gesture` on it.
//!
//! `prepare_sketch_regions` already refused a prepare while a gesture was open,
//! but that is a check-then-act: the refusal reads `active_gesture` at prepare
//! time and the damage happens during the *unlocked* drive. A `beginGesture` that
//! lands in that window sees no in-flight query and sails through.
//!
//! # What the collision actually corrupts
//!
//! Not `SolveDrag`: the worker's `Gesture` owns a **private** `Sketch` clone
//! (`SolverLane.h` `Gesture::sketch`), so incremental drag solves never read the
//! store and a concurrent re-upsert cannot make them "apply onto stale state".
//!
//! The damage lands at `EndGesture` (`worker/src/protocol/SolverLane.cpp`
//! `on_end`, ~:368), which commits the drag by reading the store back:
//!
//! ```text
//! const std::uint64_t new_rev = g.sketch_revision + 1;         // captured at BeginGesture
//! if (auto s = store_.snapshot(g.sketch_id)) {
//!     wire::apply_solved_positions(s->wire_args, *g.sketch, g.index);
//!     store_.put(g.sketch_id, std::move(s->wire_args), new_rev);
//! }
//! ```
//!
//! So the solved drag positions are written onto **whatever the store happens to
//! hold** — i.e. the region query's stale re-upserted clone, which is the sketch as
//! it stood before the drag AND before any edit committed since the query was
//! prepared. And `SketchStore::put` assigns the revision *exactly*
//! (`s.revision = revision;`, `SketchStore.h`:57-62) rather than taking a max, so a
//! `new_rev` derived from a pre-query `sketch_revision` can **regress** the store's
//! counter — the one number every later fencing/staleness check reads.
//!
//! Making `put` monotonic is the worker-side half of this and is **DEFERRED**: it
//! is a SCHEMA §7.4 response-field semantic (`sketchRevision`) and needs a §14
//! entry plus cross-track sign-off. This module closes the Rust-side half, which
//! is what makes the collision unreachable in the first place.
//!
//! # Why RAII, and why per sketch
//!
//! A [`Claim`] is acquired **under the runtime lock** and lives exactly as long as
//! the writer that owns it: it is a field of `ActiveGesture` and of
//! [`PreparedSketchRegions`](super::PreparedSketchRegions), so it is released by
//! `Drop` when the gesture ends/cancels/is cleared and when the prepared query is
//! consumed after its unlocked drive. There is no release site to forget, and no
//! window between "checked" and "acted" — the claim spans the whole unlocked
//! stretch.
//!
//! The map is keyed by [`SketchId`] because the worker's store is: a region query
//! on sketch B and a drag on sketch A touch disjoint entries and must not block
//! each other.
//!
//! Region queries are **pure reads** of the same authoritative sketch, so several
//! may share a lane concurrently (today's behavior — the frontend fires
//! `getSketchRegions` from both `sketchStaticSync` and the extrude/revolve arm
//! paths, and making those mutually exclusive would be a new failure mode). A
//! gesture is exclusive against everything.
//!
//! # Seam (flagged, not fixed)
//!
//! The gesture verbs bypass the H2 plan-circuit breaker: `WorkerManager`'s
//! `SolverEngine` impl calls `client.request` directly rather than going through
//! the breaker the plan lane uses, so a drag opened while the circuit is open
//! still round-trips to a possibly-dead worker. Composing the two is out of scope
//! here; recorded so the next pass over this lane sees it.
//!
//! The mutex below is a plain `std::sync::Mutex` and is **never held across an
//! await** — every critical section is a map lookup.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use onecad_core::ids::SketchId;

/// Who holds a sketch's solver lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneOwner {
    /// An open drag gesture (`BeginGesture` … `EndGesture`). Exclusive.
    Gesture,
    /// An in-flight `getSketchRegions` read. Shared with other region queries.
    RegionQuery,
}

impl LaneOwner {
    /// Human phrasing for the refusal message.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Gesture => "a drag gesture",
            Self::RegionQuery => "a region query",
        }
    }
}

/// One lane's occupancy. `holders > 1` only ever happens for
/// [`LaneOwner::RegionQuery`].
#[derive(Debug, Clone, Copy)]
struct LaneState {
    owner: LaneOwner,
    holders: usize,
}

/// The per-document table of solver-lane claims. See the module docs.
#[derive(Debug, Default)]
pub struct SolverLaneClaims {
    lanes: Mutex<HashMap<SketchId, LaneState>>,
}

impl SolverLaneClaims {
    /// A fresh, empty claim table.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Claims `sketch` for `owner`.
    ///
    /// # Errors
    /// The [`LaneOwner`] already holding the lane, when the claim is refused. A
    /// gesture is exclusive; region queries share with each other.
    pub fn claim(self: &Arc<Self>, sketch: SketchId, owner: LaneOwner) -> Result<Claim, LaneOwner> {
        {
            let mut lanes = self.lock();
            match lanes.entry(sketch) {
                Entry::Vacant(slot) => {
                    slot.insert(LaneState { owner, holders: 1 });
                }
                Entry::Occupied(mut slot) => {
                    let state = slot.get_mut();
                    if owner != LaneOwner::RegionQuery || state.owner != LaneOwner::RegionQuery {
                        return Err(state.owner);
                    }
                    state.holders += 1;
                }
            }
        }
        Ok(Claim {
            claims: Arc::clone(self),
            sketch,
        })
    }

    /// The owner currently holding `sketch`'s lane, if any (diagnostics + tests).
    #[must_use]
    pub fn owner(&self, sketch: SketchId) -> Option<LaneOwner> {
        self.lock().get(&sketch).map(|state| state.owner)
    }

    /// A poisoned lane table is still perfectly usable: every critical section is
    /// an infallible map lookup, so no invariant can be left half-applied by a
    /// panic. Recovering also keeps [`Claim::drop`] from panicking during an
    /// unwind (which would abort the process).
    fn lock(&self) -> MutexGuard<'_, HashMap<SketchId, LaneState>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn release(&self, sketch: SketchId) {
        let mut lanes = self.lock();
        if let Entry::Occupied(mut slot) = lanes.entry(sketch) {
            let state = slot.get_mut();
            state.holders -= 1;
            if state.holders == 0 {
                slot.remove();
            }
        }
    }
}

/// An owned solver-lane claim. Dropping it releases the lane — the claim's
/// lifetime IS the writer's lifetime, so there is no release site to forget.
pub struct Claim {
    claims: Arc<SolverLaneClaims>,
    sketch: SketchId,
}

impl Claim {
    /// The sketch this claim holds.
    #[must_use]
    pub fn sketch(&self) -> SketchId {
        self.sketch
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.claims.release(self.sketch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sid(n: u128) -> SketchId {
        SketchId(Uuid::from_u128(n))
    }

    #[test]
    fn a_gesture_claim_is_exclusive_and_releases_on_drop() {
        let claims = SolverLaneClaims::new();
        let held = claims.claim(sid(1), LaneOwner::Gesture).expect("first");
        assert_eq!(claims.owner(sid(1)), Some(LaneOwner::Gesture));
        assert_eq!(
            claims.claim(sid(1), LaneOwner::RegionQuery).err(),
            Some(LaneOwner::Gesture)
        );
        assert_eq!(
            claims.claim(sid(1), LaneOwner::Gesture).err(),
            Some(LaneOwner::Gesture),
            "a gesture does not share with another gesture either"
        );
        drop(held);
        assert_eq!(claims.owner(sid(1)), None);
        claims.claim(sid(1), LaneOwner::Gesture).expect("re-claim");
    }

    #[test]
    fn region_queries_share_a_lane_but_still_fence_a_gesture() {
        let claims = SolverLaneClaims::new();
        let first = claims.claim(sid(2), LaneOwner::RegionQuery).expect("first");
        let second = claims
            .claim(sid(2), LaneOwner::RegionQuery)
            .expect("second");
        assert_eq!(
            claims.claim(sid(2), LaneOwner::Gesture).err(),
            Some(LaneOwner::RegionQuery)
        );
        drop(first);
        assert_eq!(
            claims.claim(sid(2), LaneOwner::Gesture).err(),
            Some(LaneOwner::RegionQuery),
            "one holder left ⇒ still fenced"
        );
        drop(second);
        claims
            .claim(sid(2), LaneOwner::Gesture)
            .expect("last holder gone ⇒ free");
    }

    #[test]
    fn lanes_are_per_sketch_not_global() {
        let claims = SolverLaneClaims::new();
        let _a = claims.claim(sid(3), LaneOwner::Gesture).expect("a");
        claims
            .claim(sid(4), LaneOwner::RegionQuery)
            .expect("a different sketch is untouched");
    }
}
