# Public reference oracles

These records retain evidence from pinned public inputs. They are not generated
from OneCAD output. Keep imported-geometry checks distinct from native authored
feature histories and from a second-reader or second-kernel crosscheck.

`nist-ftc09.json` traces two identical upstream area/volume properties to the
same sole solid in the admitted NIST STEP. Inch units are explicit. The declared
source connectivity uncertainty is about 0.03381637 mm; it does not independently
justify a particular relative mass-property tolerance. The recorded `1e-6`
relative threshold was frozen as a trial before running OneCAD and must not be
widened after observing a failure. Independent numerical justification remains
part of qualification.

The [direct FTC09 STEP trace](nist-ftc09-direct/README.md) checks pinned source
entities, units, selected face loops and two-edge circle pairs using only the
Python standard library. It removes the OCCT transfer dependency for those
observations; it does not establish arc coverage, mass or the trial tolerance.

Run the production-worker check with the repository's staged pinned worker:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  cargo test -p onecad-app --test public_reference_inputs -- --nocapture
```

The test removes its temporary STEP after saving, then opens on a fresh worker
and checks identity and exact reproduction of the measured mass properties.
Its history remains `ImportStep`. To admit a new container seed after these
checks pass, set `ONECAD_REFERENCE_CONTAINER_OUT` to a new artifact path. Capture
uses exclusive creation and will not overwrite an existing fixture. Record its
hash, producing source, worker/kernel/toolchain and original STEP digest when
admitting it. Never recapture an existing failing fixture to obtain green tests.

The Framework PDF under `public-cad` describes a PCB plate. It must not be used
as a whole-enclosure dimension or mass oracle for the larger STEP assembly.

`framework-parasolid-measurement.json` records an independent Shapr3D/Parasolid
measurement of the pinned STEP's enclosure solid, selected through its imported
product hierarchy. Native accessibility values and the visible Measure panel
agreed on 2002.3754 mm³ and 3197.8774 mm². The four displayed decimal places bound
rounding only; import conversion and healing uncertainty remain unmeasured.
This observation does not replace an existing oracle or justify widening a
tolerance. It does not establish native OneCAD history or source equivalence.
## Upstream Prusa source tessellations

The separately pinned, unmodified box, cover and lever STLs come from the same
Prusa revision as the STEP inputs. `inspect_prusa_source_stl.py` uses only the
Python standard library to read binary triangles, verify opposite edge winding,
count edge-connected components, and calculate bounds, area and signed volume.
Its analytic tetrahedron and malformed-input tests run with:

```sh
python -m unittest discover -s fixtures/reference-oracles -p test_prusa_source_stl.py
python fixtures/reference-oracles/inspect_prusa_source_stl.py \
  fixtures/public-cad/prusa/MINI-fsenzor-lever.stl /tmp/new-lever-report.json
```

For box and cover, pass `--expected-sha256` with the respective exact digest from
`fixtures/public-cad/manifest.json`. Existing reports are never overwritten.
The checked reports are `prusa-box-stl.json`, `prusa-cover-stl.json` and
`prusa-lever-stl.json`. Each has one edge-connected component and closed opposite
winding. Euler characteristics are -14, -4 and -4 respectively; those values
alone do not certify vertex manifoldness, self-intersection freedom or hole
geometry. The upstream meshes are independent of OneCAD output; their export
engine and meshing deflection are not certified.

The STL and STEP poses differ for cover and lever. The native lever regression
compares sorted bound spans under an explicitly unqualified axis-permutation
assumption, plus volume and the closed-mesh Euler characteristic. Sorted AABB
spans are not invariant under arbitrary rotation and do not prove alignment. It does not compare mesh bytes across kernels.
The 0.15 mm and 2% numerical bounds reuse existing Prusa trial diagnostics;
they are not a ratified exact-BRep or physical accuracy budget. These coarse
checks and topology do not uniquely establish shape equivalence.
