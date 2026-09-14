# Claude Code — viewport-hardening consolidation brief

**Applies to:** review of `61dab320878afe7c5aa42d0ab90d5404b39ef2f1` on `viewport-hardening`.  
**Design authority:** existing `docs/viewport-hardening/` VP-HARDENING 1.0 package.  
**Review authority for this pass:** `01-PROGRESS-REVIEW.md`, findings PR-01–PR-12.

## Mission

Complete a bounded consolidation pass over the implemented foundations before expanding to additional work packages. Preserve the existing architecture and all unrelated user changes. This brief does not authorize commits, pushes, resets, destructive cleanup, silently changing acceptance thresholds, or replacing the existing design.

Review the current checkout first. The branch may have progressed since the pinned review; do not blindly replay fixes that already landed. For each finding, record `still-present`, `fixed-with-evidence`, or `needs-reproduction`. A name change or new helper is not evidence of a fixed contract.

## Evidence rules

Read `AGENTS.md`, `CLAUDE.md`, current ledger heads and the package documents before editing. Treat the reviewed source as the reproduction target, not necessarily the latest local source.

The attached Node harness contains extracted/transcribed decision probes, not application tests. Port each relevant failure into the repository at its real seam. Preserve the red result before correcting production code. A passing assertion that intentionally demonstrates a bug is not product acceptance.

The prior run logs were not available in the reviewed commit. Recover and sanitize genuine local artifacts or rerun the relevant tests; do not fabricate logs or translate a narrative into a claimed executed result. Record source/worktree hash, command, environment, timestamp, exit status, failure signatures and checksums.

Keep implementation, focused-unit, graphics, native, physical-device and benchmark states separate. Where a required environment is unavailable, mark that lane blocked and state the missing requirement. Never substitute jsdom or mock-client browser behavior for native OCCT/Tauri acceptance.

## Execution order

### Step 0 — restore portable evidence and an honest baseline

Address PR-10 before reporting any new acceptance pass. The acceptance matrix must be verifiable from a clean checkout, without ignored local `*.log` files. Choose narrowly allowed sanitized repository artifacts or a durable artifact manifest plus a verifier that can validate it honestly.

Do not delete the evidence-existence check. Do not remove expected-failure tests merely because their failures are inconvenient. Current unresolved R07/R09/R10/R18 ratchets belong to their planned work packages.

Correct PR-12's edge test section identifier and introduce a hand-authored fixture that distinguishes EDGE_RANGES (7) from EDGE_POSITIONS (8). Then capture current focused/frontend/worker baseline failures with names and signatures.

### Step 1 — typed acquisition and atomic display publication

**PR-01:** make viewport-derived picks carry the installed entry/publication proof captured at hit time. Thread it through ordinary selection, overlap selection, sketch-on-face, measurement and tool acquisition. Check stale/retired/mismatched identity before the `el_` fast path. Do not obtain an apparently current proof by pairing an old hit with a new registry lookup. Non-viewport authoritative references need a distinct typed API rather than an optional proof bypass.

Required regression: failed mesh replacement, old geometry still drawn, normal click, attempted promotion and operation initiation. Repeat for a persistent label and a delayed promotion result. Prove that the ordinary path—not only the overlap chooser—is protected.

**PR-02:** remove observing callbacks from the core publication transaction. Prepare fallible resources first. Commit registry, effective scene, resource/reservation ownership and display identity coherently. Run retirement/highlight/subscriber notifications only afterward, with per-observer exception isolation. Failed preparation releases its handle lease and resources. Post-commit observer failure must not demote or conceptually undo the installed geometry.

Required regression: inject a throwing retirement listener and a throwing highlight rebuild. Assert registry entry, actual scene geometry, pick mapping, leases and reserved bytes agree. Cover a failure after body-handle creation but before commit.

### Step 2 — charge actual resources for their entire lifetime

**PR-03:** derive one immutable preparation plan from validated geometry and captured appearance metadata. The plan decides indexed/de-indexed layout, source-buffer retention, expanded edges, capacity and byte cost before allocation. Construction must consume that same plan. A metadata-only body color or authored-face map must not bypass de-indexed cost.

Hold the old entry's reservation until the resource is actually disposed. Removal from the installed body map does not free leased retired geometry. Apply the same admission authority to live preview/ghost preparation; do not leave them as unpriced exceptions.

**PR-04:** reserve the actual planned 1.5x capacities of reused face buffers. Preserve old/new peak accounting during growth. Do not allocate first and reject after the fact. Validate exact counters under bounded degraded selection display.

Required regressions: four-to-five-triangle capacity test under a 250-byte test budget; near-64-MiB equivalent; metadata-colored indexed mesh; repeated replacements before the next flush; held old lease; cancelled preview; close/reopen with reused IDs.

### Step 3 — finish every geometry borrower

**PR-06:** explicitly call preview handle disposal on replacement, targeted clear, full clear, engine teardown and failed preparation. Remove the detached sweep after all owners are fixed; a sweep waiting for unrelated retirement events is not the lifetime authority.

Whole-body ghosts must hold leases on the exact geometry they draw. Ranged ghosts must use explicitly owned compact resources, not undisposable shared-attribute wrappers. Section pairs must lease the exact entry behind their source geometry, not a potentially different entry obtained only by body ID.

Required regressions: OffsetFace ranged ghosts, pattern/mirror ghosts, source replacement while ghosts are visible, 100 preview apply/cancel cycles, 50 document cycles, and explicit registry/scene identity mismatch. Verify application leases and renderer resource counts.

### Step 4 — scheduler and submission correctness

**PR-05:** park the scheduler after the intended bounded run of failures even when a failing callback invalidates before throwing. Defer internal scheduling until the tick outcome or cancel already-scheduled internal work at the failure boundary. Preserve dirty state for explicit external retry. Do not reintroduce a permanent idle loop or erase legitimate reentrant redraws.

Required regressions: ordinary throwing work stops; invalidate-then-throw stops; healthy one-time reinvalidation renders exactly the additional needed frame; suspend/resume and disposal remain correct. Exercise real engine listeners, not only the pure scheduler.

**PR-11:** consume the supplied FrameSubmission in mesh acknowledgement. Match expected body and publication/resource identity to actual submitted contents. Hide/isolate before the frame must not acknowledge a body on an unrelated frame. Handle no-longer-required expectations explicitly rather than falsely succeeding or timing out indefinitely.

### Step 5 — numerical closure before WP10 publication

**PR-07:** body caps count all emitted segments, including fallback endpoints. At zero remaining capacity, do not emit more geometry. Preserve topology table identity with explicit missing/degraded status. Relaxed retries retain requested and achieved tolerances separately and never inherit an unqualified certificate for the original request.

**PR-08:** distinguish true degeneracy from missing required geometry or a deliberate display-detail omission. A global body-relative positive-area threshold is not proof of zero-area topology. Carry completeness to the outer result/install policy; until the versioned wire contract is ready, do not expose partial success as an ordinary complete current body. Retain the last valid display or use explicit non-actionable degradation.

**PR-09:** close the outstanding numerical review. Angular evidence must refer to encoded chords and account for endpoint snapping/quantization. Use the straight-edge example from the review as an actual OCCT/encoder fixture. Keep positional and angular certificates separate: a valid chord-distance bound does not imply a valid angular bound. Remove the near-period semantic-closure shortcut. Treat nonfinite/ill-conditioned or unestablished arithmetic as explicit uncertainty, not a named constant that pretends to prove correctness.

Preserve surface-derived normals, face identity, orientation and singular fallback provenance. Do not indiscriminately weld geometry or change modeling/solver tolerances to make display tests pass.

## Ownership of work

Use at most three coordinated streams and one owner for each shared source file:

| Stream | Ownership | Coordination requirement |
|---|---|---|
| Frontend identity/resource transaction | promotion, registry, ingestion, preparation plans, preview/ghost leases, highlight capacities | Own shared registry/ingest signatures; other streams request changes rather than editing them concurrently. |
| Frame and evidence | scheduler, frame acknowledgement, artifact policy, focused graphics tests | Agree on submitted resource identity with the first stream. |
| Worker correctness | sampler, normals/completeness, edge-quality oracle and native fixtures | One owner for Tessellate.cpp; protocol edits wait for joint cross-layer review. |

Serialize heavy CMake/Cargo/browser/full-unit runs. The orchestrator reviews every diff and reruns final gates rather than treating subagent reports as acceptance. Do not create additional architecture layers solely to work around ownership conflicts.

## Required completion record

For each PR finding record:

```markdown
### PR-xx
Status: reproduced / implemented / focused-pass / integrated-pass / native-accepted / blocked
Current source/worktree identity:
Source mechanism:
Red test and artifact:
Production change:
Focused test command and artifact:
Graphics/native/device evidence:
Resource/identity invariants checked:
Remaining limitations:
```

A consolidation completion claim requires the portable evidence gate, PR-01–PR-09 fixes or explicit reviewed scope limitations, PR-11/12 validation corrections, no new relevant-suite failures, and the required native evidence for whichever acceptance level is claimed. A missing native environment is not a native pass.

After this checkpoint, resume the existing WP05/WP06/WP10-first dependency sequence and later packages. Do not redirect effort to WebGPU, photorealistic effects or a new scene framework.
