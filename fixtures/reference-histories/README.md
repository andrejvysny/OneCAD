# Reference history inputs

`nist-ftc09-curves.json` is an intermediate recipe for an editable sketch, not a
completed native model, geometry oracle, or qualification fixture. It was
extracted from the NIST FTC09 STEP in `fixtures/public-cad/nist`, SHA-256
`d12866d3ab0fc260efd0f1a5c54af47950be513486f61f65513877045582afa8`.
NIST produced the original public-domain model; its use here does not imply
NIST endorsement. See the upstream `README.txt` for terms.

On macOS arm64, Python 3.13.7 and the official PyPI wheel
[`cadquery-ocp-novtk==8.0.1.1.0`](https://pypi.org/project/cadquery-ocp-novtk/8.0.1.1.0/)
were used in an isolated virtual environment. Wheel SHA-256:
`78b9033a47d72049b290c18e2c4c80bffc56c055b52eb60f49aca76550a9ddc8`.
Regenerate with that environment:

```sh
python fixtures/reference-histories/extract_ftc09.py \
  fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp \
  fixtures/reference-histories/nist-ftc09-curves.json
```

The script checks the source hash and selected face planes. It maps source X/Z
to sketch X/Y and source Y to extrusion Z. The base plane has 35 wires and 114
source edges; two blind pockets are 1.27 mm deep. The actual STEP base thickness
is 3.03784 mm; the drawing's nominal 0.1195 in is 3.0353 mm. Four conical
countersinks each increase radius from 2.9718 to 3.7338 mm over 0.762 mm.
The recipe retains line and circular-arc parameters, not STEP or imported-face
references. Its values alone do not establish editable feature replay.

The reader wraps OCCT 8.0.1, the same kernel family used by the worker. This
extraction is not an independent geometric check. The frozen source-authored
mass properties and trial tolerance are documented in `fixtures/reference-oracles`.
The `reference_ftc09_probe` real-worker test authors the base and two pocket
sketches, three extrusions and a four-point countersink Hole. It checks the frozen
trial volume and area, undo/redo, save and fresh-process reopen, and edits after
reopen, and rejects an invalid countersink edit without changing the document.
It does not import a STEP or BRep into the history. It is not full public
reference qualification: cancel and topology repair cases, native host UI lifecycle,
independent tolerance justification and a second reader remain open.

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  cargo test -p onecad-app --test reference_ftc09_probe -- --nocapture
```

`nist-ftc09-provenance.json` pins the source, extractor, recipe and reader wheel
hashes and analytic source-coordinate bounds. The worker test checks those
hashes, the transformed mesh bounds to float32 precision, and actual circular
edges at all four source through-hole and countersink centres. The edge check
uses 128 float64 machine epsilons scaled by coordinate magnitude; it checks
numeric replay of the captured inputs, not independent source uncertainty. The
source STEP reader reports an additional 0.0000001 mm bounding tolerance on the
upper Y bound. The enclosing sketch remains underconstrained (319 DOF).

## Framework source inventory

Framework Computer Inc.'s Expansion Card reference STEP is distributed under
CC BY 4.0; its license and original README are in `fixtures/public-cad/framework`.
`framework-step-inventory.json` records STEPCAF occurrence paths, product labels,
face types, bounds, area, volume and a hash of these diagnostics for all six
source solids. Bounds use the leaf shape location, not the accumulated assembly
transform. Fingerprints are tied to the pinned reader and numeric serialization.
They are not persistent topology identifiers. The inventory does not define
a native history or an independent mass oracle. In particular the enclosure is
`FW_EXP_1USBC_FRAME_CLIP_BC_229_`, not the PCB or the whole source assembly. The
source PDF describes the PCB and is not an enclosure drawing. The enclosure has
18 B-spline faces, whose feature origins remain to be identified. Surface type
alone does not establish that reconstruction is unsupported.

Using the same isolated OCP environment above, regenerate the inventory with:

```sh
python fixtures/reference-histories/inspect_framework.py \
  fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp \
  fixtures/reference-histories/framework-step-inventory.json
```

The source STEP SHA-256 is
`6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09`.

For the FTC09 test, the raw recipe coordinates retain source X/Z. Authored
sketch coordinates are `(-source Z, -source X)`, because OneCAD's XY sketch
basis maps local X to world +Y and local Y to world −X. With extrusion along
world +Z, the source-to-world rotation is:

```text
[world X]   [1  0  0] [source X]
[world Y] = [0  0 -1] [source Y]
[world Z]   [0  1  0] [source Z]
```

Its determinant is +1. Arc directions are reversed in sketch coordinates to
preserve the same circular segments under that coordinate change. The worker
test checks the proper rotation, an asymmetric source hole, and the top face's
oriented normal; mass properties alone cannot detect a reflection.

`framework-enclosure-profiles.json` is extracted by
`extract_framework_profiles.py`; `framework-enclosure-provenance.json` pins the
source and recipe hashes. `reference_framework_probe` builds an **incomplete**
native history: stock, source-derived pocket and boss profiles, two blind
countersunk holes, exterior radii, underside, slot and clip-floor cuts, a connector core and
lip and inner flare with two revolved halfcaps, and an exact ruled-Loft/Boolean rear ramp. The straight side grooves cover
only source-coordinate Z −2.203927026704 through −26.0 mm; their ends and other source
features remain. No STEP or BRep is imported into the authored history.

The partial worker test checks native feature kinds, no import blobs, undo/redo,
save and reopen using a new runtime and worker in the same test process. It is
not a native-host restart, a complete model, or Framework qualification. With
the pinned OCP reader and the current real-worker export, the same-kernel
boolean diagnostic reports approximately 28.53 mm³ extra material and 0 mm³
missing material. This is not an independent geometry oracle. The source
STEP's 18 rational B-spline faces and the remaining side and exterior
features require reconstruction and source comparison.

```sh
python fixtures/reference-histories/extract_framework_profiles.py \
  fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp \
  fixtures/reference-histories/framework-enclosure-profiles.json
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  ONECAD_FRAMEWORK_DIAGNOSTIC_OUT=/private/tmp/framework-intermediate.stp \
  cargo test -p onecad-app --test reference_framework_probe -- --nocapture
python fixtures/reference-histories/compare_framework_intermediate.py \
  fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp \
  /private/tmp/framework-intermediate.stp
```

The optional `ONECAD_FRAMEWORK_EXPERIMENT_RAMP=1` test attempts a 3.0 by 0.3 mm
asymmetric Chamfer on the promoted five-edge tangent contour. With the tested
worker it fails at build with `OP_FAILED: Chamfer operation failed`. The regular
intermediate instead uses the two-profile ruled Loft and Boolean Cut. Earlier
extruding the 1.5 mm exterior connector capsule through the entire wall removed
2.978 mm³ of source material in the same-kernel boolean diagnostic; the history
uses the source's 1.3 mm core and 0.2 mm exterior lip instead.

The straight connector flare uses the source centerline and analytic meridian:
0.2 mm entrance blend, 53.130102° cone side, 1.0 mm blend and 0.2 mm
interior blend. It covers only the straight span between end centers; source
endcaps use signed 180° revolutions, followed by Boolean cuts.
`ONECAD_FRAMEWORK_ENDS_FULL=1` selects the failing full-360° endcap experiment;
`ONECAD_FRAMEWORK_ENDS_FUSED=1` additionally selects fused Revolve Cut. Both
full-360° sequences previously failed with `PUBLICATION_TOLERANCE_BUDGET` and maximum vertex
tolerance 0.0013410150097755923 mm; no limit was changed. These commands are diagnostic and may
fail; their outcomes do not establish general Revolve support.

Pinned upstream's [mechanical README](https://github.com/FrameworkComputer/ExpansionCards/blob/2904a95d60d4ba771c5c3c56d6fdba7078243fe8/Mechanical/README.md)
identifies OpenSCAD and Printable as separate enclosure variants. The pinned
Printable tree supplies STEP/STL and drawings, not the STEP's original editable
feature history. The OpenSCAD variant is not a substitute for this source.

Reopen compares exact solid and face counts, mesh vertex and triangle counts,
and f32 bounding-box bits; it requires mass within eight representable `f64`
values. This replay bound accounts for floating-point
summation order and is separate from any source-geometry acceptance tolerance.

`inspect_framework_blends.py` records exact STEP face adjacency in
`framework-blend-adjacency.json`. All 18 B-spline surfaces touch a source
0.3 mm cylinder; faces 59 and 119 also touch a 0.15 mm cylinder. This supports
identifying a connected blend network, but does not establish its original
feature order. Regenerate with:

```sh
python fixtures/reference-histories/inspect_framework_blends.py \
  fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp \
  fixtures/reference-histories/framework-blend-adjacency.json
```

The rear underside uses a −1° drafted cut and a 0.3 mm native edge fillet.
The worker test preserves the existing 1e−7 mm³ analytic slot checks by
authoring the slots first. `ONECAD_FRAMEWORK_UNDERSIDE_DRAFT=1` retains the
opposite-sign experiment, which was refused as `PUBLICATION_OPEN_MANIFOLD`
with three non-manifold edges; `0` selects the earlier unfilleted stock stage.

`ONECAD_FRAMEWORK_EXPERIMENT_CLIP_PLANES=1` replaces the source-floor
extrusions with four ruled Loft/Boolean tools derived from source support planes.
This is a pre-fillet overcut: source-minus-intermediate measured 0.426976 mm³.
With `ONECAD_FRAMEWORK_EXPERIMENT_CLIP_BLEND=1`, the grouped edge Fillet hit
the existing scheduler timeout. A later five-edge per-side selection reached
`History/NoCandidates` on a selected edge. In an OCP replay of the exported
preblend, that edge reported `IsDeleted=false`, no modified edges, one generated
face and no original edge in the valid result; this does not prove the production
worker's in-memory history. Neither experiment is a source match.

`ONECAD_FRAMEWORK_EXPERIMENT_REAR_CLIP_PLANES=1` additionally constructs the
rear opening with source-plane ruled-loft cuts. A same-kernel OCP trial found
4.143723 mm³ excess and 0.433448 mm³ missing source material with no open or
non-manifold edges. The alternative `ONECAD_FRAMEWORK_EXPERIMENT_REAR_TRIANGLE=1`
uses intersected lofts; adding `ONECAD_FRAMEWORK_EXPERIMENT_REAR_GAP=1` makes
its first gap cut fail worker manifold validation. These are unfinished preblend
models, not acceptance tolerances or source matches. Each switch requires
`ONECAD_FRAMEWORK_EXPERIMENT_CLIP_PLANES=1`.

With `ONECAD_FRAMEWORK_ROUND_CLIP_TOOLS=1`, the four half-cutters receive
0.3 mm fillets on their three source-contour edges before subtraction. The
worker published all cuts and passed undo/redo, but after reopening the exact
mesh signature differed despite matching bounds, body ID, solid count and face count. The exported
intermediate had 19.042815 mm³ excess and 0.131551 mm³ missing material in the
same-kernel source comparison. This is an experiment, not a source match; it
also requires `ONECAD_FRAMEWORK_EXPERIMENT_CLIP_PLANES=1`.
`ONECAD_FRAMEWORK_ROUND_CLIP_CORNERS=1` adds the two source-contour
longitudinal edges. An OCP trial reported zero source-minus-intermediate volume,
but its Boolean result disagreed with direct volume subtraction by 0.006123 mm³;
therefore it does not establish source containment. Before the JSON float-roundtrip
fix, the native v23 history published all four cuts and passed undo/redo, but its
reopened mesh counts differed. After the fix, the same partial history passed
its exact mesh signature and volume checks; 3,214 authored document f64 values
retained identical bits after saving and reopening. An offline default-feature
serde_json 1.0.150 parse of that post-fix document changed 86 of those values.
This is causal evidence for that replay failure, not the lost pre-fix document.
The pre-fix native export's Boolean comparison likewise
reported zero source-minus-intermediate while differing from direct volume
subtraction by 0.001934 mm³. The additional upper cut removed source material
in the OCP diagnostic. These observations do not establish source equivalence.

The optional `ONECAD_FRAMEWORK_EXPERIMENT_INBOARD=1` adds two plane-derived,
0.3 mm rounded inboard negative tools after the rear-plane cuts. A worker run
passed undo/redo and the exact reopened mesh signature; all 4,092 authored f64
values retained their bits through save/load. Its same-kernel source comparison
still found 3.453907 mm³ excess and 0.006401 mm³ missing material, so it is not
a source match. This experiment requires the rounded clip and rear-plane flags.

`inspect_framework_surfaces.py` samples each trimmed B-spline face on a fixed
19×19 interior UV grid, fits a centered least-squares sphere, and compares each
sample with its neighboring analytic source cylinders. The smallest sphere-fit
maximum residual among the 18 patches is 0.001810967 mm; the largest is
0.066686124 mm. The best adjacent-cylinder maximum residual is approximately
0.003153 mm. None is within the reader's 0.0000001 mm confusion value. The STEP
context reports 0.004470832106062 mm *connectivity* uncertainty; neither that
nor reader confusion proves a source surface approximation tolerance. These are
sampled diagnostics, not proof of exact analytic identity or a reason to change
an acceptance threshold. Results and the sampling script are pinned in the
Framework provenance file.

```sh
python fixtures/reference-histories/inspect_framework_surfaces.py \
  fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp \
  fixtures/reference-histories/framework-blend-adjacency.json \
  fixtures/reference-histories/framework-surface-fits.json
```
