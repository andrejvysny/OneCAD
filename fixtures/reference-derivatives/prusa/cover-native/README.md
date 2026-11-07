# Prusa cover native-history construction

The source is `fixtures/public-cad/prusa/MINI-fsenzor-cover.stp`, SHA-256
`e6391172f932b141483f7b8b1a0b560c4260ba1116e588873916ba0ecacb2bd5`.
Original Prusa MINI / MINI+, PRUSA RESEARCH, source revision
`853bc30c4b10190f1d669ed6d0a567e333c28f21`, GPL-3.0-only. Preserve the
attribution and license in `fixtures/public-cad/prusa`.

`extract.py` reads the pinned STEP with `cadquery-ocp-novtk` 8.0.1.1 and writes
`geometry-extraction.json`, `lower-stock-recipe.json`, and `recess-recipe.json`.
The report records 61 analytic faces and their line/circle wire occurrences.
Run it from any working directory using the prepared OCP Python environment.
Extracted values are same-kernel construction evidence, not an independent oracle.
`stock-recipe.json` separately preserves the earlier top-contour-only diagnostic.

The top contour extruded four millimetres is not a valid full stock: a same-kernel
source-minus-stock Boolean leaves 368.975962728474 mm³. The lower exposed contour
has a notch where the tab joins; bridging its existing endpoints gives a 12-segment
lower contour. Its four-millimetre extrusion misses 63 mm³, corresponding to the
5 × 2.8 × 4.5 mm tab. These Boolean volumes are diagnostics, not proofs.

The native test constructs a 32-record Sketch/Extrude/Revolve history from the
lower contour: four exact perimeter bevel cuts, tab addition, rounded underside
recess, revolved conical hinge, and two four-stage fastener cuts. Each fastener
combines a 3.05 mm radius counterbore, a circle clipped by a 3.2 mm stripe, a
3.2 mm square, and a 1.6 mm radius through cut. It does not import the source
STEP or embed imported blobs or library archives. A test may retain the native
file by setting `ONECAD_COVER_CAPTURE` to a new nonexistent path; creation is
exclusive and the test prints the SHA-256. Such a file is not an approved golden.

The resulting BRep volume and the same-kernel STEP diagnostic volume from
`fixtures/reference-oracles/prusa-sensor.json` are logged; their difference is
not asserted. Fine-mesh bounds are compared to the STEP diagnostic.
Native BRep/mesh volumes and sorted mesh spans are also checked against the
independent upstream STL report in `fixtures/reference-oracles/prusa-cover-stl.json`
using its existing 2% volume and 0.15 mm bounds trials. The sorted-span comparison
assumes an unqualified axis-permutation alignment. These broad trials do not
establish exact BRep shape equivalence, physical metrology, or independent-kernel
accuracy. No existing oracle or tolerance was changed. The fine mesh also has closed, oppositely wound edges, one connected component,
and Euler characteristic −4, matching the upstream STL report. Triangle
connectivity uses shared oppositely directed edges, as in the STL reader. Vertices
are matched by exact float positions (signed zeros normalized), without a
geometric weld tolerance. These checks do not establish vertex manifoldness. Undo/redo, wrong-kind atomic refusal, explicit save, a separate
process with a fresh worker reopening the file, and a geometric edit followed
by undo are exercised. In the recorded run, a zero-length same-kind cut was
accepted into history and reported `Extrude distance too small` as a failed
feature on regeneration; undo restored the prior geometry. The test also accepts
an atomic refusal of that invalid edit. Native GUI and exact shape equivalence remain
unqualified.
