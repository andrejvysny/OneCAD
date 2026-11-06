# Robonine simulation source observations

The input manifest admits only the two explicitly licensed visual clamp meshes
and their source/attribution records at revision
`e7cb78a7a1cd073ea8f845486398a23549343c7b`. Copyright © 2025 Robonine;
mesh license CERN-OHL-P-2.0, robot description Apache-2.0. Retained upstream
REUSE, NOTICE and license texts identify the separate sources. No vendor arm,
servo, camera or fastener geometry is redistributed.

`robonine-simulation.json` freezes source-input observations before OneCAD
measurement. `inspect_robonine_simulation.py` uses only Python's standard
library: checksum-pinned binary STL decoding, exact f32 vertex keys,
edge-incidence/winding and vertex-link checks, signed triangle sums, bounds,
and composition of the pinned xacro's joint and visual origins. It does not
use OCCT or OneCAD. Reproduce into a separate diagnostic file:

```sh
python fixtures/reference-oracles/inspect_robonine_simulation.py \
  fixtures/public-cad/robonine /tmp/robonine-source-observations.json
```

Both meshes have 8,808 triangles, 4,400 distinct bitwise vertices and 13,212
edges. Every edge has two oppositely directed incident faces; each vertex link
is one cycle. These checks do not prove absence of self-intersections, outward
orientation, physical volume, valid BREP conversion or equivalence to the
quarantined STEP. Signed triangle sums are source diagnostics, not certified
solid volumes. No native construction history is inferred from a mesh.

At q=0, both mesh-to-`link5_1` transforms have identity rotation and translation
(-7.062, 120.627, -152.066) mm. The mixed vendor parent mesh is excluded;
`link5_1` is only an abstract coordinate frame. The 1e-9 mm tolerance applies
solely to arithmetic on authored decimal pose values, whose double-precision
rounding is much smaller at this scale. It is not a physical-fit, mesh-bounds
or STEP-equivalence tolerance.

The source xacro makes the left slider mimic the right. OneCAD does not support
that coupling; independent slider authoring must be labeled as a separate
exercise. The common source simulation range gives 74 mm relative travel,
while the README advertises 84 mm. Physical variant and STEP correspondence
remain unresolved. This fixture does not complete ONECAD-531 or ONECAD-533.
