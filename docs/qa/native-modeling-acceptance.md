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

## Shared desktop and iPad modeling commands

Both hosts use `onecad-ui`, `onecad-interaction` and the same application
services. The contextual pill exposes the common operations; **More tools**
lists additional commands and explains selection requirements. On iPad, use
the selection toolbar's multi-select mode for multiple operands. On desktop,
use multi-select or the selection modifier.

| Command | Starting selection | Controls |
|---|---|---|
| Extrude | Closed sketch profile | Distance, symmetric or through-all extent; draft for blind extent; new/add/cut/intersect and target body |
| Revolve | Profile containing a sketch line, or profile plus body edge | Sketch-line axis chooser, angle, new/add/cut/intersect and target |
| Loft | 2–32 finalized closed sketch profiles in authored order | Smooth/ruled solid New Body; reorder, remove, add or replace sections |
| Offset plane | No geometry required | XY/XZ/YZ base and signed offset; creates a frozen datum for sketch support |
| Fillet / Chamfer | Edges of one body | Radius / equal-leg distance; prepared tangent closure |
| Offset face | Faces of one body | Signed offset, or radius/diameter for a single cylindrical face; prepared face closure |
| Shell | Faces to remove from one body | Wall thickness |
| Hole | One planar face | World center, diameter, blind/through-all, simple/counterbore/countersink |
| Move / Rotate | One body | Translation, rotation and keep-original copy |
| Boolean | Target body followed by one or more tool bodies | Union/subtract/intersect, target chooser and keep tools |
| Linear pattern | One body | Spacing, count, direction, separate/fused result |
| Circular pattern | One body | Angle, count, axis origin/direction, separate/fused result |
| Mirror body | One body | Plane point/normal, separate/fused result |
| Gear generator | No geometry required | Spur gear module, height, pressure angle, tooth count, world center and optional bores |

New candidates use actual worker previews. Create is disabled while expression
evaluation or preview is pending, invalid or refused. Cancel must leave no
document edit. A changed option invalidates the previous preview, and an obsolete
worker answer must never re-enable Create. Edit a saved feature through its
history row. Existing-feature edits use full-prefix replay preview and rebuild
downstream features on Apply; previews must leave the authoritative head unchanged.

Qualify these commands on **both** physical hosts: create, change every applicable
control, cancel, create again, undo/redo, save, quit and reopen. Include compact
iPad layouts and keyboard-free value entry. Choice controls use
`onecad.option.*` automation identities; numeric fields and footer buttons retain
the existing `onecad.param.*`, dimension-input and operation identities.

The GPU-free integration lane is:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  cargo test -p onecad-interaction --test real_modeling_creation
```

It authors real candidates through the reducer, checks body counts/volumes and
undo on a solid plate, and saves/reopens the Boolean result. The existing
`real_edge_ops` lane covers extrude, fillet, chamfer, offset and transforms. These
are service/interaction checks, not substitutes for physical host acceptance.

Remaining distinctions to preserve during qualification:

- Sweep has no worker executor. FeaturePattern preview refuses because
  authoritative source expansion is missing. Their commands remain disabled.
- Basic Loft accepts closed regions without holes on parallel, distinct planes
  in forward or reverse offset order. Open curves, point or face sections,
  guide rails, end conditions and Boolean result modes are explicitly refused.
- To-face/two-direction extrusion, asymmetric chamfer reference-face picking,
  total-wall face offsets and multipoint/threaded hole
  authoring still require additional operand/mode controls. Existing supported
  scalar fields can be edited without inventing missing references.
- Gear creation uses a world frame and spur geometry. Face seating, helical gears
  and advanced coefficient controls are not part of this entry flow.
- Shelling a 60×40×4 plate **after** a centered through-hole can produce a valid
  preview and committed geometry but an ambiguous split-face repair report.
  Treat that as NeedsRepair; do not report a clean success or weaken reference
  scoring. Shell-before-hole on the solid plate is the basic acceptance case.
- Library component placement/detachment, standalone draft, advanced datum tooling and
  extension commands are separate workflows; adding a catalog label does not
  establish that they execute.

## Basic Loft qualification

Create offset planes from XY at 0, 10 and 20 mm. Choose each resolved datum in
the Sketch support chooser (or double-click its Explorer row), create a fully
constrained circle, and Finish. Use radii 5, 8 and 5 mm respectively. Enable
multi-select and pick the three finalized profile fills in section order, then
choose Loft. Verify smooth preview, Cancel without a history edit, and creation
of one body. Source sketches remain visible for subsequent section selection.

Edit Loft in History. Enable Ruled and verify volume 860π mm³. Reverse the section
order and verify the same solid volume; a nonmonotone order must refuse. Verify
section replacement/addition/removal invalidates the previous preview. Change the
middle sketch radius to 9 mm and verify downstream regeneration. Create a centered
Ø2 mm through-hole from the planar top face; its volume reduction is 20π mm³.
Undo/redo, save, quit and reopen; verify section order, identities, dimensions and
the downstream hole. Invalid section choices must not mutate the committed model.
After reopening, select Loft in History and edit its Ruled choice again. Verify
Edit remains available for choice-only operations (Loft, Boolean and Mirror),
and that Apply preserves later features. Display capture must retain all
budget-admitted history records, including those without numeric fields.

`cargo test -p onecad-app --test loft_ops` with required real-worker settings
checks this lifecycle through shared services. `cargo test -p onecad-ui --lib
loft_ui::tests::` checks section-control input and selection fencing. These checks
do not replace native mouse, touch or Pencil qualification.

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

## Separate plate depth and reopen captures

Keep the existing 17-constraint, 4 mm width-edit gate unchanged. Capture the
following separate journey from an actual native GUI session using the same
mounting plate, including both holes and the authored construction centerline:

| Immutable save | Width | Extrusion depth |
|---|---:|---:|
| `committed-6.onecad` | 80 mm | 6 mm |
| `edited-8.onecad` | 80 mm | 8 mm |
| `undo-6.onecad` | 80 mm | 6 mm |
| `redo-8.onecad` | 80 mm | 8 mm |
| `width90-8.onecad` | 90 mm | 8 mm |
| `reopened90-8.onecad` | 90 mm | 8 mm |
| `further-edited90-6.onecad` | 90 mm | 6 mm |

The reopened checkpoint requires clean Quit and a fresh process opening the
saved QA copy. Edit again in that fresh process before saving the last capture.
Keep input traces, clean-quit evidence, source/worker hashes and screenshots with
the run. Copy saves without overwriting any earlier checkpoint. Run:

```sh
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_FROM_NEW_DEPTH_CAPTURE_ROOT=/absolute/path/to/depth-captures \
  cargo test -p onecad-app --test native_from_new_capture \
  native_from_new_depth_capture_semantics -- --exact --ignored --nocapture
```

The gate independently opens each immutable capture in a fresh service process.
It checks the 17 constraints, both hole centers and diameters, construction
exclusion, persistent sketch/entity/constraint/region/operation/body identities,
eight BRep faces, analytic volume and surface area, centroid, checked mesh bounds,
fully constrained re-entry and unchanged capture hashes. It never manufactures
GUI evidence. Missing captures or required worker prerequisites fail the lane.
The older prepared-profile depth gate remains separate and does not qualify
holes or the From-New plate.
