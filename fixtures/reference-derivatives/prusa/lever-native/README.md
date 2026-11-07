# Source-derived native lever recipe

`recipe.json` records analytic geometry extracted from
`fixtures/public-cad/prusa/MINI-fsenzor-lever.stp`, SHA-256
`beee16c2e5c09d26777d4971df8c343d467f926121d1753c6b41bc0b5ff6bbc6`.
Source: Original Prusa MINI / MINI+, PRUSA RESEARCH, source revision
`853bc30c4b10190f1d669ed6d0a567e333c28f21`, GPL-3.0-only. Preserve the source
attribution and license in `fixtures/public-cad/prusa`. The numeric recipe and
source diagnostic were extracted with `cadquery-ocp-novtk` 8.0.1.1. They are
same-kernel evidence, not an independent oracle.

The native test constructs eight history records: a 16-segment analytic XZ
outer sketch and 8 mm extrusion, the 6.25 × 2.25 mm through-slot cut, a revolved
hinge cut, and a tilted transverse cylindrical cut. The hinge has radius
1.65 mm over its central 4 mm, with opposed 5° and 15° conical portions. The
transverse radius is 2.06963673538225 mm. Their source axes and transition
coordinates are in the JSON. Hinge and transverse cutters extend beyond the
stock to ensure through cuts; their interior profiles and axes are unchanged.
No source STEP import, imported blobs, or library archives are used.

The uncut outer extrusion has a same-kernel source-derived volume of
1137.0780439176528 mm³. The source's same-kernel volume is
917.0862260401047 mm³. The test reuses the numerical 2% volume and 0.15 mm bounds from the Prusa
import/tessellation trial diagnostics. Applying 2% to native BRep mass here is a
construction diagnostic, not an established BRep acceptance tolerance. These
broad comparisons do not prove source equivalence. It also exercises
undo/redo, invalid operation-type edit refusal, explicit save, fresh-process
reopen and a geometric edit followed by undo. Native GUI, independent-kernel
comparison and source-level geometric equivalence remain unqualified. No
existing oracle, source tolerance, or golden was changed.

An additional independent source-data check uses Prusa's upstream STL, read
without OCCT. It compares sorted bound spans under an unqualified axis-permutation
assumption, and volume under the same numerical trial bounds. Only volume is
invariant under arbitrary rigid motion; this does not establish common pose. Both meshes must have opposite edge winding
and Euler characteristic -4, with exact position welding for OneCAD's decoded
MESH1 (signed zero is canonicalized; no weld distance is used). This catches
loss of through-hole topology that gross volume/bounds alone can miss. It is
not an independent exact-surface or physical measurement. The source report,
reader and limitations are in `fixtures/reference-oracles`.

To retain a native document for later GUI review, set `ONECAD_LEVER_CAPTURE` to a
new, nonexistent output file before running the test. Creation is exclusive and
the test prints its SHA-256. The output is a test artifact, not an approved golden.
The test also probes a zero-distance same-kind cut edit, accepting either a
refusal or an explicit failed feature followed by undo. In the observed run, the
edit entered history and regeneration reported `Extrude distance too small`,
publishing the preceding feature prefix before undo restored the complete part.
This is low-level diagnostic behavior, not native UI invalid-edit acceptance.
