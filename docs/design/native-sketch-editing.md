# Native sketch and extrusion editing contract

The native host presents application-owned sketch visits and modeling sessions.
`onecad-app` remains the only document mutation authority. Transient editors,
pointer ownership and label placement do not belong in the saved document.

## Selection and visit lifecycle

Selection takes coordinates and modifiers from the owning primary-button press.
Marquee selection and the overlap chooser retain that mode through release.
Ambiguous batches containing multiple primary-button cycles are refused.
Single-click sketch and history rows select; double-click and explicit Edit enter
the same edit transition, including from an initially unselected row.

Text widgets own text Undo and Redo even when their history is empty. IME events,
retained input and clicks that dismiss an editor must not reach CAD commands.
Escape unwinds the current numeric editor, gesture or primitive, active tool and
selection in that order. Idle Escape stays in the sketch. **Finish sketch** and
**Discard sketch edits** use the application's visit history. A whole-visit discard
refused after history eviction remains visibly refused; it is not a successful
rollback.

## Dimension editors and primitive authorship

Creation measurements appear as unfocused compact geometry-adjacent chips.
Click or Tab engages numeric editing; Tab and Shift+Tab cycle fields. Existing
dimension edits use the same placement primitive while retaining their original
constraint identity. Chips clamp outside the Explorer, Inspector and session
controls; short witness lines preserve association during view changes.

An existing dimension editor uses the original constraint identity and application
candidate admission. Its text identity is the sketch visit plus constraint, not
the current camera capture. Placement remains presentation state and clamps to
the viewport after resize. Selected operands and active editors expose their
dimensions; the Inspector and explicit Show dimensions remain fallback access.

Stored expressions remain expressions. Replacing one with a literal removes its
binding. Coordinate differences allow their supported zero and signed values;
angle text converts degrees to radians once; radii and diameters stay positive.
Confirming an unchanged value does not author a history entry or dirty the file.

Creation fields distinguish measured values from deliberately authored inputs.
Focus and Tab alone do not author a field. Pointer movement can update untouched
fields only when the canvas owns that input. Confirmation submits one primitive
batch and only the driving dimensions explicitly authored by the user. For
example, entering only a rectangle width does not add a height dimension.
Arc sweep determines initial geometry without becoming a persistent constraint.
Structural anchors, inference, construction state and solved continuation points
remain part of the existing primitive creation path.

Enter confirms once; blur never confirms. Pending and refused editors retain text
and local errors. Their submission retains one request correlation through its
terminal reply. Cancellation and application admission share an atomic decision:
cancellation before admission prevents mutation, while admission that has already
won may complete. Executing worker work retains its lane ownership until safe
completion. Closing an editor cannot make a late reply reopen it.

## Support planes and identity

Sketch starts a nonmodal viewport support chooser. Labelled translucent principal
patches and directly picked planar faces compete by captured ray depth. Hover
resolution is transient and bounded; only the current candidate and capture may
publish a frame. Navigation remains available, and acceptance aligns the camera
once through the existing entry path. Curved or stale faces never fall back to XY.

The analytic grid uses the hovered or active effective support frame. World-ground
contact shadows remain on world XY. Displayed user X is geometric +Y, user Y is
geometric -X, and user Z is geometric +Z. Saved plane bases remain unchanged;
arbitrary supports show local axes. Grid visibility is presentation state.

World-plane entry preserves the persisted XY, XZ and YZ conventions. Face entry
uses checked topology evidence, the worker's exact planar frame and locked
projected boundaries. It must not substitute a world plane for unresolved support.
Preparation captures the runtime, revision, snapshot, worker epoch and provider;
worker queries run unlocked; commit revalidates the captured scope.

Identity promotion and sketch insertion are staged together. Refusal or
cancellation before admission leaves no sketch, persistent binding, history entry
or dirty-state change. Rendering, picking, camera alignment and downstream
extrusion consume the same effective frame. Support identity remains persistent
through regeneration, history and reopen. General datum authoring and support
reattachment are outside this entry contract.

## Extrusion drafts

New extrusion defaults to New Body. Add and Cut require an explicit displayed,
valid target body. Choosing a target retains the exact selected sketch region,
including its internal loops. Every mode or target change replaces preview
correlation and regenerates the candidate; old evaluation or preview replies
cannot restore an earlier draft.

Distance has one signed expression draft. Flip negates that draft through the
existing expression resolver. Preview and commit consume the same validated
operation and record identity. Saved-feature editing retains replacement
semantics: appending a preview at the current head is not a valid replacement
preview.

## Qualification

Finished regions use one indexed fill mesh and separately stroked true contours,
including hole contours. Individual triangulation edges are never stroked.
`viewport::tests` reproduces the former acute-triangle stroke-join spikes and
checks bounded fill output. `sketch_finish_stability` reconstructs a polygon,
rectangle with inner circle and separate circle, recording final solver receipts
and checking geometry, regions, history and fresh-process persistence on principal
and translated oblique stored frames. This synthetic fixture is not the original
user document or proof of planar-face admission.

Pure transitions, real-worker geometry and persistence, Metal rendering,
automated native-host input and physical input are separate evidence classes.
Required worker and GPU checks fail when prerequisites are unavailable. Exact
ignored native-host inventories stay in `xtask/src/evidence/host_names.rs`.
Do not regenerate approved renderer goldens to conceal appearance changes.

The [native modeling acceptance procedure](../qa/native-modeling-acceptance.md)
preserves the 17-constraint, 4 mm From-New mounting plate gate. The separate
prepared-profile 6 mm, 8 mm, Undo 6 mm, Redo 8 mm and fresh-process reopen captures
remain a distinct gate. Synthetic documents and service assertions supplement
actual GUI saves; they cannot substitute for that evidence.
