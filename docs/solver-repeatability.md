# Sketch solver repeatability evidence

ONECAD-305 investigates an open quarter Arc whose center can move during a radius
edit. Constraint correctness, repeatability of identical inputs on one build,
cross-host floating-point tolerance, and desired movement of an underconstrained
Arc are separate questions. A different valid pose does not establish a violated
constraint.

The bounded real-worker investigation is explicit and ignored by the ordinary
workspace suite. Use the pinned toolchain and a matching production OCCT 8.0.1
worker. The output directory must not exist:

```sh
ONECAD_WORKER_PATH="$PWD/worker/build-sketch-native/onecad-worker" \
ONECAD_SOLVER_REPLAY_OUT=/private/tmp/onecad-305-replay \
cargo test -p onecad-app --test solver_repeatability -- --ignored --nocapture
```

The free fixture retains the reported geometry: center (0, 0), radius 10 mm,
start angle 0, end angle π/2, followed by a persistent Radius constraint at 10 mm
and an edit to 14 mm. It retains four free DOF after the edit. The separate control
fixes the center and constrains the endpoint directions with construction lines;
it has zero DOF at either radius. Both use typed core entities and the production
application wire adapter, including the shared center point and `centerRef`.

The first accepted solve is captured before constructing the radius-edit request.
The resulting three OCW1 request frames are frozen, hashed, and replayed byte for
byte twelve times in one process and twelve times in fresh worker processes for
each fixture. Request IDs are intentionally repeated with the complete sequence.
Each sequence resets the authored sketch before applying the two dimensional
states; it does not reuse a previous edit's solved coordinates.

Evidence includes the authored typed sketch, accepted pre-edit wire geometry,
exact binary request frames, request and concatenated sequence SHA-256 hashes,
all raw response frames as hex, stderr logs, solver residuals, DOF, branch and
accepted geometry. Frame evidence is flushed incrementally, including before a
failed assertion. The report records source revision and dirty state, tracked
diff hash, harness source hashes, worker SHA-256, worker hello including OCCT
fingerprint, toolchain, platform, CMake cache and compilation database hash.

Same-build comparisons require exact equality of geometry, residuals, DOF and
branch. Transport sequence counters and sketch revisions are retained in raw
evidence but excluded from the solved-state comparison. The test reports the
exact value-set counts and numeric ranges separately for geometry, branch,
residuals and DOF, so floating-point noise remains distinct from a changed pose.
The test also checks the
published constraint budget and radius without fixing a preferred free-Arc
center or endpoint angles. Cross-host comparisons require review of the raw
numeric values and an explicit tolerance policy; this harness does not establish
that policy or authorize changing application movement behavior.

## Parameter-order regression

Solver policy 4 preserves semantic declaration order when selecting subsystem
columns and combining dual-priority parameter lists. Reduced aliases initialize
each solver value from its canonical representative once; pointer-map iteration
must not select the initial coordinate when coincident aliases differ by
roundoff. The vendored PlaneGCS adjustments are recorded in its `VENDOR.txt`.
DogLeg, its linear solver and convergence budgets are unchanged.

The worker regression permutes scalar storage while retaining declared semantic
parameter and equation order, including the reported free Arc, a constrained
control and unequal reduced aliases. The original free case remains free; the
fix does not establish a minimum-displacement radius-edit policy. Run the strict
OCW1 replay above against a freshly built policy-4 worker as well as CTest.
Do not replace exact same-build assertions with cross-host tolerances.

Host and worker must agree on policy 4. Their compatibility check remains active,
and policy-3 checkpoint/container caches require regeneration. OCCT 8.0.1 and its
fingerprint are unchanged; the solver-policy axis carries this change.
