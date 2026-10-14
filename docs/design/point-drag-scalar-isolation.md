# Point drag structural isolation

ONECAD-15 changes only the Point drive path. Group, ArcEnd, Radius and EntityBody
retain their existing target and pin contracts. The protocol is normative in
`protocol/SCHEMA.md` §7.4; this note explains the local PlaneGCS patch.

## Structural graph and numerical scope

`GCS::System::connectedUnknowns` walks the existing parameter/constraint graph
from the selected point's X and Y, traversing only registered unknown scalar
parameters. Constants cannot join components: two independent points constrained
to the same numerical dimension remain independent. The result preserves the
full unknown declaration order.

Point-only scoped declaration filters rows that touch no declared unknown from
solve subsystem construction, the diagnosis Jacobian, temporary-constraint
selection, redundant-error checking, and final redundant-tag subtraction. The
last filter matters for a constraint whose X and Y rows split into components:
an inactive sibling must not erase an active row's redundancy tag. Default full
mode includes every row as before. Solver algorithms, convergence thresholds,
constraint equations and non-point driving methods are unchanged.

The Point RAII scope removes tag −1 drives, restores full declaration mode and
unknowns, invalidates scoped diagnosis, and builds a fresh retained-pose reference
on every exit. Diagnosis cannot rewind entity scalars; complete scalar values are
preserved around that refresh. Public preview DOF/entity states remain the Begin
snapshot; End independently classifies released full exact truth.

## Acceptance and rollback

A Point request captures all point coordinates, circle radii and arc radii/angles.
Refusal restores raw scalars rather than setters that normalize angles. A refusal
therefore preserves even a disconnected arc's noncanonical angle representation.
Accepted Point candidates pass finite/nonnegative domain checks before exact
Fixed point-target normalization and exact restoration of captured immutable
reference scalars, then immutable reference, Fixed-target and arc
coupling checks. Affected arcs also retain the existing CCW sweep floor. No new
per-frame authored residual threshold is introduced.

Affected Angle constraints retain the original ordered-vector branch through the
entire linear candidate path, not just at its endpoints. The signed cross product
is quadratic in interpolation time; evaluate both endpoints and any interior
minimum. The local span uses the maximum of participating vector lengths at both
poses, which bounds intermediate norms by convexity. Undefined arithmetic refuses
rather than certifying a branch. Bounded retries restore the complete pre-request
pose when exhausted and carry `branchBoundary` through worker, Rust DTO and
frontend diagnostics. An unrelated Angle component does not restrict the selected
point.

## Known limitation: stalled drives

PlaneGCS can report Success for a drive that stalled at a stationary point:
the SQP line search collapses to a zero step (`GCS.cpp` ~4677–4721) while
primary error stays small. The chord guard then certifies the stalled pose.
Two cases were measured on 2026-10-14. A free operand dragged straight through
its fixed partner has ASLR-dependent outcomes, because pointer-ordered PlaneGCS
columns perturb rounding. A length-locked point targeted at the far side stays
at the antipode. Canonical fixtures therefore pin only non-singular branch cases.
A drive-progress acceptance rule needs an Astra derivation; lever-limited
stepping was tried and reverted (regressions, per-frame cost).

## Preview and lifecycle

Begin solves scratch and returns its optional corrections and Point handle/grab
offset. Rust retains the corrected baseline for subsequent deltas while the
original document memento remains unchanged. The frontend snaps the desired handle
before converting back to cursor coordinates; pending pointer-up waits for Begin.

Cancellation is `EndGesture {cancel:true}` and disposes scratch before any solve
or store update. Rust emits no mutation/history/projection event. Complete frontend
session truth and geometry are restored with `solveCurrent:false`. Begin, preview,
End and cancellation are fenced by worker connection/epoch, document geometry,
active gesture and sequence. The frontend drains Begin/finalization before arming
a replacement gesture; Esc takes precedence over a queued pointer-up.
