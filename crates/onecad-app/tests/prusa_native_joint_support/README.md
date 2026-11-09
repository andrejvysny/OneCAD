# Native Prusa revolute diagnostic

This test builds the existing source-derived 32-record cover and 8-record lever
through production services, then embeds those histories with the pinned imported
box. The fixture has three occurrences: a grounded box, a rigid cover, and a
revolute lever. The cover frame and occurrence placements are authored test
inputs. The original static placement aligned the unoriented cylinder lines but
left their directed axes antiparallel and their frame origins 6 mm apart. Before
the solve, the test uses `SetPose` to author a logical zero-angle lever rest
pose from the resolved box and lever frames; this is not a measured physical
placement. The hinge origins use occurrence-scoped element IDs resolved from the
box and native lever cylinders; the box reference must carry the pinned STEP
face 3042 certificate. The native lever is not represented as an import.

The existing source observations check the pivot axis lines, directions and
radii against box face 3042 (1.4 mm) and lever face 515 (1.65 mm). They use the
same OCCT kernel family and do not establish independent measurement or physical
assembly fit. The angular limits ±π/6 are authored solver inputs, not measured
physical travel. The workflow does not model contact, interference or dynamics.

The real-worker test checks one free degree of freedom, a driven angle and an
endpoint, invalid limits and out-of-range refusal, Undo/Redo, explicit save,
and a fresh OS process with a fresh worker that reopens and drives again. It is
service-level evidence, not a native UI or physical-input qualification. Full
ONECAD-533 acceptance remains open.

The separate `native_prusa_hinge_face_deletion_preserves_repair_evidence` case
removes the native lever's hinge Revolve cut while its body survives. A complete
worker `NeedsRepair` answer retains the old face as nonbinding evidence; the
imported box remains an independent definition. The accepted occurrence poses
stay fixed until explicit repair. An unrepaired fresh process refuses driving,
selects a supported current face through the checked mesh and exact worker datum
API, then repairs the binding. That selected replacement is an authored test
choice, not a claim that the deleted physical hinge has been reconstructed.
Repair Undo/Redo restores exact bindings and poses. The free joint retains one
DOF and valid limits; a separate zero-angle drive pins that DOF. A second fresh
process reopens the repaired file, drives again and saves. Both processes retain
the old reserved entry exactly. This does not exercise viewport input or prove
physical mechanism behavior.
