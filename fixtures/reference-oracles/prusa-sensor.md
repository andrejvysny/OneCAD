# Prusa MINI sensor source-reader evidence

The pinned GPL-3.0-only box, cover, and lever STEP inputs are listed in
`../public-cad/prusa/README.md`. `prusa-sensor.json` records their SHA-256,
STEP entity links, millimetre bounds, surface areas, solid volumes, and two
candidate cylindrical surfaces. Reproduce it with Python and
`cadquery-ocp-novtk==8.0.1.1`:

```sh
python fixtures/reference-oracles/inspect_prusa_sensor.py \
  fixtures/public-cad/prusa fixtures/reference-oracles/prusa-sensor.json
```

This source-reader diagnostic uses the OCCT kernel family, as does OneCAD. It
is not an independent-kernel or physical-geometry qualification. Each file
contains one solid in this reader. The source entity associations are:

- Box: `ADVANCED_FACE #3042` → `CYLINDRICAL_SURFACE #77`, radius 1.4 mm,
  origin (-16.0000009536743, -13.5000000298023, 4.00000005960464) mm,
  direction (0, 0, -1).
- Lever: `ADVANCED_FACE #515` → `CYLINDRICAL_SURFACE #16`, radius 1.65 mm,
  origin (-16.0315999507904, 4.00000005960464, 13.5072009086609) mm,
  direction (0, -1, 0).

The reader uniquely finds each candidate by its exact source cylinder data.
[Prusa's current guide](https://help.prusa3d.com/article/noisy-filament-sensor-mini_162222)
identifies the centre M3x12 screw as the lever hinge and the outer screws as
cover fasteners. It does not establish that these two faces are its mating
surfaces. The guide is not pinned to the STEP revision. The box candidate's
trimmed Z range is approximately -9 to -6 mm; the lever candidate's trimmed Y
range is approximately -2 to 2 mm. A simple axis-coincident placement does not
make these ranges overlap. Hardware seating, clearance, travel stops, and
physical assembly placement remain unverified.

The associated test proposes only a mathematical fixture: ground the box,
rotate the cover and lever +90 degrees about X, translate the lever to align
candidate axis origins, rigidly attach the cover, and drive the candidate hinge
by 0.1 rad. These poses, chosen X directions and drive are test inputs, not
source-derived physical constraints. No travel limits are asserted. Its mesh
comparison uses 0.15 mm bounds and 2% volume trial tolerances to accommodate
tessellation; neither is a physical accuracy guarantee. Source components remain
`ImportStep` histories. UI interaction, independent-kernel verification,
Robonine, and complete 531/533 qualification are outside this evidence.
