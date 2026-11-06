# Pinned public CAD inputs

These immutable upstream files support ONECAD-527. `manifest.json` records each
file's exact bytes, SHA-256, source URL, immutable revision/archive digest and
license. They are offline input fixtures, not approved output goldens. Never
replace their bytes to make a geometry test pass. Native feature histories and
authored occurrence/joint intent must be stored separately from imported STEP.

## Sources and attribution

- NIST FTC07–10: National Institute of Standards and Technology, PMI Test Models
  (2014), geometry-only STEP and accompanying drawings. The original archive's
  public-domain statement and disclaimer are retained in `nist/README.txt`.
  Acknowledge NIST; no endorsement or logo use is implied. The manifest pins the
  definitions archive bytes and exact member paths rather than treating its
  mutable download URL as immutable provenance.
- Framework Expansion Cards: © 2021 Framework Computer Inc, CC BY 4.0. The exact
  upstream license and README are retained. The STEP and drawing are unmodified.
- Original Prusa MINI filament sensor: Prusa Research, GPL v3 license text and
  upstream README retained. Box, cover and lever remain separate original STEP
  inputs. Their inclusion does not assert mechanical joints or native history.
- Robonine simulation clamp meshes: © 2025 Robonine, CERN-OHL-P-2.0. Only the
  two explicitly attributed visual clamp STLs are admitted, alongside the
  Apache-2.0 xacro description, REUSE license map, notices and license texts.
  The xacro's 0.001 scale supplies the STL-to-metre unit convention. Its
  authored poses are simulation data, not physical measurements or native
  construction history. No vendor arm, servo, camera or fastener mesh is copied.

The Robonine gripper's source revision, size and digest are recorded in the
manifest's `quarantined` section, but its bytes are not redistributed here.
Its embedded vendor-derived content must be audited before that STEP is admitted.
The licensed simulation subset does not establish equivalence to it. The pinned
guide and STEP name different clamp/rack revisions, and the README's 84 mm stroke
differs from the xacro's 74 mm common coupled range. Neither is accepted as a
physical travel oracle. OneCAD does not claim the xacro's mimic coupling.

## Independent drawing observations

Visual inspection of FTC09 drawing page 1 confirms inches, nominal stock
thickness `.1195` inch (3.0353 mm), six explicitly annotated `.250`-inch holes
in two groups of three, and separate `.234`-inch holes. Bracketed dimensions are
millimetres. These are source drawing observations, not measured import results.
Further dimensions require the model/remaining drawing views as stated in the
drawing; do not invent missing native construction dimensions.

Visual inspection of `ExpansionCard_Printable_M2.pdf` confirms a flat 0.80 ±0.08
mm plate, width 26.05 ±0.20 mm, height 29.70 ±0.20 mm, two 2.20-mm holes and a
Molex connector reference. These bounds must not be used as the bounds of the
whole enclosure STEP. The STEP contains a product hierarchy including PCBA,
frame clip, nuts and screws. Enclosure qualification must select and inspect
the correct subtree; flattening the entire input cannot establish that oracle.

FTC09 STEP declares inches; FTC10 declares millimetres. FTC07/08 contain multiple
unit contexts that need proper representation-context resolution. Framework and
Prusa declare millimetres. A text match for a unit or solid entity is not an
independent semantic geometry measurement.

## Qualification boundaries

Input integrity, production import, independent dimensions/volume/openings,
native reconstruction/edit history, static occurrences, joints and native
platform workflows are separate acceptance tiers. Hash verification alone
qualifies only input identity. No import, geometry, part-history, assembly,
performance, macOS or iPad acceptance is claimed by this directory.
