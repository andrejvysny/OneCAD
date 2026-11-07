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
917.0862260401047 mm³. The test compares native BRep volume within the existing
Prusa diagnostic's 2% bound and fine-mesh vertex bounds within its 0.15 mm bound;
these broad checks are not proof of exact shape equivalence. It also exercises
undo/redo, invalid operation-type edit refusal, explicit save, fresh-process
reopen and a geometric edit followed by undo. Native GUI, independent-kernel
comparison and source-level geometric equivalence remain unqualified. No
existing oracle, source tolerance, or golden was changed.

To retain a native document for later GUI review, set `ONECAD_LEVER_CAPTURE` to a
new, nonexistent output file before running the test. Creation is exclusive and
the test prints its SHA-256. The output is a test artifact, not an approved golden.
A zero-distance same-kind cut edit is accepted into history and fails regeneration
with `Extrude distance too small`; undo restores the preceding geometry.
