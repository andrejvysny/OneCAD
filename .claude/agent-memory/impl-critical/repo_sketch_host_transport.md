---
name: repo-sketch-host-transport
description: Sketch host-face tracking (WP-1/WP-1b) — where the identity anchor comes from, that EVERY unresolvable host halts, the authored/effective plane split, and the frameTransportVersion/placement wire rules
metadata:
  type: project
---

What the "sketch follows its host face" feature actually rests on.

**Why:** the accepted derivation assumed the frozen anchor is a real pick point ON the
face. It became one only in WP-1b; WP-1 shipped an info-only escape around the gap
and that escape is now gone, so the anchor is load-bearing for the halt policy.

**How to apply:**

- The identity anchor comes from the WORKER: SCHEMA §7.6 `ProjectFaceBoundary` result
  `exact.anchor` (optional) is a point classified IN/ON the face —
  descriptor centre, else `nearest_point_on_face(pln.Location())`, else the surface
  centre of mass. `add_sketch_on_face` (`api/mod.rs`) freezes it as
  `anchor.world_point`, falling back to `frame.origin` only when the key is absent.
  NEVER freeze `exact.origin`: it is the `gp_Pln` LOCATION, a property of the
  surface not the boundary. Measured `step_import_gate.rs` PHASE 4: location
  (0,0,10) for a cap centred at (−5,215,10) ⇒ ladder `anchor` feature 0 ⇒ the
  CORRECT unambiguous host scored 0.750000 / margin 0.200000 (under the 0.85 gate);
  with the on-face anchor the same case is 1.000000 / margin 0.275378. Even on a
  simple extruded box the two differ — the cap's plane location is one of its
  CORNERS, and on that box the corner is equidistant (0) from the cap AND the side
  face sharing that edge, so an anchor-only ref there resolves `ambiguous`.
- EVERY unresolvable host halts (no `SKETCH_HOST_UNTRACKED` — removed in WP-1b).
  The §9 item is the LADDER's own (`LadderResolution::to_needs_repair_json()` with
  our `uiLabel`, reason overridden to `ambiguous` only for the stricter host gate),
  so candidates + `scoringVersion` ride along. The two surviving diagnostics are
  `SKETCH_HOST_RESEATED` / `SKETCH_HOST_NORMAL_REVERSED`.
- Seat witness: the transported frozen anchor, NO exemptions. An absent anchor is
  `sketchSeatUnmeasurable`. A hand-authored test fixture with an origin-as-anchor is
  now a halt — `transform_body.rs::hosted_rect_sketch` had to take a real on-face
  anchor for that reason.
- `TransportStatus::FrameInvalid` is reserved for inputs validated BEFORE any
  arithmetic; a candidate that fails validation AFTER transport is
  `FrameIllConditioned`. Projecting X against n1 cancels catastrophically near τGS
  (b = 2e−8 ⇒ ortho residual ~2e−9 > the 1e−9 bound), so the basis is rebuilt with
  cross products (`yc = n̂(n1 × qx)`, `xc = n̂(yc × n1)`); `xc·x0 == h`, which is
  TINY near the guard — do not assert `xc·x0 ≈ 1` there.
- `frameTransportVersion` is a DEFAULT, not a stamp: `wire.rs` must
  `map.entry(...).or_insert_with(...)`, because a newer build's version rides in
  `SketchOpParams::extra`. The worker compares at the value's own width
  (`is_number_unsigned` → `uint64_t`), never `get<int>()`.
- `parse_sketch_placement` is FALLIBLE: malformed ⇒ the whole `planStep` is
  rejected, never degraded to "absent". Absent means "authored plane is current",
  but the worker already materialised the transported frame into the step it ran.
- `KnownOperation::element_refs_mut` HAS a `Sketch` arm (`host_face`, index 0) and
  is load-bearing: without the `intent.descriptor` it stamps at mint, a face-hosted
  sketch cannot resolve on a from-0 replay — and every Rust regen is from-0.
  `UpdateOperationParams` RE-STAMPS that descriptor, so you cannot construct an
  "unstamped ref" by stripping `intent` through the edit layer; move the ANCHOR
  instead to build a below-gate case.
- The authored/effective plane split: `Sketch::plane` is AUTHORED (what
  `plane_ref_of` writes into the record and what `sketch_geometry_token` hashes);
  `Sketch::resolved_plane` is DERIVED (adopted by `sync_sketch_placements` inside
  `finish_regen`'s fencing guard, no undo entry); `effective_plane()` is what
  `worker::wire::sketch_wire` publishes. `SketchDto` carries NO plane.
- A `Sketch` step CAN fail: an unimplemented `frameTransportVersion` ⇒
  `OpOutcome::unsupported`. It is otherwise body-less, so a repair item on it makes
  the step `NeedsRepair` (`merge_outcome`'s body-less rule).
