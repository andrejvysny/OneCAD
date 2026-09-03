---
name: projection-seams
description: WP-P projected-geometry seams — PROJECTION_STALE carries no evidence, feature rows carry no sketch id, and why every projection verb re-enters the sketch
metadata:
  type: project
---

Facts about the projected-geometry feature (WP-P) that the code cannot tell you.

**Why:** two of these are backend gaps the frontend works around; a future
implementer who assumes otherwise will build on a value that is never populated.

**How to apply:** read before touching the projection banner, the stale badge, or
any projection verb.

- `PROJECTION_STALE` is minted in `src-tauri/src/document_runtime.rs`
  (`step_diagnostics`) with `evidence: None`. The stale entity ids stay
  Rust-side; the only machine-readable thing that crosses is the count at the
  START of the human-readable `message`. `projectionStale.ts` parses it.
- A `FeatureRecord` carries no sketch id, and EVERY sketch timeline record is
  named literally `"Sketch"` (`upsert_sketch_record`), so `label` is not a link
  back either. With two sketches both holding projections and only one stale, the
  banner attributes the verdict to whichever sketch is open. Closing this needs
  `sketchId` (or the entity ids) added to the diagnostic's `evidence`.
- After `projectToSketch` / `updateProjection` / `detachProjection` the session
  MUST be re-hydrated (`sketchService.rehydrateSketchSession` → a fresh
  `client.enterSketch(sketchId)`): a freshly projected entity's backend uuid is
  not in the tauri client's sketch id-map yet, so the returned `entities` array
  can legitimately be EMPTY on the real lane. Re-entry reseeds the map via
  `seedIdMapFromWire`; `enter_sketch` is idempotent for the already-open sketch.
  Never call `engine.enterSketch` to refresh — it re-aims the camera and
  overwrites the saved restore view.
- Count "sources sent minus refusals", never returned entities, when reporting a
  projection batch: it is the one number that means the same thing on both lanes.
- The mock lane only resolves the seed box `body1` (planar faces via
  `lookupMockFace`, edges `e:0..e:11`); everything else refuses
  `unsupportedCurve` with a `MOCK LIMIT` message. Mock bodies never move, so
  nothing can ever go stale in the mock/Playwright lane.
