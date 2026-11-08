# Retained compact public assembly input

`native-compact-10x20.onecad` preserves the exact retained QA input bytes:
SHA-256 `7c39c64fa89618fcb7cf40dcb2a230b0f3f68acd2d09f4fd7c8af58be006ae9d`.
It contains ten distinct embedded definitions and twenty authored occurrences.
`manifest.json` records the six unchanged imported payloads and their source paths.
This is a measurement input, not an output golden or an independent geometry oracle.

The source fixture and capture procedure are in
[the shared-service stress workflow](../../../crates/onecad-app/tests/public_mixed_stress_support/README.md).
The compact placements are test-authored and separated to satisfy the existing
renderer precision bound. Recreating the workflow can change container identity
or metadata; do not replace these pinned bytes with a new capture to obtain a
passing result. The input contains no authored joint or physical mate intent.
The application timing example adds explicitly symbolic logical joints.

## Attribution and source distinctions

- Prusa Research: imported MINI sensor box plus source-derived native cover and
  lever histories. Preserve the [upstream README](../../public-cad/prusa/README.md)
  and [GPL v3 license](../../public-cad/prusa/LICENSE). Native cover and lever
  construction recipes remain in `crates/onecad-app/tests`; they contain 32 and
  8 features respectively. Imported box geometry is not a native history.
- Copyright (c) 2025 Robonine: four scoped analytic imported frame/clamp/rack/pinion
  leaves under CERN-OHL-P-2.0. Preserve the
  [derivative source record](../../reference-derivatives/robonine/analytic/README.md),
  [notice](../../public-cad/robonine/NOTICE),
  [licensing map](../../public-cad/robonine/LICENSING.md) and
  [license text](../../public-cad/robonine/LICENSES/CERN-OHL-P-2.0.txt).
  No vendor assembly, servo or other quarantined geometry is included.
- National Institute of Standards and Technology: imported FTC09 STEP geometry.
  Preserve the [original public-domain statement and disclaimer](../../public-cad/nist/README.txt).
  No NIST endorsement, native FTC09 reconstruction or qualified accuracy is implied.
- Two synthetic OneCAD native plate histories are test inputs, not public product
  geometry or physical-fit evidence.

The pinned upstream files and provenance remain in
[the public input manifest](../../public-cad/manifest.json).
The application timing example is documented in
[its procedure](../../../crates/onecad-app/examples/application_joint_perf_support/README.md).
Static correctness, native histories, joints, resources, GUI input, presentation,
performance and latest-platform qualification remain separate tiers.
