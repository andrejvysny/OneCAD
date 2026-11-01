---
name: placement-gesture-gotchas
description: Native component placement (crates/onecad-ui/src/placement_gesture) wiring facts and the plate delivery-test harness traps
metadata:
  type: project
---

Placement is a plain PlaceComponent operation in the shared reducer; `placement_gesture` only owns hover/snap/repeat state in `DocumentEditor.placement`. Library-lane replies for it (`Lane::Prepare`/`Lane::Classify`) land in the catalog's `placement` inbox as the clonable `Placed` enum (catalog is Clone, `Reply` is not), read by sequence in `poll`. Classification keeps one request in flight (bridge permits), asking for the newest hovered target when it returns.

Test traps in `plate_delivery_tests/placement.rs`: the sketch plane maps local (x,y) to world (-y, x), so derive world targets from `effective_plane().to_world`, never hardcode; a placed screw occupies the hole and changes later hover picks (use a face point for later legs); egui 0.37 `RawInput` has no `modifiers`, send `Event::ModifiersChanged` first; `ONECAD_LIBRARY_ROOT` must be unset (`env -u`).

`cargo fmt --all` / builds fail transiently when a concurrent session adds modules to onecad-app (e.g. `library_instances`); wait for the file instead of touching it.
