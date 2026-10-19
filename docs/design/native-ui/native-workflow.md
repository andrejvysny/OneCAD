# Native plate workflow: shared-service regression

This code-coupled guide describes ONECAD-273's synthetic fixture and executable
application regression. It does not establish native GUI acceptance, approved
product dimensions, or ONECAD-274 data continuity. The native UI host builds and
has automated source-level coverage; native input acceptance remains separate.
The launch recipe below documents its isolated interface and does not prove a
completed desktop workflow.

## Fixture and independent expectations

The fixture contains one finished, fully constrained rectangle on the named XY
plane: **80 × 50 mm in sketch coordinates**, eight point endpoints, four lines,
four Coincident constraints, four Horizontal/Vertical constraints, one Fixed
anchor and two dimensions. The real worker must report DOF 0, no conflicts and
one region with usable `previewTriangles`, no holes, and current new-authoring
`regionIdentityVersion: 3` (`cell-v3`; see [SCHEMA §7.4](../../../protocol/SCHEMA.md)).
The saved fixture contains no solid; its visible profile is ready for selection.

These dimensions and the **6 mm initial / 8 mm edited extrusion** are synthetic
test inputs, not an approved mounting-plate requirement. The named XY basis is
preserved verbatim: local X maps to world +Y, local Y maps to world -X, and the
normal is world +Z. Thus the expected world bounds are `[-50, 0, 0]` to
`[0, 80, depth]`, with centroid `[-25, 40, depth/2]`. The volume is
`80*50*depth` mm³ and surface area is `2*(80*50 + (80+50)*depth)` mm². No axis
swap or scene-root rotation is applied. The basis is the existing
[Sketch.h-compatible contract](../../../crates/onecad-core/src/sketch/plane.rs).

## Generate and validate

Run from the repository root with the already-built real worker. This command
writes the explicit fixture under QA temporary storage and runs both workflow
phases. Without `ONECAD_NATIVE_PLATE_FIXTURE`, all outputs stay in the test's
automatically removed temporary directory. `ONECAD_WORKER_PATH` is mandatory;
a missing path/binary fails rather than skipping.

```sh
ONECAD_NATIVE_PLATE_FIXTURE=/private/tmp/onecad-273-qa/constrained-plate-80x50.onecad \
ONECAD_WORKER_PATH="$PWD/worker/build-native-migration/onecad-worker" \
ONECAD_REQUIRE_WORKER=1 \
CARGO_TARGET_DIR=/private/tmp/onecad-native-rust-check \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 \
cargo test -p onecad-app --test native_plate_workflow -- --nocapture
```

The [test](../../../crates/onecad-app/tests/native_plate_workflow.rs) and
[profile helper](../../../crates/onecad-app/tests/native_plate_support/profile.rs)
use `AppState::start(HostContext)` and the production application services:
`new_document`, typed `AddSketch`, `enter_sketch`, `sketch_upsert`,
`finish_sketch` and `save_document`. They do not construct a private `.onecad`
container or bypass application mutation through a separate test runtime.

The parent launches two fresh copies of the test binary. Each child starts a
fresh application host and real worker. Before launch, the parent creates that
phase's private directories and sets only the child's `TMPDIR`,
`ONECAD_LIBRARY_ROOT` and `ONECAD_LOG_DIR`. These point to `temp`, `data/library`
and `logs` under the corresponding `create-host` or `reopen-host` directory.
Library seeding, import workspace sweeping, logs, recents, configuration and
autosave therefore stay inside the QA area. Parent environment is unchanged.

The create phase proves:

- The supported constrained profile is saved while visible, with no bodies.
- Typed `preview_operation` creates an exact 6 mm candidate without changing
  document projection, history, worker epoch, snapshot or history-prefix hash.
- `apply_operation` commits the same body identity at 6 mm. Automatic profile
  hiding belongs to that same undo step.
- A typed `UpdateOperationParams` edits the extrusion to 8 mm in one undo step.
- One undo restores 6 mm and byte-identical fine MESH1; the next removes the
  extrusion and restores profile visibility. Two redos restore 6 then 8 mm.
- Save reports `clean`, close removes the document, and the worker process exits.

The reopen phase starts in another test-host process, opens the saved 8 mm
container, and proves clean state both before and after replay. Before editing,
it checks stable record/body/profile IDs, the original region ID, version 3,
all typed extrusion parameters and derived input refs. It then edits to 6 mm,
checks one new undo item, saves cleanly and closes the worker.

At each committed/reopened height, independent analytic expectations check one
body, one BRep solid, six faces, exact volume/area/centroid, mesh bounds and a
closed connected triangulated shell. Mesh reads use checked `MeshView`; expected
sizes are not inferred from the returned mesh or copied from persisted caches.
Existing-feature numeric edits use `UpdateOperationParams`; the current
`PreviewOp` service previews a new operation and is not used to double-apply an
existing extrusion.

## Isolated native host launch interface

Coordinate the current build/launch status with the host owner. Build before
setting a fresh launch-time `TMPDIR`: Cargo/rustc may need a pre-existing temp
directory, while the host must create its own marked QA root before shared
application startup. For the isolated target used by this regression, the
intended commands are:

```sh
CARGO_TARGET_DIR=/private/tmp/onecad-native-rust-check \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 cargo build -p onecad-ui

TMPDIR=/private/tmp/onecad-273-native-ui/temporary \
ONECAD_LOG_DIR=/private/tmp/onecad-273-native-ui/logs \
ONECAD_LIBRARY_ROOT=/private/tmp/onecad-273-native-ui/data/library \
ONECAD_WORKER_PATH="$PWD/worker/build-native-migration/onecad-worker" \
/private/tmp/onecad-native-rust-check/debug/onecad-ui \
  --qa-root /private/tmp/onecad-273-native-ui \
  --open /private/tmp/onecad-273-qa/constrained-plate-80x50.onecad
```

Use a fresh dedicated QA root. The host's `QaPaths` contract rejects production
roots, symlink escapes, unmarked nonempty shared roots, concurrent ownership and
unscoped log/temp/library paths. It copies the source fixture into
`<qa-root>/documents/`; saved edits belong to that copy. On a later launch, pass
the saved QA copy explicitly via `--open` rather than copying the original over
it. These path rules have source-level tests in
[qa.rs](../../../crates/onecad-ui/src/qa.rs); they do not prove native input.

## Contract parity and limits

This is a scoped migration map, not complete parity or a native gate result.
Test sources listed outside the evidence section are implementation references;
their presence alone is not a fresh test run. TypeScript paths in the table are
relative to `src/`; Rust paths are relative to the repository root.

| Existing TypeScript contract | Rust coverage / implementation | Current qualification |
|---|---|---|
| `tools/preview/regionPick.test.ts`, `viewport/engine/RegionPickLayer.test.ts` | Real app fixture has one selectable triangulated region; `onecad-ui/src/profiles.rs` projects its supplied fill. | Fixture/service support verified; native profile click unverified. |
| `tools/modelTools/modelToolMachine.test.ts`, `ModelToolController.extrudeGesture.test.ts`, `tools/preview/depthProjection.test.ts` | `onecad-interaction/tests/transitions.rs` checks exact operation/profile identity, expression evaluation, invalid drafts and stale results. | Numeric/exact-candidate state implemented; complete pointer/arrow-drag parity and native input unverified. |
| `ModelToolController.previewOwnership.test.ts`, `ModelToolController.commit.test.ts` | Interaction tests check late replies, duplicate confirms, admission/publication ordering and failed/repair terminals; real workflow checks preview head immutability and accepted commit. | Scoped single-profile service behavior verified; multi-region/broad-tool parity not claimed. |
| `ModelToolController.commit.test.ts` re-edit parameter preservation | Interaction test `committed_extrusion_edit_uses_apply_operation_without_auto_hiding_or_rollback`; real workflow verifies persisted typed refs/params before re-edit. | Shared numeric edit path verified; native history-row editing unverified. |
| `features/shell/undoActions.test.ts`, `tools/sketch/sketchService.undo.test.ts` | Interaction `text_focus_owns_undo_even_without_text_history`; app transactional undo/redo and cancel/rollback tests. | Model history verified; native text/IME/clipboard/menu focus routing unverified. |
| `viewport/engine/CameraRig.test.ts`, `cameraFit.test.ts`, `Picker.test.ts` | `onecad-scene/tests/camera.rs`, `picking.rs`, `budget.rs`: Z-up cameras, fit, synchronous snapshot-scoped CPU picking, 8 logical-pixel edge aperture and 2-pixel occlusion slack. | Pure scene coverage exists; native trackpad/mouse/Retina behavior unverified. |
| `viewport/mesh/parseMeshPayload.test.ts` | `onecad-protocol/tests/mesh_checked.rs`: checked immutable MESH1 access, malformed inputs and worker-produced fixtures. | Rust structural checks implemented; wire format unchanged. |
| Shell visual tokens/icons and event dispatch | UI source tests cover QA paths, theme tokens, icon mapping and bounded bridge permits. | Host builds; automated tests do not establish AX, IME, visual goldens or native-workflow acceptance. |

## Sketch interaction and donor adaptations

The [interaction-to-worker test](../../../crates/onecad-interaction/tests/real_sketch_modeling.rs)
drives the native `SketchInteraction` rectangle reducer into the same typed
application services used by the host. Its synthetic 30 × 20 mm rectangle emits
one 12-operation batch with shared endpoint identities and four H/V constraints.
The independent expected DOF is four; adding a fixed origin and two dimensions
reduces it to zero. Finishing yields a version-3 region. Extrusion to 5 mm must
produce one solid with volume 3000 mm³ and world bounds `[-20, 0, 0]` to
`[0, 30, 5]`. A 7 mm edit, undo, redo and sketch re-entry check retained identity
and editability. This automated test supplies captured pointer coordinates; it
does not establish physical native input acceptance.

SolidCraft adaptations use pinned revision
`6bde126a5cabd1f5af0c68e6955bd0176fab805a`: ray/plane intersection for sketch
coordinates, separate renderer highlight batches, analytic line expansion and
exclusive temporary-file creation before cleanup ownership. OneCAD retains its
plane bases, checked scene identities, history, solver and document format.
Exact source paths, local changes, maintenance ownership and license notices are
listed in [ATTRIBUTION.md](../../../ATTRIBUTION.md). The donor's application and
document model are not dependencies of this workflow.

The extracted application already has pure coverage for provisional cancellation
and failed transactions: `begin_geometry_correction_is_preview_and_cancel_discards_it`,
`invalid_begin_payload_cleans_scratch_and_lane_without_authoring`,
`cancel_new_sketch_removes_creation_and_clears_redo`,
`failed_operation_receipt_rolls_back_exact_transaction_once`, interleaving/depth
ABA refusals, and runtime-bound rollback receipts. These are in
[gesture_tests.rs](../../../crates/onecad-app/src/document_runtime/gesture_tests.rs)
and [runtime tests](../../../crates/onecad-app/src/document_runtime/tests.rs).
The real plate test does not duplicate these fake-backend failure cases or claim
that they prove a real native Cancel gesture.

## Recorded evidence and provenance

Checkpoint: **2026-10-18**, base Git HEAD
`d76857c20ad777398689b517cccb3c71cc0d2356`, with concurrent uncommitted migration
changes. There is no commit attribution for the new app/protocol/test files;
these results describe that working tree, not a clean immutable revision.

The real worker was `worker/build-native-migration/onecad-worker`, Release,
OCCT **8.0.1**, local CMake build ID `local`, protocol 1, worker 0.1.0,
quantization 1 and solver policy 2. Its SHA-256 was
`76772827bfba8698548cafbb5981b7d7918a665e69da6e63877a27ac587ee215`, identical to
`src-tauri/binaries/onecad-worker-aarch64-apple-darwin`. The staged manifest
records OCCT fingerprint `0a6a1dce34181289`. Worker sources also had dirty changes;
the binary hash and manifest identify the tested artifact, not a clean worker
source revision or a release provenance qualification.

Observed results:

- Isolated create/reopen driver: **1 passed, 0 failed, 0 ignored**, with each
  child reporting 1 pass. Final run took 0.32 s after compilation.
- `cargo test -p onecad-app --lib document_runtime::`: **171 passed, 0 failed,
  0 ignored**; this includes the cancel/rollback cases above.
- `cargo clippy -p onecad-app --test native_plate_workflow -- -D warnings` and
  `rustfmt --check` passed for the final test changes.

Full local logs are temporary evidence, not durable project knowledge:

| Temporary path | Meaning |
|---|---|
| `/private/tmp/onecad-273-workflow-isolated.log` | Passing final isolated create/reopen test. |
| `/private/tmp/onecad-273-workflow-clippy.log` | Targeted Clippy pass. |
| `/private/tmp/onecad-273-cancel-rollback-tests.log` | 171 passing pure runtime tests. |
| `/private/tmp/onecad-273-workflow-first-failure.log` | Test expected conventional XY; corrected to existing documented +Y/-X basis. |
| `/private/tmp/onecad-273-workflow-bodyid-failure.log` | Test passed projection UUID to query API expecting `body_<uuid>`; corrected with existing typed `body_id_wire` lowering. |
| `/private/tmp/onecad-273-workflow-region-version-failure.log` | Test assumed stale DTO-comment version 2; corrected to current normative authoring version 3. |
| `/private/tmp/onecad-273-workflow.log` | Earlier passing service run before child environment-isolation hardening. |

The generated `.onecad` fixture is likewise temporary. Regenerate it from the
command above when reviewing another worker/source revision. Native profile
selection, numeric focus, Enter/Escape, drag/manipulator behavior, menu routing,
undo/redo, real process quit/reopen, visual quality and unsupported-tool/error
messaging require separate observation in the built native host.

## Stabilized shell contracts

The native shell uses separate tool cancellation and selection clearing. Cancel
ends an editable transient draft without changing the authoritative document;
idle Escape clears selection. The selection label's close button cancels and
clears. Body and viewport selection clear conflicting profile/feature selection.
History-edit cancellation retains the previously selected body. Until persistent
face attachment is wired, the creation action explicitly says **Sketch (XY)**,
including in face context; it does not claim to attach to that face.

All egui text editors own text undo/redo and native editing menu actions. IME
Enter/Escape protection runs before CAD routing, including composition commit
and end frames. Panel shortcuts use the same text-focus policy. Outside text focus,
active sketches use a typed session-scoped history operation; other contexts use
model history. Title, File and native Edit menus share this routing. Sketch history
uses bounded application-owned undo mementos, a unique edit-visit token and the
visit's worker epoch. It refuses crossing the entry boundary, foreign transactions,
stale completions and redo not owned by the visit. Undo/Redo cancels a transient
draft before moving committed history. A failed candidate solve leaves document
and both stacks unchanged; empty scoped history is explicit. No UI undo snapshots
or direct UI document mutations are introduced.
Command lifecycle/error status takes precedence over ready-preview status.
Number stepping preserves authored fractional precision; expressions are not
silently replaced by stepped numeric values.

Camera, topology hover and authoring overlays share higher-layer pointer
ownership. New viewport gestures cannot begin through chrome. Captured gizmo
moves/releases can cross chrome; focus loss, Escape and popup opening end capture.
History uses the authoritative applied cursor and actual suppressed/error states.
Its rollback marker is display-only until typed structural mutation is exposed.

Target replacement first tries the transactional old/new allocation. After both
CAD and final host sampling fences drain, a budget/allocation refusal may retry
by retiring intermediate attachments while retaining and charging the previous
output texture. A failed retry keeps that image but invalidates its pick capture;
the host offers Retry and retries changed dimensions. Save remains available.
Device loss remains a separate unavailable state. Resource caps, including
512 MiB GPU replacement residency and 500,000 admitted triangles, are unchanged.
The required Metal regression covers the default 2x target, repeated resizes,
refusal followed by a valid size, suspension/restoration and quality transitions.
Hardware tests do not establish physical monitor or input acceptance.

Startup discovers recovery before opening a saved UUID. Restore, Open saved
(discard recovery), and Discard are explicit decisions. Failed saved-open must
retain the pending offer, autosave and marker; destruction happens only after a
validated replacement runtime is installed. One document opens per startup;
other offers remain protected for a later launch. The isolated host does not
implement production settings migration or cross-process project ownership.

Lifecycle metadata uses `api::get_document_state` and a bounded latest-state slot
independent of the renderable display. Capture refusal cannot clear dirty state
or remove Save access. Ordinary dirty close retains recovery; explicit close
discard checks the runtime session under the same lock that removes it. Recovery
containers never expire by age. Released owners retain their labels with PID zero.
Allocation recovery prioritizes the root out-of-memory error over dependent
validation errors; validation-only failures remain device errors. Renderer
regressions inject both errors and verify retained pixels and retry on real Metal;
they do not reproduce hardware OOM or establish egui sampling-fence acceptance.
Wireframe uses edge-only picking for clicks and hover; invisible face interiors
cannot select bodies or occlude profiles. Shaded mode keeps face occlusion.

Focused regression sources are `onecad-interaction/tests/transitions.rs`,
`onecad-ui/src/input/tests.rs`, `pointer_policy.rs`, `gizmo_tests.rs`,
`inspector_history.rs`, `feature_card.rs`, `startup_recovery_tests.rs`,
`onecad-widgets` input/card/icon tests and
`onecad-render/src/tests/resize_recovery.rs` (all paths under `crates/` unless
otherwise stated). The recovery tests simulate a dead owner and assert exact
restored semantic values; they do not replace a physical crash/restart drill.

## Frozen-reference reachability map

The reference revision is `d76857c20ad777398689b517cccb3c71cc0d2356`.
The following paths refer to source/tests at that revision. They establish
specific migration contracts to inspect; test-file presence is not evidence of
a fresh Tauri run. This map does not promote inert design-registry entries,
placeholders or new operations into parity requirements. Rows marked unavailable
must pass through the native application before migration can be qualified.

| Reference behavior and source/test anchor | Native entry and application authority | Regression / acceptance boundary |
| --- | --- | --- |
| File menu and save/open/recents: `src/features/shell/fileActions.ts`, `fileActions.test.ts` | Native File menu/start screen, typed `document_flow::Request` and bounded `document_services`; `api::{new_document,open_document,save_document}` | `document_services_tests`, dirty-transition tests and persistence suites; real metadata recents replace examples. Physical dialogs, dirty transitions and process reopen remain unqualified. |
| Dirty close and recovery: `src/features/shell/UnsavedChangesDialog.tsx`; prior backend recovery contracts | Native close prompt/startup recovery modal; `api::{check_recovery,recover_document,close_document}` | `startup_recovery_tests`, `autosave_recovery`, `recovery_hardening`; physical crash/restart and clean teardown unqualified. |
| Scoped history: `src/features/shell/undoActions.ts`, `undoActions.test.ts`, `src/tools/sketch/sketchService.ts` | Shared native text/title/File/Edit-menu dispatcher; model `api::{undo_with_outcome,redo_with_outcome}` and fenced `api::sketch_history_with_completion` | Text editor/retained-input tests, sketch transition tests and real-worker `sketch_history` cover precision, entry boundary, empty history, redo ownership and edit visits. Physical sketch Undo/Redo, focus/IME and complete native workflow acceptance remain required. |
| Sketch primitives, face entry, constraints and construction: `src/tools/sketch/SketchController.face.test.ts`, `SketchController.draw.test.ts`, `SketchController.construction.test.ts`, `src/features/sketch/ConstraintMenu.test.tsx` | Native XY Select/Line/Rectangle/Circle reducer; singleton Point Fixed, Line Horizontal/Vertical/length and Circle diameter through strict application constraint admission | `real_sketch_modeling`, `sketch_constraint_admission`, `sketch_center_refs` and UI/reducer tests. Face attachment, other constraints, existing-value editing, snapping and broader editing remain unavailable; physical native constraint/dimension acceptance is outstanding. |
| Extrude preview, commit and re-edit: `src/tools/modelTools/ModelToolController.commit.test.ts`, `modelToolMachine.test.ts` | Profile pill and feature card; typed interaction effects; `api::apply_operation` | `transitions`, `native_plate_workflow`, provisional rollback tests; physical 6/8/6/8 mm input/save/reopen still required. |
| Additional solid operations and history: `src/tools/modelTools/ModelToolController.filletFromFace.test.ts`, `ModelToolController.patternReedit.test.ts`, `src/features/inspector/historyActions.test.ts` | Existing application operation authority; most native creation/structural controls disabled | `fillet_blend_class`, `feature_pattern_integration`, `history_outcomes`; no native reachability or full-family acceptance claimed. |
| Variables/expressions: `src/features/inspector/variableAuthoring.test.ts`, `src/ipc/mockClient.variables.test.ts` | Feature numeric expression evaluation; `api::{evaluate_expression,upsert_variable_expr,rename_variable}` | `variable_driven_ops`; native variables UI unavailable. Mock tests alone do not prove a working product flow. |
| Measurement/section/display: `src/features/measure/MeasurePanel.test.tsx`, `src/features/shell/DisplayModePopover.test.tsx` | Native Display popover exposes canonical Shaded, Shaded + edges, Wireframe and capability-gated X-ray through fenced renderer options; section remains view-only; measurement unavailable | `viewport_options::tests` checks mode choice and unsupported X-ray; renderer CPU/required Metal suites cover their declared contracts. Physical appearance, display preferences and measurement workflow acceptance remain separate. X-ray is an existing native renderer capability, not a claim of frozen Tauri parity. |
| Import/export: `src/features/shell/fileActions.ts` | Native File menu/start-screen import through typed document services; `api::{import_step,export_step_file,export_stl_file,export_obj_file,export_3mf_file}` | `document_services_tests`, `step_import`, `step_export_attributes`, `threemf_export`; opening a project copy is supported, merge-import remains unavailable. Backend pass does not qualify native dialogs or appearance continuity. |
| Library browsing/placement and gears: `src/features/library/LibraryModal.test.tsx`, `ComponentDetailRail.test.tsx`, `src/tools/modelTools/gearMachine.test.ts` | Existing library and gear application services; native UI unavailable | `library_ingest`, `gear_ops`; native browsing/configuration/placement/generation acceptance required. |

The native input acceptance sequence remains: foreground the isolated app; select
its prepared profile through the viewport; preview/commit 6 mm; history-edit to
8 mm; undo to 6 mm; redo to 8 mm; save; quit cleanly; reopen the saved QA copy.
Record exact feature/profile/body IDs and typed parameters before/after each
step, persistent refs, bounds/BRep semantics, history depths and dirty state.
Repeat cancellation and failed provisional operations to verify unchanged prior
state and intended undo grouping. Run the focus/IME/menu, chrome-crossing gesture,
minimum-window/Retina resize and autosave/crash/restart/second-crash drills with
actual macOS input. A blocked input tool is not a passed or failed modeling gate.

### Qualifying documents captured from native input

Save a QA checkpoint after each committed 6 mm, edited 8 mm, Undo 6 mm and
Redo 8 mm state. Copy those saved containers into a separate capture directory as
`committed-6.onecad`, `edited-8.onecad`, `undo-6.onecad` and `redo-8.onecad`.
After a clean quit and real native reopen, save and copy `reopened-8.onecad`.
Use one document throughout; do not regenerate missing checkpoints with a
headless fixture or substitute captures from an earlier failing run. Record the
host binary hash, source/diff identity, worker manifest and native logs beside
the captures. Confirm Save reports a clean document before copying each file.

Run the explicit semantic gate against those immutable captures:

```sh
ONECAD_NATIVE_CAPTURE_ROOT="/absolute/qa-captures" \
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build-native-stabilization/onecad-worker" \
cargo test -p onecad-app --test native_capture_acceptance --offline -- --ignored --nocapture
```

Missing captures or worker prerequisites fail this gate. Each capture opens and
regenerates through shared services in an isolated child process. The test checks
document/feature/body identity, exact sketch and persistent profile references,
6/8/6/8/8 mm parameters, clean saved state, BRep solid/face properties and checked
MESH1 geometry. Source hashes must remain unchanged. Saved containers do not
persist the live Undo stack: verify one history step per accepted action and the
Undo/Redo depths separately from native adoption logs. This semantic test does
not qualify text focus, gestures, IME, rendering or the input sequence itself.

For clean shutdown, require the host's `Native host exited cleanly` log after
`eframe::run_native` returns successfully and inspect stderr for AppKit exceptions.
`open -W` exiting with status 0 alone is insufficient: Launch Services may return
success even when teardown raises an Objective-C exception.

### Scoped sketch history regression gates

Run with the freshly built production worker, preserving explicit prerequisites:

```sh
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build-native-stabilization/onecad-worker" \
cargo test -p onecad-app --test sketch_history --no-fail-fast
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build-native-stabilization/onecad-worker" \
cargo test -p onecad-app --test sketch_history_solid --no-fail-fast
cargo test -p onecad-app --lib document_runtime::sketch_adoption::tests:: --no-fail-fast
cargo test -p onecad-interaction --test sketch_transitions --no-fail-fast
cargo test -p onecad-ui --lib input::tests --no-fail-fast
```

The real-worker gate verifies exact sketch/entity identity and authored precision
across Undo/Redo, the entry boundary, explicit empty history, new-edit redo clearing
and rejection of a prior visit. Unit gates additionally cover solver failure,
conflicting/overconstrained replies, runtime/revision/worker fences and cancellation
without a revision bump. Strict constraint candidates share the edit-visit fence,
so Cancel or a new visit cannot accept a detached old candidate.

`sketch_history_solid` starts with an already-full 200-entry model history and a
fully constrained sketch driving an extrusion. It checks dimension expressions,
solver-moved coordinates, local Undo/Redo, partial Undo followed by Finish,
Discard, retained identities and fresh-process save/reopen equivalence. Active
visits qualify solved sketch state and replay completion; solid volume and mesh
are checked after Finish/Discard refresh the timeline sketch record. Prefix
eviction preserves the visit boundary. A visit that evicts its own edits keeps
bounded local history, but Discard conservatively reports `history was trimmed`
and preserves changes; Finish retains granular steps rather than squashing an
incomplete visit.

Native acceptance remains separate: create two sketch edits, verify title/File/Edit
menu and keyboard Undo/Redo each move exactly one step, cancel a transient draft
with Undo before changing committed history, verify text Undo remains text-only,
and confirm Finish/Discard retains intended model undo grouping. Repeat after a
worker restart and ensure the old visit cannot mutate history. These automated
gates do not establish physical macOS input, IME or displayed solver truth.
