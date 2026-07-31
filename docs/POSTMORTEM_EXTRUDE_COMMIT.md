# Postmortem: Extrude ✓ silently did nothing (EXTRUDE-COMMIT-FIX, 2026-07-30 → 2026-07-31)

## Symptom

Sketch → Extrude → arm + drag preview all worked; clicking ✓ (or Enter / click-away)
appeared to do **nothing**. No body was created. Switching tools tore down the
preview, leaving only the sketch. At one stage the History panel showed a red
errored `Extrude` row; later attempts showed *no* row at all.

## Root cause

**The regen planner resolves a modeling op's profile ONLY from a `Sketch` timeline
record — and no interactive frontend path ever authored one.**

Two halves:

1. **Backend contract half.** Only `finish_sketch` mints/refreshes the sketch's
   `Sketch` timeline record (`DocumentRuntime::upsert_sketch_record`). A document
   whose sketch has entities but no record fails every extrude commit inside the
   regen executor with **"profile sketch not found in plan"**.

2. **Frontend wiring half (the part that kept it broken after the first fix).**
   No production path called `client.finishSketch` for the extrude flow:
   - `SketchController.exit()` ran **`cancelSketch`** on *every* mode exit —
     squash only, no record.
   - Extrude arm used the deliberately pure-read `getSketchRegions`
     (MODEL-HARDEN W0.5) — no record.
   - The AUTO-MODE handoffs (`finishSketchToTool`, Enter's `finishSketchAction`)
     only flushed mutations and flipped mode.
   - Only Revolve finished at arm time — which is why revolve behaved differently.

### Why it looked like "nothing happens"

- **Preview and commit resolved the profile from different sources.** PreviewOp
  reads the SolverLane cache (worker-side sketch sync), so the drag preview was
  perfect while the commit-time regen plan had no Sketch step at all.
- **The failure was visually silent.** The commit applied, its regen step failed,
  and `rollbackFailedCommit` (added to stop errored-row stacking) undid the record
  — so the timeline snapped back to its pre-click state. Sole feedback was a small
  StatusBar line.
- **Every suite was green.** The mock lane (vitest + Playwright) commits without
  timeline records; the worker-backed Rust tests called `finish_sketch_with_outcome`
  **directly**, simulating a finish the app never issued. The gap lived exactly in
  the seam no suite crossed: real frontend → real backend interactive flow.

## How it was diagnosed

1. **Autosave forensics** (no terminal logs needed): autosave containers live at
   `~/Library/Application Support/com.andrejvysny.onecad/autosave/*.onecad`
   (zip: `document.json` + `timeline/ops.jsonl`). The broken docs showed sketches
   with 12 entities but **0 Sketch records** (one doc: 1 errored Extrude / 0 Sketch;
   post-rollback doc: 0 records total).
2. **Headless replay**: `cargo run -p onecad-regen -- <doc.onecad> --worker
   ../worker/build/onecad-worker --json` replayed the *user's exact broken document*
   through the fixed runtime → `[Sketch ok, Extrude ok] → published, 1 body`,
   proving the kernel and identity spine were fine and only the record was missing.
3. **Code trace** of the full ✓ path: chip → `confirmExtrude` → `endPreview(commit)`
   → `applyOperation` → `apply_edit_command` → 3-phase regen → `regen-finished`
   correlation → rollback.

## The fix

Rust half (part 1, was already in tree):
- `finish_sketch_with_outcome`: mints/refreshes the `Sketch` record (append on
  first finish, `UpdateOperationParams` on content change, no-op when unchanged);
  api command forwards the outcome to the regen scheduler + emits projection.
- `backfill_missing_sketch_records` in `from_document`: legacy containers get
  their missing records inserted at the timeline front on open/recover.
- `rollbackFailedCommit`: an applied-but-regen-failed commit undoes its own record
  so retries never stack errored rows.

Frontend wiring half (part 2, the actual unblock):
- `tauriClient.finishSketch`: id-map miss no longer throws — falls back to the
  backend UUID (`?? sketchId`), so never-entered/reopened sketches can finish.
- `SketchController.exit()`: every keep-exit now runs `cancelSketch` (worker
  gesture teardown + take-once squash) **then `finishSketch`** (record mint).
- `confirmExtrude`: commit-time guarantee — gen-gated `await finishSketch(sketchId)`
  before the commit loop. Idempotent; covers any path that skipped exit. Failure
  re-arms with a named hint without touching the preview sessions.
- Mock lane `finishSketch`: a sessionless finish answers from the cached regions
  instead of clobbering the cache with `[]`.

## Tracing added (keep — it makes the next silent failure loud)

- **Rust (`tracing` → dev-terminal stderr):** `apply_edit_command` digest +
  post-apply revision; `begin_regen` step list (a missing `Sketch:` step is now
  visible at a glance); `regen:` outcome per completion + `warn` per failed step
  **with reason**; `finish_sketch`/`cancel_sketch`; `sketch record: MINT/REFRESH`;
  backfill count; `preview_op` failures.
- **Frontend (`src/debug/trace.ts` → devtools console):** `[extrude]` confirm
  lifecycle (guards, exact-preview barrier, apply result, rollback, re-arm);
  `[ipc]` applyEdit/regen-finished/document-changed correlation; `[lane]` commit
  op + result; `[sketch]` exit path.

## Lessons / guardrails

1. **A op that depends on a timeline record must guarantee it at the commit
   boundary**, not hope some earlier UI flow authored it. (`confirmExtrude` now
   does; give any future sketch-consuming op the same guarantee.)
2. **Preview success proves nothing about commit** here — they resolve profiles
   from different sources. Treat "preview fine, commit fails" as a plan/record
   problem first.
3. **Silent rollback needs loud logging.** Auto-undo of failed commits is good UX
   but erases the evidence; the `regen: FAILED step … reason=…` warn line is now
   the durable trace.
4. **Suites can all be green while the product is broken** when mock lane and
   backend tests each assume the other side does the missing step. The
   `interactive_sketch_flow_…` test only became honest once the frontend actually
   made the same calls.
5. **Autosave containers + `onecad-regen` replay are the fastest forensics** for
   "user says nothing happens": inspect `timeline/ops.jsonl` record census, then
   replay headlessly against the real worker.
