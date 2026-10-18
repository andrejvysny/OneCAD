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
