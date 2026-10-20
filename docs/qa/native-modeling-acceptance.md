# Isolated native modeling acceptance on macOS

Use this procedure for the local ONECAD modeling milestone. Results and remaining
acceptance obligations belong in the existing Plane tasks. This procedure is not
a claim that native acceptance passed.

## Build and isolation

Record `git rev-parse HEAD`, `git status --short`, macOS/toolchain versions and
the staged worker manifest. Coordinate builds with the implementation owner.

```sh
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" \
ONECAD_WORKER_BUILD_DIR="$PWD/worker/build-sketch-native" \
  scripts/build-worker.sh Release
cargo build -p onecad-ui
```

The resulting native Rust host is `target/debug/onecad-ui`. The retired Tauri
build and browser renderer are not part of this qualification. Launch with a
new isolated profile and an explicit matching worker:

```sh
TMPDIR=/private/tmp/onecad-arc-slot-physical/temporary \
ONECAD_LOG_DIR=/private/tmp/onecad-arc-slot-physical/logs \
ONECAD_LIBRARY_ROOT=/private/tmp/onecad-arc-slot-physical/data/library \
ONECAD_WORKER_PATH="$PWD/worker/build-sketch-native/onecad-worker" \
  target/debug/onecad-ui --qa-root /private/tmp/onecad-arc-slot-physical
```

Add `--open /absolute/path/to/copied-fixture.onecad` for the prepared-profile
workflow. The host copies external fixtures into its QA documents directory;
fresh-process reopen must use the saved copy. Settings, recents, autosave,
recovery, library, temporary imports and logs remain in the isolated profile.
Verify actual paths and archive logs before restarting. Never clear production
storage. See [host instructions](../../crates/onecad-ui/README.md) and the
[native workflow](../design/native-ui/native-workflow.md) for immutable
ONECAD-273 checkpoints and their semantic gate.

## Foreground execution

Announce the foreground interval before taking mouse/keyboard input. Verify
Accessibility and Screen Recording permissions and identify the QA app's process
and window. Do not attach to an original document or stop unrelated processes.
Use new QA documents saved to a dedicated temporary directory; copy supplied
fixtures before opening them. Record hashes of any original fixtures and verify
they remain unchanged. Stop input on focus/ownership loss and release held keys.

Draw/constrain/change a rectangle, drag and release a corner, finish its selected
profile, Extrude, edit upstream dimensions, undo/redo, save, quit, reopen and edit
again. Record dimensions and authoritative geometry as well as visible status.
Retain exact input values, selections, timestamps, screenshots, bounded logs,
source revision, app/worker hashes and the action journal.

Automated frontend tests, real-worker integration tests, accessibility actions
and foreground pointer/keyboard acceptance are separate evidence classes.
Permission denial, unavailable capture and skipped checks are blockers, not
passes. Stop only test-owned processes when the session ends; retain QA files
and evidence for review.

## From-New plate semantic checks

Author a fully constrained 80×50 mm plate with two Ø6 mm holes, each 15 mm
from its opposing width edge and on a shared horizontal construction centerline
25 mm from the height edge. Construction geometry must not enter the material
profile. Extrude 4 mm, change width to 90 mm, Undo to 80 mm, Redo to 90 mm,
then save, quit cleanly and reopen in a fresh process. Verify Finish and sketch
re-entry preserve the dimensions and fully constrained solve.

Copy each clean GUI save into a fresh capture directory, without overwriting:
`plate80-4.onecad`, `plate90-4.onecad`, `plate-undo80.onecad`,
`plate-redo90.onecad`, and `plate-reopened90.onecad`. Preserve actual input,
history and build evidence separately. Run:

```sh
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build-sketch-native/onecad-worker" \
ONECAD_FROM_NEW_CAPTURE_ROOT=/absolute/path/to/captures \
  cargo test -p onecad-app --test native_from_new_capture \
  native_from_new_capture_semantics -- --exact --ignored --nocapture
```

Each capture opens in a fresh service process and must regenerate successfully
before body queries. The gate checks stable identities, profile boundaries,
construction exclusion, analytic mass properties and checked mesh bounds.
After body qualification, an isolated enter/cancel checks the editing solver's
current DOF; fresh-open regeneration alone does not populate that solver cache.
Capture hashes must remain unchanged. This semantic gate supplements actual
foreground input and history evidence; it does not establish them by itself.
