# Native technical contracts

The native application is the root Rust workspace. Current build and test entry
points are in the [repository README](../../../README.md), with crate ownership
and invariants in [architecture](../../ARCHITECTURE.md).

Retained code-coupled contracts:

- [Native workflow](native-workflow.md): existing QA paths and workflow contract.
- [Data continuity](data-continuity.md): persistence compatibility requirements;
  implementation and qualification are separate from repository cleanup.
- [In-process experiment](inprocess-worker-spike.md): optional C ABI build and
  isolation constraints. Production and normal development use the sidecar.
- [Licensing](licensing.md): source/relink obligations and dependency provenance;
  local artifact verification does not establish release compliance.

Decisions, research and QA outcomes belong in the existing
[Plane documentation](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/0bfe867d-215a-4cfb-a501-9f628976bc9d).
Completed toolkit/render investigations and their captures are retained at the
[reviewed revision](https://github.com/andrejvysny/OneCAD/tree/ed9ed33f6f394897612e0a40edee015edd775ef8/spikes),
with conclusions preserved in the existing Decisions and risks Page. They are
historical evidence and do not qualify current native behavior.

Live visual inputs remain in `assets/native/tokens.css` and
`assets/native/cad-icons.json`; the latter is baked by `cargo xtask bake-cad-icons`.
Current renderer fixtures and goldens remain under `fixtures/render`.
