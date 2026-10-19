# Native sketch gesture contract

The native Select tool uses typed targets from the same accepted captured view as
selection. A pointer press arms a target without changing the document. Crossing
the native drag threshold starts one gesture. A click retains normal selection
semantics. Ambiguous or unsupported targets cannot silently become a curve drag.

## Targets

| Selected role | Gesture |
| --- | --- |
| Standalone point or line endpoint | Point |
| Circle or ellipse center | Point on the persisted center |
| Arc center | Whole-arc translation |
| Arc start or end | Arc endpoint reshape |
| Whole line | Line-body translation |
| Whole circle or arc | Radius |
| Whole ellipse | Unavailable; use its center |

Initial pointer grab coordinates preserve worker offsets. Derived arc endpoints
remain roles of the analytic arc; no endpoint entities are invented. Locked
owners and driven points are refused. An unlocked endpoint or radius may move
while another defining point remains locked; checked adoption protects the locked
geometry. Ellipse shape, radii and rotation are outside this gesture contract.

## Authority and bounds

`onecad-app` owns every accepted mutation. The native API admits a caller-supplied
document, runtime, edit visit, revision, snapshot, worker epoch and history-prefix
scope under the runtime lock. Worker requests execute outside that lock. The
driver also captures accepted sketch geometry and the concrete worker connection.
Each phase checks runtime authority and the pinned worker head before publishing
a receipt. The unpublished snapshot values `None` and `0` share the existing
preview-context meaning; positive snapshot identities must match exactly.

One application slot owns the driver and solver-lane claim. It retains one
in-flight update, one replaceable latest target and one terminal intent. Gesture
identifiers are process-unique checked integers. Admission returns a handle before
worker Begin finishes, so cancellation and final targets can arrive during Begin.
Dropped caller futures signal cancellation rather than dropping remote ownership.

The pure interaction controller retains one accepted baseline and one checked
transient preview. It queues exact pointer-up coordinates behind pending Begin or
Update, then flushes End. Preview point and curve channels accumulate the worker's
incremental deltas. They never replace accepted document geometry or invent solver
DOF. Escape, focus/pointer loss, tool changes, document changes and capture changes
cancel the owning gesture. Stale receipts cannot recreate a cancelled preview.

End adopts checked analytic geometry against the Begin baseline. Equal before and
after sketches produce no transaction, revision, dirty/autosave change or
regeneration.

An End whose final intent reproduces the accepted pose is a proven no-op: it
discards worker scratch, adopts no Upsert/Begin/End re-solve correction and
returns `moved = false`. Intent is per kind against accepted document geometry:
Point, Radius and EntityBody keep their Begin grab offset, so a final target equal
to `grab` is unchanged; without `grab`, Point/EntityBody compare the anchor point
and Radius compares the exact target distance with the radius. ArcEnd ignores
`grab` and is unchanged only when the target equals the accepted derived endpoint;
an off-handle release therefore still moves the endpoint. An End without a target
is unchanged only when no Update ran. Returning to the initial intent after moving
away is also unchanged, so solver branch history cannot become a document edit.
Every other target, including sub-micron movement, takes the normal solve path.
Begin returns its re-solve correction for every kind, keeping End deltas and
the worker store on one pose. A changed accepted pose creates exactly one sketch drag edit.
Native local Undo/Redo uses the existing edit-visit history contract.

## Cleanup and failure

The driver retains its lane through bounded cleanup. Cancellation addresses only
the captured worker connection. A stale End may already have changed the worker's
sketch cache; cleanup reseeds from current application authority, never the older
gesture snapshot. Cleanup does not adopt solver corrections into the document.
Uncertain cleanup retires the captured worker or retains one quarantined admission
slot with an explicit error. It cannot claim clean release or spawn retry loops.

A known accepted End receipt survives later cleanup failure. `cleanup_error`
reports that failure separately from `moved`. The native host refreshes authoritative
document state and retains the accepted marker even if final scene capture fails.
Its single bounded completion observer cannot consume another command's terminal
event or the actor's shutdown notification.

If accepted End cannot obtain a current scene, the host clears stale picking and
blocks sketch authoring until refresh succeeds. Save and Save As remain available
for the accepted sketch recovery document. Saving an active edit visit does not
Finish the sketch or regenerate its downstream feature payload; the saved sketch
must still be finished after recovery. New and Import retain their normal active
tool guards. A secondary Cancel acknowledgement cannot release the UI owner while
an accepted End and its capture outcome are still pending.

The service actor requests repaint after gesture submission and delivery. An
early pointer release therefore completes even when no further pointer event
arrives; continuous viewport repaint is not required for progress.

Legacy application gesture APIs remain available to existing backend callers.
They still hold the runtime lock across worker requests; the native host uses the
new detached APIs. This work does not change ONECAD-238 solver branch/stall math.

## Verification

Pure transitions live in `crates/onecad-interaction/tests/sketch_gestures.rs`.
Application driver tests live under
`crates/onecad-app/src/document_runtime/native_gestures/tests/`; real worker and
connection-lease cases exercise the production sidecar. Native raw-input cases
live in `crates/onecad-ui/src/gesture_delivery_tests.rs` and the prepared-arc suite.
The required macOS host gate explicitly inventories and executes ignored tests
with the production worker and Metal. Missing prerequisites fail.

These layers remain distinct from physical macOS input and the five immutable
ONECAD-273 document checkpoints. Synthetic input, service persistence and Metal
success do not qualify physical save, clean Quit or fresh-process native reopen.
