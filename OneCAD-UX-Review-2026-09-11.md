# OneCAD native modeling UX review

Date: 2026-09-11  
Method: Manual interaction through Computer Use; native OneCAD `tauri://localhost` window.  
Application: `/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src-tauri/target/debug/bundle/macos/onecad.app`

## Outcome

Created a dimensioned plate, a counterbored hole, a three-body linear pattern, and an edge chamfer. Tested fillet dragging, numeric sketch dimensions, numeric feature editing, tool switching, selection, cancellation, and Undo discovery. A 2 mm shell preview appeared, but confirming it was followed by repeated native inspection timeouts. Its committed result is unverified.

**This is a partial modeling review, not a completed acceptance test.** Useful multi-feature geometry was produced, but the session could not finish the requested interaction matrix. In particular, actual grid-snapping placement, sketch dragging, and several additional operations remain untested. No successful result from an earlier session is counted here.

The app's basic feature creation is promising. The largest observed risks are unreliable recovery, hard-to-read or inaccessible floating controls, selection interference, and weak visibility of newly created operations in history. Some early input and viewport problems may belong to Computer Use rather than OneCAD; these are explicitly separated below.

No code, specifications, logs, or backend data were inspected. No fixes were made. Geometry correctness here means visible UI evidence, not a kernel audit or metrology check. The test document remains unsaved; no existing recovered project was restored or discarded.

## Test object and execution checklist

The exercise evolved into a three-plate fixture concept: a 100 × 70 mm plate, 12 mm extrusion, central counterbored through-hole, three plates spaced along Y, and a treated front edge. This exercises more dependencies and selection states than a single primitive, but does not constitute a finished production assembly.

- [x] Attach to native app; create fresh Untitled project.
- [x] Inspect snap settings and default choices.
- [x] Enter a sketch; construct a rectangle using numeric width and height.
- [x] Create extrusion; correct its depth through history.
- [x] Start a face sketch and enter a numeric circle diameter.
- [x] Attempt multi-region extrusion; cancel without committing.
- [x] Create a custom counterbored through-hole.
- [x] Create a three-body linear pattern using numeric spacing.
- [x] Attempt Undo via keyboard and native menu; search command palette.
- [x] Hide an interfering sketch; select a solid edge.
- [x] Drag fillet handle; test oversized numeric input; switch to chamfer and commit.
- [x] Preview shell; attempt commit and recovery.
- [ ] Verify shell committed geometry.
- [ ] Compare actual point placement with grid snapping on/off and Alt bypass.
- [ ] Test sketch geometry dragging and extrusion-depth dragging.
- [ ] Finish navigation, additional modeling operations, and redo validation.

### Model state before loss of interaction

| Component | Evidence | Limit |
|---|---|---|
| Sketch 1 | Finish status: four entities, DOF 2; Distance 100 mm and Distance 70 mm constraints | Position remained under-constrained; exact plane was not confidently established during early targeting problems |
| Base plate | Visible solid; history corrected to Extrude 12 mm | Initial typed “12” produced 2 mm before correction |
| Sketch 2 | Face sketch; Diameter 30 mm constraint; finish status five entities, DOF 4 | Not used to create a solid feature; additional entities were not individually inspected |
| Hole | Visible stepped opening after commit; status “Hole” | Ø8, through, Ø16 counterbore, depth 4 entered in UI; no independent measurement |
| Linear pattern | Three visible plates; status “Linear pattern ×3”; Y selected, spacing 35 mm | Replica hole interiors obscured in available perspective; not independently inspected |
| Chamfer | Visible bevel on front plate; status “Chamfered 1 edge”; 3 mm entered | Post-commit selection highlight became unreliable |
| Shell | Visible cavity preview at 2 mm | Confirm followed by tool failures; commit outcome unknown |

## Detailed test record

| Test | Interaction and result | Assessment |
|---|---|---|
| Startup/new project | Start page exposed Recent, project search, New project, and an existing unsaved-recovery prompt. New project opened empty Untitled with separate Bodies, Sketches, and Datums sections. | Pass. Clear initial structure. Recovery existence observed; recovery itself not tested. |
| Snap settings discovery | Opened Snap settings. Grid, guide lines/points, quadrants, intersections, on-curve points, dimension rounding, and polar tracking were on. Radius M, auto-constrain Standard, input device Auto. Help explained Alt free placement. | Discoverability pass. Behavior not validated; reading an enabled switch does not prove snapping works. |
| Plane selection/cancel | New sketch entered a plane-selection state. Cancel and Escape returned to selection. Datum tool exposed visible planes and a 10 mm offset chip, then Escape cancelled it. | Cancellation pass. Initial plane choice was difficult under intermittent input/capture failures. |
| Rectangle numeric input | Placed first point through accessible viewport target, typed 100, Tab, 70, Enter. UI reported DOF 2 and 100/70 mm distance constraints; finishing reported four entities. | Pass for width/height entry and feedback. Sketch position not fully constrained. |
| Extrude numeric input | Started extrusion. With its chip visible, sent Select All, typed 12, Enter. Result reported Extrude 2 mm. | Unexpected result. Focus/first-keystroke behavior needs reproduction; do not assume every multi-digit input is broken. |
| History correction | Selected Extrude history entry, clicked its 2 mm value, typed 12, Enter. History showed 12.0 mm / Extrude 12 mm; solid visibly thickened; status “Feature updated.” | Pass. Effective direct-edit recovery when field is explicitly focused. |
| Face sketch/circle | Selected broad plate face, started new sketch, chose Circle, placed center through accessible viewport, typed 30, Enter. Diameter 30 mm constraint appeared. Finish reported five entities and DOF 4. | Numeric circle entry passed at UI level. Full sketch geometry was not reliably visible during this phase. |
| Region extrusion | Extrude on Sketch 2 presented “0 regions” and requested region selection. No confidently visible region target was available in captured state; cancelled. | Incomplete, not a proven extrusion failure. |
| Counterbore | Hole requested flat face. Placed on plate, navigated controls with Tab, switched Simple to CBore with Space. Set diameter 8, retained Thru, set counterbore diameter 16 and depth 4. Clicked checkmark; stepped opening appeared. | Pass. Keyboard navigation provided a useful fallback. |
| Standard-size picker | Opened Std dropdown. It showed Thread, Close/Normal columns and M3–M12 rows; cells appeared as dashes in capture. No preset chosen. | Presentation concern; preset functionality not tested. |
| Linear pattern | Selected body; activated pattern; switched X to Y; explicitly focused spacing and entered 35. Preview explained three total / two new bodies / source retained. Confirm produced three plates. | Pass. Count semantics are particularly useful. |
| Undo | Sent Command-Z after pattern; three plates remained. Invoked native Edit → Undo; three plates remained. Command palette search for “undo” returned no match. | No successful modeling Undo observed. Serious recovery concern; native text Undo and document Undo may differ. |
| Edge selection | Clicking apparent plate edge selected Sketch 2 instead. Hid Sketch 2 through its eye control, retried, and obtained Body 1 Edge; Fillet/Chamfer enabled. | Workaround passed. Visible sketches compete with solid selection. |
| Fillet drag | Started default 2 mm fillet. Dragged visible handle about 20 px up-left. Chip changed to approximately 6.871 mm; rounded preview appeared. | Pass for this drag and preview. Repeatability, snapping, and lower-bound dragging not tested. |
| Oversized radius | Explicitly focused fillet field, typed 100, then Tab. Display became approximately 11.99 mm with a large preview and no visible explanation. | Silent adjustment concern. Exact limit and underlying validation not inspected. |
| Chamfer switch/commit | Opened Fillet options; selected Chamfer; entered 3 mm in main field; confirmed. Bevel appeared and status reported one edge chamfered. | Pass. Switching preserved selection; secondary distance/angle modes not exercised. |
| Selection after commit | Blue selection line appeared displaced across plate face. Clearing and selecting face yielded “Selection is out of date — pick again.” Re-picking allowed Shell to start. | Observable selection recovery defect/friction; no topology cause inferred. |
| Shell | Chose Shell on front face, previewed 2 mm wall, explicitly entered 2 and confirmed. Confirm returned noWindowsAvailable after about 14 seconds; subsequent inspection and reconnect timed out. Escape plus inspection also timed out. | Blocked. Preview visible; committed result unverified. |

## What works well

1. **Direct numeric sketch construction.** Width → Tab → height → Enter is compact and useful. Constraints and DOF communicate the difference between geometry size and remaining positional freedom.
2. **Direct history editing.** Clicking an extrusion's value made correction straightforward. “Feature updated” and visible thickness change provided good confirmation.
3. **Context-sensitive modeling tools.** Shell and Offset face became available on face selection; Fillet/Chamfer became available on edge selection. Bodies, sketches, and datums are clearly separated.
4. **Responsive previews once interaction stabilized.** Hole sizing, pattern direction/spacing, fillet dragging, chamfer switching, and shell preview produced visible changes.
5. **Useful pattern semantics.** “3 total · 2 new bodies · source retained” removes a common count ambiguity.
6. **Keyboard alternatives.** Tab/Space exposed and operated hole controls even when coordinate clicks failed. Escape successfully cancelled earlier sketch, datum, and region-selection modes.
7. **Compact commit controls.** Repeated checkmark/cancel patterns are consistent, and status instructions explain Enter/Escape for many tools.

## Prioritized UX findings

Severity indicates observed workflow impact, not a confirmed implementation cause.

### High — modeling recovery not demonstrated

**Reproduce:** Create the three-body pattern, use Command-Z, then native Edit → Undo. Search “undo” in command palette.

**Observed:** Pattern remained after both Undo attempts; command search returned no result. There was no visible message explaining an unavailable modeling Undo.

**Impact:** Users cannot confidently explore variations or recover from mistakes. Native menu availability suggests recovery should work.

**Recommendation:** Expose document Undo/Redo with operation names and enablement tied to modeling history; clearly distinguish text-field Undo. Revalidate using pattern removal and restoration with explicit body-count checks.

### High — shell commit ended usable testing

**Reproduce context:** A 12 mm plate with counterbored hole, subsequent linear pattern, and 3 mm chamfer; select front face, Shell 2 mm, confirm.

**Observed:** Preview appeared. Confirm was followed by repeated Computer Use failures. Inventory continued to list OneCAD as running. Inspection, reconnect, and Escape did not restore observable control.

**Impact:** No way to confirm commit, cancel, or continue this session.

**Confidence:** High that native testing became blocked; insufficient evidence to call this a OneCAD crash or kernel hang. Tooling already exhibited intermittent failures earlier.

**Recommendation:** Reproduce with a person watching the window. Validate progress feedback, cancellation, completion/failure notification, and responsiveness throughout the operation before assigning the owning component.

### High — stale or displaced post-feature selection

**Observed:** Chamfer committed visibly, but blue edge highlight lay across the plate face rather than on the newly chamfered boundary. Next face interaction reported “Selection is out of date — pick again.”

**Impact:** Selection cannot be trusted when chaining operations. Re-picking adds friction and can target the wrong feature.

**Recommendation:** Clear or resolve selection after geometry changes before displaying actionable tool state. Preserve an accurate highlight or provide an explicit selection-reset message.

### Medium — numeric entry can commit an unexpected value

**Observed:** Initial extrusion received Select All → “12” → Enter but history read 2 mm. Explicitly clicking the history value, typing 12, and committing worked.

**Impact:** A visually subtle thickness error can survive into downstream geometry.

**Confidence:** One occurrence, with input/capture instability nearby. Could be focus or input routing; not established as a general parser defect.

**Recommendation:** Make active field focus unmistakable; test first typed character, multi-digit values, paste, and Enter. Verify intended value before commit and echo committed value in feedback.

### Medium — oversized fillet silently changed

**Observed:** Entering 100 mm displayed approximately 11.99 mm after leaving the field, without an explanation in visible status.

**Impact:** A limit may protect geometry, but silent substitution violates dimensional intent.

**Recommendation:** Preserve rejected entry with a clear validation message, or explicitly disclose the applied limit and require an intentional acceptance of the adjusted value.

### Medium — floating controls obscure geometry and have weak labels

**Observed:** Hole chip crossed the opening being inspected. Several numeric controls had generic or implementation-like accessibility names: “Dimension value,” “chip-hole-cb-diameter,” and “chip-hole-cb-depth.” Many chip controls only appeared in accessibility output after keyboard focus. Chamfer auxiliary fields were visually tiny and icon-led.

**Impact:** Harder visual verification, slower discovery, and poor assistive/tool access. Users must remember field meaning and units.

**Recommendation:** Provide stable semantic labels, concise unit labels, and a docked inspector alternative. Allow the floating chip to move away from the selected feature.

### Medium — sketches intercept solid selection

**Observed:** Picking an apparent plate edge selected the overlaid face sketch. Hiding Sketch 2 allowed the intended solid edge to be selected.

**Impact:** An inactive sketch obstructs downstream edge operations with little explanation.

**Recommendation:** Offer selection filters, cycling through overlapping candidates, or a visible candidate list. Consider hiding consumed/finished sketches where appropriate while retaining explicit visibility control.

### Medium — operation history did not reflect all visible modeling work

**Observed:** After hole and pattern, the visible inspector history continued to show Sketch, Extrude 12 mm, Sketch. After chamfer, selected-edge/face history showed Extrude 12 mm. No editable Hole, Linear pattern, or Chamfer row was visible in inspected states.

**Impact:** Successful direct edits appear difficult to find and revise. A user cannot tell whether they are parametric, direct, or omitted by the current selection filter.

**Confidence:** Applies to inspected UI states only; no claim that the operations are absent from internal history.

**Recommendation:** Show all relevant editable operations or explicitly explain filtered/non-history operations and how to revise them.

### Low — constraint messaging is not always context-specific

**Observed:** Newly entered Sketch 2 displayed “Fully constrained · DOF 0” and “Sketch is fully defined,” while its constraint panel had no constraints and generic instructions to drag/add constraints. Later it reported five entities and DOF 4.

**Impact:** Empty, inherited/projected, and actively constrained geometry states are hard to distinguish.

**Recommendation:** Use explicit messages for empty geometry, projected references, and user geometry; describe remaining freedom rather than repeating generic instructions.

### Low — default snap policy is powerful but dense

**Observed:** Many snap categories were enabled together; Standard auto-constrain and radius M were selected. Alt free-placement help was useful.

**Impact:** Beginners may struggle to identify which system moved a point or created a relationship.

**Recommendation:** During placement, identify the active snap type and relationship being created. Test the defaults with real placements before changing them; this session did not validate their behavior.

## Computer Use limitations and attribution

Early coordinate clicks repeatedly returned `Computer Use server error -10005: noWindowsAvailable`, although accessibility controls remained readable and operational. Reattaching by bundle ID, raising/resizing the window, and resetting the Computer Use session did not consistently resolve it. Accessible viewport clicks and keyboard inputs enabled progress. Later coordinate clicks and fillet dragging succeeded.

One face-selection action was followed by `com.apple.ScreenCaptureKit.SCStreamErrorDomain Code=-3812`, “Failed due to an invalid parameter”; a subsequent observation showed that face selection had succeeded. Therefore, a failed capture does not necessarily mean the preceding action failed.

During early sketching, captures showed changing toolbars/constraint state without corresponding visible sketch geometry or camera changes. Geometry later appeared. Treat this as an observation limitation requiring direct visual reproduction, not proof of a viewport rendering defect.

Following Shell confirmation, failures became sustained: approximately 14 seconds for the initial noWindowsAvailable result, approximately 17 seconds for subsequent timeout, approximately 17 seconds on reconnect, and approximately 18 seconds for Escape plus inspection. No force-quit or destructive recovery was attempted. Shell completion remains unknown.

## Remaining manual validation

Resume only after the native window responds and the shell outcome is established.

| Area | Required follow-up |
|---|---|
| Grid snapping | In an unobstructed orthographic sketch, place measured points with Grid on/off; compare coordinate multiples; verify visible grid spacing versus snap spacing; test Alt bypass and restore original choices. |
| Sketch manipulation | Drag under-constrained rectangle corners and circle center; confirm fixed dimensions persist, DOF changes are correct, and constraints resist prohibited movement. |
| Numeric entry | First-keystroke focus, decimals, negatives, zero, expressions if advertised, paste, unit suffixes, Tab/Shift-Tab, Enter, and invalid-value feedback. |
| Dragging | Extrude arrow in both directions, reverse after typed value, fillet lower bound, snap transitions, preview latency, release-to-commit semantics. |
| Region operations | Pick circular versus surrounding profile; verify Join/Cut/New behavior and preview/commit agreement. |
| Feature breadth | Revolve, slot, trim, sketch offset, mirror, face offset, combine, and both chamfer auxiliary modes. |
| Navigation/inspection | Deliberate orbit/pan/zoom, all standard views, orthographic mode, measure, isolate, section, display modes; check inputs do not mutate geometry. |
| Recovery/history | Undo and Redo each feature; inspect history editing, dependent geometry updates, cancellation and error recovery. |
| Persistence | Save/reopen only when requested; confirm the multi-feature geometry and dimensions survive. Not exercised here. |

## Review verdict

**Basic construction and several advanced operations worked through the native GUI. Precision confidence and recovery are not yet validated.** The most valuable next test is a controlled repeat of numeric focus, Undo, post-chamfer selection, and shell commit, with reliable direct observation. Grid snapping and the remaining drag interactions must still be exercised before claiming thorough modeling UX acceptance.

---

## Addendum A — second-pass attempt and professional CAD comparison

Date: 2026-09-11. This addendum extends the original report; all earlier observations and qualifications remain intact.

### Second-pass execution status

**Blocked before a new part could be created.** Attaching to the same native app bundle timed out after approximately 22 seconds. App inventory still listed OneCAD as running. A second attachment through bundle ID `com.andrejvysny.onecad` timed out after approximately 21 seconds. A subsequent cancellation attempt was rejected by the tool because no active app state had been acquired; it is not evidence that Escape reached OneCAD.

No new modeling, snapping, drag, navigation, or hover result was obtained in this pass. The first-pass shell outcome remains unknown. No force-quit, restart, or discard of unsaved work was performed. The persistent attachment failure is a testing blocker, not proof that OneCAD itself crashed.

The intended second part was a **flanged cylindrical housing**, to exercise a different modeling strategy from the patterned plates. The executable follow-up scenario appears below.

**Hover limitation:** The available Computer Use API documents clicks, dragging, scrolling, and keyboard actions, but no standalone pointer move/hover or modifier-held mouse drag. Earlier tooltip appearances after clicks do not constitute controlled hover tests. Do not simulate hover by clicking or by zero-length dragging and report it as equivalent. Pure hover, dwell timing, and common modifier-based orbit gestures need supported input tooling or a human-assisted pass.

### Comparison method and limits

OneCAD evidence comes from the first manual pass and this failed reattachment attempt. Shapr3D and Autodesk Fusion (formerly Fusion 360) evidence comes from official product documentation consulted for this addendum; neither reference application was operated in this session. The comparison concerns documented interaction patterns, not measured speed, stability, kernel quality, or overall feature parity.

“Not validated in OneCAD” does not mean “not implemented.” In particular, advanced extrusion extents, navigation presets, selection filters, and hover highlighting were not exhaustively explored. Recommendations below are reviewer judgments, with evidence level identified.

### Comparison by modeling task

| Task | OneCAD evidence | Professional reference workflow | Improvement for OneCAD |
|---|---|---|---|
| Discover tools from selected geometry | Face selection enabled Shell/Offset face; edge selection enabled Fillet/Chamfer. This worked. | Shapr3D adapts available tools to selected geometry and highlights related sketch constraints. [Selecting geometry](https://support.shapr3d.com/hc/en-us/articles/7770768736924-Selecting-geometry) | Keep contextual tools. Add a short reason for unavailable actions, such as “Select one or more solid edges,” without implying that disabled tools are broken. |
| Select overlapping sketch/solid geometry | An inactive visible sketch intercepted the intended solid edge. Hiding it resolved the selection. | Shapr3D documents an overlapping-item picker. Fusion exposes object-type filters and face/edge/component priorities. [Shapr3D selection](https://support.shapr3d.com/hc/en-us/articles/7770768736924-Selecting-geometry), [Fusion selection](https://help.autodesk.com/view/fusion360/ENU/?guid=SLD-SELECTION) | Offer a candidate list with type/name and preview highlight; make edge-only selection easy to discover. Preserve visibility while resolving ambiguity. |
| Hover to understand an unfamiliar model | No controlled hover test completed. Tooltips appeared during some click sequences. | Shapr3D documents Items Manager hover highlighting and a Reveal in Items command linking geometry to its list entry. [Navigation and item updates](https://support.shapr3d.com/hc/en-us/articles/20524622488476-5-880-Navigate-faster-manage-items-better-track-edits-clearly) | Test bidirectional tree/canvas highlighting without changing selection. Add Reveal in Model and tooltip text containing tool name, shortcut, prerequisites, and a brief purpose. This is a proposed acceptance target, not a confirmed missing feature. |
| Drag then specify an exact dimension | Fillet drag changed preview from 2 to about 6.871 mm. Explicitly focused numeric fields worked. Initial extrusion input yielded an unexpected 2 rather than 12 mm. | Shapr3D documents both gizmo and numeric distance interaction for Offset Face, and gizmo-driven extrusion with editable history parameters. [Offset Face](https://support.shapr3d.com/hc/en-us/articles/7874400678428-Offset-Face), [Extrude](https://support.shapr3d.com/hc/en-us/articles/7874453786908-Extrude) | Make drag and numeric entry two consistent routes to one visible value. Preserve the first typed digit; keep units and active focus clear; allow refinement after dragging without an unexplained value reset. |
| Choose extrusion intent | Simple extrusion worked. Multi-region selection was entered but not completed. Additional extent modes were not inspected. | Shapr3D exposes a Boolean badge and history extents. Fusion explicitly documents start, direction, distance/to-object/all extents, taper, operation, and affected cut bodies. [Shapr3D Extrude](https://support.shapr3d.com/hc/en-us/articles/7874453786908-Extrude), [Fusion Extrude reference](https://help.autodesk.com/cloudhelp/ENU/Fusion-Model/files/SLD-REF-EXTRUDE.htm) | Show selected-profile count, direction, New/Join/Cut intent, and affected bodies beside the preview. Expose advanced options progressively in a stable inspector. Do not infer their absence from this incomplete test. |
| Understand snapping versus constraints | Many snap options were enabled; numeric rectangle constraints and DOF feedback worked. Actual point snapping remains untested. | Shapr3D provides pointer-adjacent text identifying snap targets. Fusion's Sketch Palette separates grid display, grid snap, profiles, dimensions, constraints, and projected geometry. [Shapr3D Snapping Options](https://support.shapr3d.com/hc/en-us/articles/7873946289564-Snapping-Options), [Fusion sketches](https://help.autodesk.com/cloudhelp/ENU/Fusion-Sketch/files/SKT-3D-SKETCH.htm) | Show what captured the pointer and which persistent relationship will be created. Keep grid visibility independent of snapping. Use distinct feedback for temporary alignment, committed constraint, and fixed dimension. |
| Explain remaining sketch freedom | Rectangle retained DOF 2; the inspector advised constraints. A new face sketch initially showed fully constrained messaging that did not explain its contents. | Fusion distinguishes constrained from movable geometry. Shapr3D connects selected sketch elements to their dimensions/constraints and describes constrained movement behavior. [Fusion sketches](https://help.autodesk.com/cloudhelp/ENU/Fusion-Sketch/files/SKT-3D-SKETCH.htm), [Shapr3D selection](https://support.shapr3d.com/hc/en-us/articles/7770768736924-Selecting-geometry) | Retain numeric DOF, but add actionable explanation where possible, for example “Size fixed; position still free.” Distinguish inherited/projected geometry from newly drawn entities. |
| Revise features after several operations | Extrusion history value editing worked. Hole, pattern, and chamfer were not visible as editable rows in inspected history states. | Shapr3D has expandable parameter cards and selection-filtered history. Fusion parametric mode records features in a timeline and exposes Edit Feature. Its direct mode has different editing limits. [Shapr3D History](https://support.shapr3d.com/hc/en-us/articles/11567903089180-History), [Fusion modeling modes](https://help.autodesk.com/view/fusion360/ENU/?contextId=DESIGN_HISTORY), [Fusion editing](https://www.autodesk.com/support/technical/article/caas/sfdcarticles/sfdcarticles/How-to-edit-existing-features-in-Fusion-360.html) | Label history filtering explicitly and provide Show All. Make every editable operation reachable from geometry and history. Explain direct operations that cannot be edited parametrically. |
| Undo a modeling experiment | Neither Command-Z nor native Edit → Undo visibly removed the pattern. Search returned no Undo command. | Shapr3D documents Undo/Redo in its modeling-space interface. Fusion documents timeline replay, parameter editing, and suppression as distinct history mechanisms. [Shapr3D modeling space](https://support.shapr3d.com/hc/en-us/articles/7873880676508-Shapr3D-modeling-space), [Fusion Timeline](https://help.autodesk.com/view/fusion360/ENU/?contextId=LP-STEPS-P13N-SNP-GS-OTH-CRD-2), [Fusion modeling modes](https://help.autodesk.com/view/fusion360/ENU/?contextId=DESIGN_HISTORY) | Prioritize reliable document Undo/Redo and operation-specific labels. Keep text editing, document undo, and history rollback distinct. Feature editing is not a substitute for Undo. |
| Navigate without altering geometry | View controls were visible, but intentional orbit/pan/zoom coverage was incomplete. Early captures did not consistently show camera changes. | Shapr3D documents input-device-specific navigation and customizable shortcuts. Fusion lists pan, wheel zoom, and modifier/middle-button orbit controls. [Shapr3D shortcuts](https://support.shapr3d.com/hc/en-us/articles/7873906073884-Keyboard-shortcuts-gestures-and-hotkeys), [Fusion shortcuts](https://help.autodesk.com/view/NINVFUS/ENU/?guid=GUID-F0491540-0324-470A-B651-2238D0EFAC30) | Provide an in-app navigation cheat sheet for the active device/preset. Validate that orbit, pan, zoom, and view changes never commit a sketch point or alter dimensions. Keep an obvious normal-to-sketch and fit-selection route. |
| Inspect internal geometry | Hole and shell previews were visible; shell commit could not be verified. | Shapr3D documents section fills, internal measurement, and editing within section view. [Section View](https://support.shapr3d.com/hc/en-us/articles/7873938030492-Section-View) | Validate section inspection of bore depth and wall thickness. Make preview versus committed geometry unmistakable. This capability remains untested in OneCAD. |

### Recommended product direction

**Preserve OneCAD's compact, geometry-centered interaction; strengthen precision, visibility, and recovery before adding more command breadth.** This is a design recommendation derived from the observed friction, not a claim that either reference application is defect-free.

Shapr3D provides useful references for selection-driven actions, spatial controls, and immediate geometric feedback. Fusion provides useful references for explicit feature parameters, selection filters, and revisiting design intent. OneCAD can combine these interaction principles without reproducing either interface wholesale.

A practical layout would retain a small near-selection chip for the main value and commit/cancel, with an optional fixed inspector for named secondary parameters, operation type, target bodies, and validation messages. Users should not need to infer counterbore diameter or depth from tiny symbols or implementation-like labels.

### Prioritized improvements and acceptance tests

The following are proposed requirements, not implemented fixes or newly observed results.

| Priority | Improvement | Concrete acceptance test |
|---|---|---|
| P1 | Reliable modeling recovery | Pattern one body into three, Undo to one, Redo to three; repeat with hole and chamfer. Menu, shortcut, and command search agree. Focus inside a number field only undoes that field until focus leaves. |
| P1 | Exact numeric intent | Enter 12, 12.5, and 0.5 by typing and pasting into a freshly opened tool; values match preview and committed history. For invalid values, show a reason; never silently substitute a new dimension. |
| P1 | Correct selection after geometry edits | After chamfer/fillet, hover and select the new boundary; highlight follows visible geometry. Immediately start the next valid operation without a stale-selection warning. |
| P1 | Observable long-operation lifecycle | Shell preview → Apply exposes working/completed/failed state. A cancellation request is acknowledged, even if safe cancellation is deferred. Failure leaves the last valid model inspectable. Assign implementation ownership only after reproducing outside the current tool failure. |
| P2 | Accessible full parameter inspector | Every field has a meaningful name and unit. Tab order follows visual order. Counterbore diameter/depth and chamfer legs/angle are readable without guessing. Chip placement does not obscure the selected feature. |
| P2 | Overlap-aware selection | At a coincident sketch/solid edge, choose either candidate without hiding geometry. Preview the candidate and identify whether it is a sketch curve, solid edge, or face. |
| P2 | Complete, understandable history | Find and edit the hole, pattern, and edge treatment after creating them. Show whether current history is filtered; Show All restores the full visible operation list. |
| P2 | Predictable snapping and drag refinement | Toggle snap while placing the same test points; validate coordinates and displayed snap type. Drag a dimensioned sketch without violating fixed dimensions. Drag a feature handle, type an exact value, then resume dragging with clear semantics. |
| P2 | Hover and navigation confidence | Hover a tool, list item, face, edge, and constraint without clicking: appropriate hint/highlight appears and model state stays unchanged. Orbit/pan/zoom during an active tool preserve its inputs and never add geometry. |
| P3 | Context-specific guidance | Empty sketch, projected-only sketch, under-constrained sketch, and fully defined sketch each show accurate guidance. Disabled tools state their selection prerequisites. |

### Prepared second-part scenario: flanged housing

**Not executed.** Use a new test document once native interaction is restored. Dimensions below are proposed targets, not measured results.

1. **Datum and profile:** Select an explicit vertical plane, use normal-to-sketch view, and create a closed axial housing profile with a 40 mm bore, 60 mm outside diameter, 90 mm flange diameter, 8 mm flange thickness, and 35 mm total axial length. Use construction geometry for the axis. Compare grid-snapped points with exact dimensions; inspect constraints before finishing.
2. **Revolved body:** Revolve the profile 360 degrees around the construction axis. Test axis highlighting, selection order, angle entry, preview, Cancel, and re-entry. If revolve cannot be completed, record the exact UI barrier before trying concentric-profile extrusion as a separate alternative.
3. **Flange fastener feature:** Create one 6.6 mm through-hole on a 74 mm pitch circle. Test face preselection versus tool-first placement and repositioning before commit.
4. **Circular repetition:** Locate circular-pattern mode through visible UI and attempt four equally spaced holes. Determine whether the tool patterns features or whole bodies; do not pass a body pattern as equivalent to repeated holes. If hole-feature patterning is unavailable or undiscoverable, record that result and attempt separate holes only as a documented fallback.
5. **Slot and cuts:** Sketch an axial or flange slot, test numeric width/length and endpoint snapping, then select the intended region and perform a cut. Verify direction and affected body before committing.
6. **Edge treatment:** Drag a fillet toward a valid size, type 2 mm, cancel, and repeat. Try selecting multiple edges and switching to chamfer; inspect whether selection and values remain intelligible.
7. **Navigation and hover:** Inspect bore from front/back/isometric views and section it. Test zoom-to-fit, zoom-to-selection, orbit pivot, panning, and wheel direction. Check tooltips and tree/canvas hover correspondence using a supported hover input method.
8. **Change propagation and recovery:** Change flange thickness from 8 to 10 mm. Inspect hole extents, slot, and edge treatments; Undo and Redo the change. Leave the document unsaved unless saving is requested.

This scenario tests a different dependency structure and exposes distinctions between geometry creation, feature repetition, precise placement, and design revision. It should be reported as completed only after each visible result has been inspected.

### Addendum verdict

The first pass demonstrated useful construction functionality, including a counterbore, pattern, and chamfer. Relative to the documented workflows above, **the clearest observed gaps are trust in recovery, visibility of editing history, selection clarity, and numeric-input confidence—not lack of toolbar commands.** Hover, grid snapping, advanced extents, and navigation remain open validation areas rather than confirmed product deficiencies.

The second manual modeling pass requires a responsive native OneCAD window. This addendum provides comparison and concrete improvements now, without presenting research or a prepared scenario as completed manual testing.

## Addendum B — relaunch and new-project recurrence

After the user reported quitting the previous OneCAD session, Computer Use reopened the native bundle successfully. Its start screen was readable in approximately 4 seconds and exposed New project plus two unsaved-document recovery entries. Neither recovery entry was restored or discarded.

Clicking **New project** and requesting the resulting window state returned `noWindowsAvailable` after approximately 14 seconds. A subsequent state/screenshot request returned `timeoutReached` after approximately 17 seconds. Resetting the Computer Use session and attaching through the bundle ID also returned `timeoutReached` after approximately 17 seconds. No editor state or new geometry could be verified.

**Attribution update:** The failure pattern recurred before any new sketch or shell operation. The earlier failure's timing after Shell confirmation therefore does not establish Shell as its cause. The remaining possibilities include application responsiveness and native control/capture problems; UI evidence alone does not distinguish them. A direct user check of window responsiveness was requested.

Second-part construction, hover, snapping, dragging, and navigation remain unexecuted in this resumed attempt. All prior report content is retained; this entry narrows the interpretation of the earlier blocker rather than declaring another modeling failure.

## Addendum C — rebuild recovery and completed second live exercise

### Recovery and evidence boundary

The user confirmed that the window appeared frozen and explicitly requested termination, recompilation, and relaunch. The identified bundled OneCAD process and its child worker were terminated. Autosave files were not deleted. The geometry worker was rebuilt and staged with `scripts/build-worker.sh Release` (exit 0, compiler deprecation warnings); the native debug bundle was rebuilt with `bun run tauri build --debug --bundles app` (exit 0, frontend chunk-size warning). These are build results, not regression-suite results.

The rebuilt native app launched successfully. **New project now opened a responsive editor**, and the live modeling exercise below proceeded without the sustained timeouts of the preceding attempts. Rebuilding and terminating processes changed more than one condition; no root cause or permanent fix is established.

This recovery required reading repository build instructions, package metadata, and status ledgers. No application source was edited or inspected to derive the UX conclusions below. Earlier UI-only findings remain as originally recorded. The report now includes build/recovery evidence separately from the manual review.

### Different part produced

Created a **revolved flanged housing**, then added four separate flange Hole operations and a slot cut. This is a visibly more varied feature chain than the first-pass patterned plates.

- Sketch 1: six-segment closed stepped section and one construction axis; seven entities, DOF 9 on finish.
- Revolve: construction line selected as axis, 270° numeric preview verified, then 360° committed. UI showed Body 1 / Solid body, Revolve 360°, and “Revolved.”
- Four Hole operations: default Ø6.6 and Thru setting, individually positioned approximately around the flange. Hole outlines appeared and Hole history became visible. Exact pitch circle, equal spacing, and through-depth were not independently verified.
- Sketch 2: slot constructed with endpoint clicks and numeric width 6 mm; UI expressed the end radius as 3 mm. Finish reported ten entities and DOF 12, including existing/reference geometry.
- Slot extrusion: selected one region, dragged through zero, then committed explicit positive 15 mm with Cut shown. A visible open slot resulted. The status said “Extruded,” although this was a cut.
- Sketch 3: two straight-line snap comparisons. Clicking sketch Cancel retained both lines and the sketch. It was hidden afterward, rather than deleted.

Approximate housing proportions followed a 40 mm bore / 60 mm barrel / 90 mm flange / 35 mm axial length concept, with a roughly 10 mm flange instead of the earlier proposed 8 mm. These proportions came from grid-based placement and were **not fully dimensioned or independently measured**. Do not treat them as certified dimensions or a manufacturing-ready model.

Final state: housing visible in Home view; test Sketch 3 hidden; grid snapping restored to on; section view off; document unsaved. No existing recovery entry was restored or discarded.

### Second-pass manual test matrix

| Test | Observed result | Judgment |
|---|---|---|
| Vertical sketch plane | Clicked a visible vertical plane; camera aligned to RIGHT and empty sketch grid appeared. | Pass. Clearer and more responsive than first pass. |
| Wheel/scroll zoom | Scrolled over sketch viewport; grid scale changed from 20 to 5 mm and view zoomed around the pointer area. | Pass for this input; precise zoom-anchor invariance not measured. |
| Polyline closure | Six clicks plus return to starting point produced a shaded closed stepped region. Horizontal, vertical, and coincident constraints accumulated. | Pass. Shading provides useful closure feedback. |
| Under-constrained sketch dragging | Dragged upper-right vertex with grid on, with grid off, and after explicitly selecting it. Pointer coordinates changed, but vertex/profile did not visibly move; DOF remained 6. | No successful sketch drag observed. Reproduce with physical mouse before assigning cause, because gesture injection remains a possible factor. |
| Construction geometry | Enabled Construction, drew a dashed vertical line, finished sketch, selected that line for revolve. | Pass. Construction line was excluded from shaded profile. |
| Revolve axis and angle | Axis-selection prompt accepted construction line. Changing 360 to 270 showed partial body; changing back to 360 and committing produced full housing. | Pass. Explicitly focused multi-digit angle input worked. |
| Revolve Undo | Command-Z left body and Revolve 360° history present. Control-Z also left them unchanged. | Recovery concern reproduced on a different operation after rebuild. |
| Top/Home navigation | Top face control oriented the housing for hole placement; Home returned to isometric framing. | Pass in inspected states. A later Top invocation around tool transitions did not leave the expected final top view; consistency not established. |
| ViewCube dragging | Diagonal and vertical drags changed grid/model roll while cube continued to show TOP. | Camera movement observed; free out-of-plane orbit not demonstrated. Tooltip says drag to orbit, so gesture behavior needs clearer explanation/testing. |
| Hole creation and history | Four individual Ø6.6 Thru operations produced circular boundaries. First Hole entry appeared in history, and selecting it exposed parameter/history controls. | Improvement over first-pass history visibility. Full downstream history still inconsistent by selection state. |
| Feature circular pattern | Selected Hole history entry; opened pattern dropdown. Both Linear and Circular pattern were disabled. | Intended hole-feature repetition could not be invoked from this selection. Four manually placed holes are a fallback, not a passed circular pattern test. |
| Face selection after operations | Face selection following holes reported “Selection is out of date — pick again”; sketch creation nevertheless proceeded. | Stale-selection warning reproduced. State/action consistency remains confusing. |
| New sketch tool state | Sketch 2 started with Construction still on from Sketch 1. Disabled it before drawing slot. | Persistent state can surprise users; cross-sketch construction mode needs conspicuous feedback. |
| Slot creation | Two endpoint clicks advanced to width placement; typing 6 and Enter created slot with Radius 3 mm constraint. | Pass. Guidance still used generic length wording while visible chip indicated width. |
| Region selection | Extrude first showed 0 regions; clicking slot changed count to 1; Enter advanced to depth preview. | Pass. Explicit region count helps avoid unintended broad-profile extrusion. |
| Extrusion dragging | Dragged arrow upward from default +10 mm; chip became approximately −20.4 mm and Add changed to Cut with red preview. | Pass for handle manipulation and mode feedback. Sign alone did not communicate useful cut direction for this face orientation. |
| Explicit cut refinement | Opened options and inspected New Body/Add/Cut, Blind, Draft controls. Entered +15 mm; Cut remained displayed. Enter committed visibly open slot. | Pass for this refinement. Cut preview initially extending away from body illustrates why operation and direction must be checked separately. |
| Single-edge measurement | Clicking a flange-hole rim displayed Length 20.735 mm. | Pass for length readout; diameter was not directly labeled. |
| Two-edge measurement | Selecting a second hole rim displayed Center ↔ center 50.849 mm, with ΔX −35.962, ΔY −35.949, ΔZ 0. | Pass for displayed measurements. These were two selected rims, not a verified diametrically opposite pair. |
| Measurement layout | Two edge-length chips and center-distance/delta chip overlapped around the geometry; lower-left measurement panel also displayed result. | Useful data, crowded annotation placement. |
| Section view | Toggle clipped housing to reveal barrel wall/bore; Escape restored normal view. | Pass for default section and exit; arbitrary plane positioning was not tested. |
| Grid-on placement | With displayed 5 mm grid, clicked line endpoints 108 screen pixels apart, slightly away from grid intersections. Selected line displayed 31 mm. | Behavior observed, not proof of grid accuracy or failure. |
| Grid-off placement | Disabled only Grid; drew another 108-pixel horizontal line at a different location without changing view. Selected line displayed 30 mm. | Comparative behavior observed. Dimension rounding, guide snapping, and auto-constraints remained on; effect of grid alone is not isolated. |
| Restore snapping | Re-enabled Grid and confirmed switch on before leaving comparison. | Pass. |
| Sketch Cancel semantics | Clicked Cancel while Sketch 3 contained two lines. Editing ended, but Sketch 3 and both lines remained visible with DOF 6. | Confirmed semantic mismatch with a common expectation of discarding current sketch work. |
| Pure hover/pan/modifier orbit | No supported standalone hover or modifier-held drag API was available. | Not tested. Click-triggered tooltips and ViewCube roll do not fill this coverage gap. |

### New and updated UX findings

#### C1 — High: Undo still did not recover a modeling operation

The earlier pattern Undo concern now has a second operation as evidence: the newly rebuilt app retained its revolved solid after Command-Z and Control-Z. This strengthens the observed recovery problem without proving where shortcut/menu routing fails. Validate both visible geometry and history removal, then Redo restoration. A subtle camera/frame change is not a successful model Undo.

#### C2 — Medium: sketch Cancel means exit, not discard, in this test

**Reproduction:** Create a fresh sketch, draw two lines, click Cancel. Both lines and the sketch remained after edit mode closed.

**Improvement:** If retaining edits is intentional, label the action “Exit sketch” or “Close.” If Cancel is intended to revert, state its rollback boundary and enforce it. Do not leave users to infer whether their work was accepted. This matters especially when Undo is unreliable.

#### C3 — Medium: feature repetition workflow blocked by selection type

Hole was available in history, but selecting it left both pattern modes disabled. This is a concrete barrier to a common flanged-part workflow. It does not establish that no alternate feature-pattern route exists.

**Improvement:** Explain the supported pattern subjects in the disabled state. If only bodies are supported, say “Select bodies; feature patterns unavailable” and guide the user toward a supported sketch-pattern or repeated-hole approach. If features are supported elsewhere, expose that route from the Hole entry.

#### C4 — Medium: sketch drag affordance and DOF explanation need work

A selected vertex showed an orange ring, and the sketch reported remaining freedom, yet repeated drags did not visibly alter it. The status coordinates reflected pointer movement without explaining why geometry stayed put.

**Improvement:** Verify drag routing with a physical mouse. If constraints prevent the requested motion, highlight the limiting relationships. If movement is allowed, preview and commit it consistently. Remaining DOF does not guarantee any particular vertex can move in any direction, so the UI should explain the local limitation rather than rely on the aggregate number.

#### C5 — Medium: operation intent and spatial direction are separate decisions

Dragging across zero gave helpful Add/Cut text and color changes. However, the negative Cut volume visibly extended away from the housing; explicit positive depth with Cut retained produced the actual slot.

**Improvement:** Indicate affected material and target body, not merely signed distance. Preserve an explicitly chosen operation and distinguish it from automatically inferred operation. Give a non-intersection message when a cut does not intersect its target. After successful cut, prefer “Cut created” over generic “Extruded.”

#### C6 — Medium: persistent Construction mode crosses sketch boundaries

Construction remained on when a new face sketch started. This could quietly turn intended solid profiles into reference geometry if the user misses the highlighted icon.

**Improvement:** Show a persistent “Construction geometry” banner or clear near-pointer cue; consider a per-sketch default based on observed user workflow. Do not change persistence policy solely on this one test without checking intended conventions.

#### C7 — Medium: snap/rounding feedback does not expose which rule won

The two equal-screen-span lines displayed different lengths with Grid on/off, while dimension rounding and guides remained active. The 31 mm result with a 5 mm grid does not by itself prove a defect: point placement, length rounding, and relationships can interact.

**Improvement:** Expose the actual snapped coordinates and the winning snap/rounding rule during placement. Distinguish “point snapped to grid” from “length rounded.” Next test should disable all competing aids and measure endpoints, then re-enable each aid separately.

#### C8 — Low/Medium: measurement is functional but congested

Edge length and center distance are useful, precise readouts. Multiple chips competed for the same small area. Circular edge selection showed circumference-style Length rather than a readily recognizable diameter.

**Improvement:** Offer diameter/radius as primary circular-edge values with length available secondarily. Prevent annotation overlap or use the existing fixed measurement panel as the primary detailed display.

### Reassessment of earlier findings

- **Startup/reliability:** This rebuilt run stayed usable throughout the second exercise. Previous frozen sessions remain recorded; the failure is not proven fixed permanently.
- **Missing history:** Hole history was visible in this run. Therefore, “Hole history absent” is not a universal finding. Later body/edge/sketch selections still displayed different and apparently incomplete subsets: for example, the post-slot body inspector initially showed only Sketch/Revolve/Hole, while an edge selection exposed Extrude 15 mm. Clear filtering and Show All remain valid recommendations.
- **Viewport and plane visibility:** Right-plane alignment, stepped-profile shading, revolve preview, and cut preview were now visible. Earlier inconsistent captures should remain qualified rather than generalized as a persistent renderer failure.
- **Dragging:** Extrude dragging passed, supplementing first-pass fillet dragging. Sketch-vertex dragging did not visibly succeed; ViewCube dragging produced roll but did not establish general orbit behavior.
- **Numeric input:** Explicitly focused 270/360-degree revolve and 15 mm cut inputs worked. This narrows the first-pass unexpected “12 → 2” observation to its specific focus/input sequence.
- **Snapping:** Live on/off placement and drag comparisons were now attempted. They improve coverage but do not establish tolerance, precedence, or coordinate accuracy.
- **Professional comparison:** The documented Shapr3D/Fusion recommendations still apply as interaction references. This pass especially supports better feature-selection guidance, recovery semantics, and visible numeric intent. It does not justify a claim of overall functional parity or infer absence of untested tools.

### Second-exercise completion and remaining coverage

The second live exercise is complete as a bounded UX review: a different multi-feature housing was created, dragging and grid settings were exercised, and navigation/measurement/section controls were inspected. The prepared ideal scenario was adapted: four holes were placed separately; the part was not fully dimensioned; no edge fillet was added in this run; save/reopen and dependency-change propagation were not tested.

Pure hover, physical-device panning/orbit, isolated snap-accuracy tests, successful sketch dragging, robust Undo/Redo, and persistence remain open. The app is left running with the unsaved housing for further inspection.

## Addendum D — engineering response: what was done and how

Date: 2026-09-11. Scope: every finding above that had a code-side cause. Method: plan → root-cause evidence → twelve work packages delegated to implementation agents, every diff reviewed by the orchestrator, identity-critical work reviewed by a fresh-context adversarial reviewer and by GPT-6 Astra (`/codex-astra break`), and the full gate ladder re-run on the main thread after every change. The working tree holds the result uncommitted; the ledger is the head of `TODO.md`, the run log is `PLAN.md`, and the accepted Astra derivation is `docs/design/astra/wp-u4-selection-survival.md`.

### Root causes established before any code moved

The review deliberately inspected no logs. The session log of the Addendum C run (`logs/dev.jsonl`, pid 48299) and one new real-worker test settled the four highest-impact findings:

| Finding | Evidence | Cause |
|---|---|---|
| Undo did nothing (H1, C1) | Both ⌘Z / ⌃Z presses after the Revolve arrived at the backend as `redo` (11:48:52, 11:49:02); `undo` was never invoked. | The injected chord carried Shift (uppercase `Z`), and ⇧⌘Z is Redo. Independently: the app had no native menu (the Edit → Undo you used was macOS text undo), the command palette had no Undo entry, a no-op revert produced no message, and the chord fired even inside a focused number field. |
| "Selection is out of date — pick again" on a fresh pick (H3, second pass) | `promote_selection` refused a pick addressed to the current head snapshot; a new test (`src-tauri/tests/selection_promote.rs`) reproduced it on the revolve + four holes scenario, red on all four stages. | The tessellation id table names an element that an operation has already bound by its persistent ElementId (`el_…`) instead of its snapshot ordinal (`f:N`). The viewport copied that label into the pick and sent it for promotion, which the worker cannot parse. The hole's host face was exactly such an element. |
| Displaced highlight after the chamfer (H3) | Element counts only grow across a regeneration (12 → 15 edges after the chamfer), so every old ordinal still resolves — to a different element. | The viewport geometrically re-pointed the consumed edge's selection to its nearest neighbour, and trusted a reused ordinal as identity. |
| "12" committed as 2 mm (M1) | Deterministic from source: typing a digit on the canvas seeds the field with "1", and the field's mount effect selected that text, so the "2" replaced it. | Seed-then-select race, not a parser defect. |

The silent fillet clamp discarded its own "clamped, and why" result; the History section for a selected body was a hard slice of the first three timeline rows; sketch Cancel was "squash and finish"; construction mode was sticky across sketches by design; the Std picker rendered a placeholder glyph in every cell.

### Resolution by finding

| Review finding | Status | What changed |
|---|---|---|
| H1 / C1 — modeling recovery | Fixed | One shared undo router (`src/features/shell/undoActions.ts`) serves the keyboard chords, the command palette ("Undo Extrude" / "Redo Fillet", enabled by history depth) and a new native menu (File → New/Open/Save/Save As, Edit → Undo ⌘Z / Redo ⇧⌘Z, `src-tauri/src/menu.rs`). A focused text field keeps native text undo; outside one, ⌘Z undoes the document. A no-op revert now says "Nothing to undo" / "Nothing to redo" immediately; a revert blocked by an open drag says "Finish the drag first". The projection carries undo/redo depth and the label of the next step. |
| H2 — shell commit ended testing | Not reproducible from code | Addendum B showed the freeze on New project with the old bundle and the rebuild cleared it; the pre-rebuild log was truncated by the relaunch. No evidence survives. Recorded as a follow-up (a frontend heartbeat line in the debug log so a future hang leaves a trace). |
| H3 — stale / displaced selection | Fixed (identity-critical) | A pick whose label is already an ElementId is never sent for promotion; the backend refuses a non-TopoKey pick with a self-describing message. The geometric rebind was deleted. After a regeneration a selected face or edge survives only when the new mesh names its ElementId or the backend confirms it by ElementId (fenced on body, kind and drawability in the displayed mesh); an unpromoted selection is dropped. Fillet, chamfer and shell clear the inputs they consumed; a hole keeps its seat face, so four holes on one flange no longer need a re-pick. Edge picks and projection sources now address by ElementId. The first cut of this package was returned "defective" by both Astra and the local adversarial reviewer on the same blocker (a stale ordinal always resolves); every finding was closed with a failing test first. |
| M1 — numeric entry | Fixed | A seeded field places the caret at the end instead of selecting the seed. |
| M2 — oversized fillet | Fixed | The clamped value is applied and disclosed: "Radius limited to 11.99 mm — largest that fits the selected edges" (or "moved to … — nearest size that builds"), cleared on the next in-range edit. |
| M3 — floating controls / labels | Fixed (labels, Std picker, accessibility); docked inspector deferred | Every chip field has a semantic name with its unit ("Depth (mm)", "Counterbore diameter (mm)", "Angle (°)"); the chip layer is no longer hidden from assistive technology; the Std picker prints the value each cell applies ("Ø3.4") with a full accessible name. Chip relocation and a docked parameter inspector are follow-ups. |
| M4 — sketches intercept solid selection | Fixed | A body edge wins a tie against a coincident finished sketch; a sketch still wins against a body face and when it is clearly in front; Alt pick-through is unchanged. |
| M5 — incomplete history | Fixed (as far as the data allows) | The selection History lists the whole timeline, labelled "N features · nothing filtered". The projection carries no feature-to-body lineage, so there is no per-body filter and therefore no "Show all" toggle; the label makes the absence of filtering explicit. |
| L1 — constraint messaging | Fixed | A face sketch that holds only projected references reads "Projected geometry only · N projected reference edges · draw geometry to begin" instead of "Fully constrained". The "Size fixed; position still free" sentence was not implemented: it cannot be stated honestly without a per-entity constraint decomposition the projection does not expose. |
| L2 / C7 — which snap rule won | Fixed | Cursor rounding names itself: the snap hint reads "Grid · Rounded" when both applied. A rounding-only placement is, by the existing snap contract, not a snap and stays unlabelled. |
| C2 — sketch Cancel | Fixed (your decision D-1) | Cancel now reverts to the state at sketch entry and deletes a sketch created in that visit; Esc and Finish keep today's exit-and-keep. A refused discard (history trimmed, or model edits interleaved) keeps the geometry, finishes the sketch, and says why. The adversarial review's one major finding on this package (a refused discard must still write the timeline record) was fixed. |
| C3 — feature patterns | Deferred; reason now stated | Patterns are body-level in the kernel and the protocol. The disabled reason now reads "Select a body to pattern — feature patterns are not supported yet", and it is shown as a status message when a disabled tool is invoked by shortcut or palette, not only as a tooltip. Feature-level patterning is a kernel work package recorded for later. |
| C4 — sketch vertex drag | Unattributed | The mock-lane drag specification is green; the failure is not reproducible from code and needs a physical mouse. |
| C5 — intent versus direction | Fixed (messages) | Commit messages name the operation: "Cut created", "Joined", "New body created", and "Cut did not intersect any body" when a cut changed nothing. Fixing this also exposed a latent bug: the boolean mode was read after the tool machine had reset it, so every Cut would have been labelled as a new body. |
| C6 — construction mode | Fixed (your decision D-2) | Off on every sketch entry; a persistent "Construction" chip sits next to the toggle while it is on; toggling reports "Construction geometry on — new entities are reference only". |
| C8 — measurement | Fixed (values); overlap deferred | A circular edge reads "Ø 6.6 mm · Length 20.735 mm" and a cylindrical face "R 20 mm · Area …". The protocol audit refused adding a radius to the element-info verb (one of its two addressing rungs never reaches geometry), so the value comes from the existing classification verb with no wire change. Chip de-overlap is a follow-up. |

### Verification

Measured on the main thread, sequentially, after the last change:

| Gate | Result |
|---|---|
| Frontend unit suite (`bun run test`) | 322 files, 5700 passed / 78 skipped / 0 failed (two consecutive runs) |
| Playwright, chromium + webkit, `retries: 0` (`bun run e2e`) | 542 passed / 0 failed, 31.0 min, run alone |
| Rust workspace with the real worker (`ONECAD_REQUIRE_WORKER=1 cargo test --workspace`) | 1593 passed / 0 failed / 0 ignored, 99 targets |
| Worker CTest | 193 / 193 |
| tsc, fmt, clippy `-D warnings`, hex gate, three QA verifiers, worker stdout hygiene | clean |

One load-dependent flake surfaced in the full frontend run (a sketch-finish confirmation asserted after a fixed tick count); the tests now poll for the message. One Playwright lane that had been started before the last identity fix was discarded as contaminated and re-run.

### What only a person at the bundled app can confirm

The mock lane cannot express these; they are recorded as owed in `TODO.md`:

- The menubar shows OneCAD / File / Edit / Window; Edit → Undo removes a committed Extrude and Redo restores it; ⌘Z undoes once, and held ⌘Z repeats steadily; ⌘Z inside a number field undoes typing only; ⌘Z on an empty history says "Nothing to undo" without a pause; ⌘W still closes the project; cut/copy/paste/select-all in text fields still work.
- Typing 100 mm on the 12 mm plate's fillet shows 11.99 mm and the "limited to" message.
- Revolve, then four holes on one face: the seat face stays selected and no stale-selection message appears.
- Chamfer, ⌘Z, re-pick the restored edge, ⇧⌘Z: the selection empties (undo and redo publish no body change in the mock lane).

### Follow-ups recorded

Feature-level hole patterns; the freeze root cause; Astra's remaining finding that the documented identity ladder (SCHEMA §10) permits a congruent twin to bind onto a stale anchor, which is weaker than "backend confirmation proves physical identity" and lies outside this package's path; chip de-overlap and relocation; a docked parameter inspector; ViewCube orbit versus roll; sketch-vertex drag with a physical mouse; and the "Size fixed; position still free" sentence once the projection carries per-entity constraint state.

---

## Addendum E — independent post-fix native manual retest

Date: 2026-09-11, evening, Europe/Bratislava. This addendum preserves all previous findings and the engineering response above. “Fixed” in Addendum D is an implementation claim; the verdicts below describe what this manual session actually demonstrated.

### Outcome and evidence boundary

**Substantial improvement, but no clean native UX sign-off.** A moderately complex housing was modeled through the native application, with sketches, a shell, annular boss, four holes, a side cut, and an angled chamfer. Body patterning, undo/redo, autosave recovery, measurement, navigation, and additional sketch interactions were exercised. Three persistent losses of native UI access interrupted the run. Each followed a different operation whose backend log recorded successful publication.

The `onecad-computer-use` skill determined the native bundled-app workflow, accessibility-first interaction, and unsaved test-project policy. Modeling used Computer Use only: visible controls, screenshots, clicks, plain dragging, and keyboard input. No application source or modeling specifications were explored. Build instructions and the updated review were read; runtime logs and short process samples were captured only after persistent failures, before relaunch. No application fixes were made.

Build and environment:

- Base commit `80d4e74`, with the existing uncommitted engineering changes present. This is not a clean-commit qualification.
- `scripts/build-worker.sh Release`: exit 0; stdout hygiene passed; worker and manifest staged. Compiler warnings remained, including OCCT deprecations. Compiling test executables is not a test-suite run.
- `bun run tauri build --debug --bundles app`: exit 0; TypeScript/Vite and Rust compilation succeeded; Vite reported large-chunk warnings. The extra `--` in the skill example was omitted as required by the repository's Tauri build instructions.
- Native app: `/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src-tauri/target/debug/bundle/macos/onecad.app`; accessibility identified `tauri://localhost`, not a browser mock.
- App version 0.1.0; ARM64; the process sample reports macOS 26.4 (25E246). Most screenshots used an approximately 1156 × 768 logical-pixel window.
- Addendum D's automated-suite results were not rerun or independently re-certified in this session.

### Part produced and final state

The final recovered body was renamed **UX Housing**. The application was left responsive, in Select mode, with no selection, Shaded + edges display, and section view off. The project remains unsaved; no existing project was overwritten or deleted.

| Feature | Input and observed result |
|---|---|
| Base | Rectangle entered as 100 × 70 mm; both distance constraints visible; DOF 2. Extruded by typing `12`, producing history `Extrude 12 mm`, then edited through history to 30 mm. |
| Hollow enclosure | Top face shelled at 3 mm; visible open cavity, floor, and walls; `Shelled` confirmation. |
| Internal boss | Concentric circles entered as Ø30 and Ø12; a center snap and Coincident constraint appeared. Single annular-region extrusion at 15 mm joined to the floor. Later cylinder measurement reported R15 mm and area 1413.717 mm², consistent with the intended outer cylindrical surface. |
| Mounting holes | Four Ø6.6 through-holes placed individually on the floor in the final uninterrupted run. One measured rim reported Ø6.6 / length 20.735 mm. One horizontal center-to-center pair measured 48.048 mm, ΔY 0, ΔZ 0. Positions were pointer-placed, not a fully constrained production bolt pattern. |
| Connector opening | Side-face slot: two pointer-placed centerline endpoints, numeric width 8 mm, represented by Radius 4 mm. Extrude used explicit Cut, Symmetric, and 10 mm depth. Opening visible; `Cut created` confirmation. Overall slot length/position were not dimensionally certified. |
| Corner treatment | One outer vertical edge: Chamfer 1 mm, angle 30°, with Flip reference exercised. Commit reported `Chamfered 1 edge`. Exact bevel angle was not independently measured. |
| Repetition test | Linear body pattern: X axis, 3 total, 120 mm spacing. Three complete housings visible after Zoom to fit; one native Undo removed the two copies. |
| Final timeline | 12 features: three sketches, base extrusion, shell, boss extrusion, four holes, slot extrusion, chamfer. The later Offset Face attempt is not in the recovered final model. |

This is a UX exercise, not a manufacturing-ready, fully constrained design. The exact Revolve-plus-four-holes scenario from Addendum D was not rerun: the four-hole retest used the shelled extruded housing.

### Regression verification against the engineering response

| Finding / promise | Live verdict | Evidence / limitation |
|---|---|---|
| Native menu and empty Undo feedback | Verified with new concern | File/Edit/Window present. Edit → Undo on a newly created empty project immediately said `Nothing to undo`; an Unsaved changes marker appeared afterward. |
| Modeling Undo/Redo | Verified through explicit native/menu and palette clicks; interaction details remain | Palette row click removed the base extrusion and explicit Redo restored it. Native menu Undo removed Chamfer (12 → 11 features); re-pick restored edge; native Redo restored the feature (11 → 12), and inspector selection cleared. Pattern Undo removed copies. |
| Semantic undo labels | Not as described in D | Live palette/status used `Undo Add Operation`, `Undid Add Operation`, `Redid Add Operation`, and `Undo Toggle Visibility`, not feature-specific `Undo Extrude`/`Undo Chamfer`. |
| Single-action extrusion recovery | Friction remains | First native Undo after extrusion made its automatically hidden sketch visible; the model operation required another undo. A cosmetic automatic step separated the user's modeling action from its recovery. |
| Keyboard recovery | Not certified | Injected `super+z` did not reproduce a dependable undo and appeared to restore visibility. The earlier injector/Shift explanation remains relevant. One palette Enter attempt closed the palette without removing the extrusion; direct row clicking later worked. Input timing/injection versus app behavior was not isolated. |
| First-digit numeric entry | Verified in exercised cases | Canvas type-to-enter preserved `100` and `70` for rectangle dimensions, `12` for extrusion, and `30` / `12` for circles. |
| Fillet clamp explanation | Not verified; silent clamp still observed | Typing 100 on a 12 mm plate changed the field/preview to 11.999. After Tab and direct-field retry, visible status still showed generic fillet guidance, not a limit explanation. Commit then caused persistent UI-access loss. |
| Fresh face picks / hole seat | Improved in exercised sequence | Floor selection after shell, repeated hole placement, and side-face sketch entry worked without the old stale-selection message. One two-hole sequence retained the selected face; a later four-hole sequence completed from a body-selected workflow. Exact coincident edge-versus-sketch tie and arbitrary identity survival were not isolated. |
| Consumed selection clearing | Logical state verified; visual ambiguity remains | Shell and chamfer commits showed `Nothing selected`. After chamfer undo/re-pick/redo, the inspector also cleared, but a blue edge remained visible. This could be retained hover at the old pointer location; it is not proof of wrong physical-identity rebinding. |
| Full timeline | Verified | Visible history grew past three rows to 12 and 13 features, with `nothing filtered`. |
| Face-sketch messaging | Verified | `Projected geometry only` and `4 projected reference edges · draw geometry to begin` appeared on floor and wall sketches. |
| Construction reset / indicator | Verified | Toggle showed persistent Construction chip and reference-only message. Cancel, then new sketch: Construction off. |
| New-sketch Cancel | Verified twice | A construction-line sketch disappeared with `Sketch changes discarded`; a later multi-tool test sketch was completely discarded without disturbing the 12-feature housing. Existing-sketch rollback and refused-discard branches were not tested. |
| Named operation completion | Verified | `New body created`, `Joined`, and `Cut created` observed. Non-intersecting cut message not exercised. |
| Hole sizing / labels | Verified | Standard-size cells displayed values; M4 normal fit Ø4.5 applied 4.5. Fields exposed Hole diameter (mm), Hole depth (mm), and through-depth help. Counterbore commit was not repeated this pass. |
| Circular measurement values | Verified | Rim Ø6.6 / 20.735 mm; cylinder R15 / 1413.717 mm². Annotation collisions remain. |
| Feature-pattern explanation | Verified, with activation-state issue | Status explicitly said `Select a body to pattern — feature patterns are not supported yet`. Disabled menu entries alone still did not explain it. Selecting a body afterward did not start the active tool; Select → body → Pattern was needed. |
| ViewCube orbit | Verified this pass | Plain diagonal cube drag from RIGHT produced a perspective on front/side/interior geometry, not merely an in-plane roll. |
| Sketch vertex dragging | Still unverified / ineffective through this tool | Both dimensioned rectangle-corner drag and an under-constrained line-endpoint drag changed pointer coordinates but not the visible geometry. Extrusion-handle dragging worked. A physical-mouse comparison is still needed. |

### Detailed interaction coverage

| Interaction | Test and result |
|---|---|
| Drag → exact number | Boss handle dragged from 10 to 21.87 mm with visible geometric preview; focused depth field then set to 15 and committed. Successful transition. |
| Focus / immediate input | One history-value click followed immediately by select-all/type/Enter did not change 12. After observing the mounted focused input, typing 30 and Enter succeeded. Fast-input behavior deserves explicit testing; not counted as recurrence of the first-digit bug. |
| Region confirmation | Annulus click showed `1 region`. Clicking floating Confirm changed guidance to `2 regions` and eventually created two Extrude rows; center looked filled. Repeating on the recovered same sketch with Enter preserved a single-region operation, one Extrude row, and a visibly hollow boss. See E2. |
| Grid / rounding | With Grid on and Dimension rounding off, a 116-pixel line span measured 30.457 mm. With Grid off and rounding on, a matching-span line elsewhere measured 30.5 mm. Guides/auto-constraints remained enabled; the second line gained VerticalPoints alignment, so this is not an isolated solver/snap accuracy benchmark. |
| Snap feedback | `Center`, `Grid`, and `Horizontal` hints observed. Exact `Grid · Rounded` combined hint was not captured; do not mark that specific acceptance test passed. Grid availability did not make every arbitrary click an exact grid placement. |
| Sketch constraints | Numeric rectangle dimensions and automatic H/V/coincidence worked. Three-point arc added HorizontalPoints; vertical crossing line added Vertical. UI names `VerticalPoints`/`HorizontalPoints` read like implementation terminology. |
| Three-point arc / trim | Start/end/third-point clicks produced an arc. A crossing vertical line enabled trimming the right arc segment; the left segment remained. DOF rose from 14 to 15 and HorizontalPoints disappeared. No explicit explanation of the released relationship was visible. |
| Slot guidance | During width placement, the live field said W/Live width, but bottom guidance continued saying type a number for length. Entering 8 yielded Radius 4 mm. Geometry worked; terminology did not track the active phase. |
| Chamfer variants | Fillet → Chamfer switch, angle 30°, Tab navigation, and Flip reference exercised. Flip reference gave no persistent A/B state in the exposed controls; geometry was too small to independently certify its direction. |
| Invalid Offset mode | Planar face offered enabled Radius/Diameter modes. Clicking Radius reported `Radius requires a cylindrical operative set` and removed the parameter controls, while Offset remained active. Reset through Select was needed. |
| Camera / view | TOP, RIGHT, Home, Zoom to fit, Persp/Ortho, and cube orbit worked. Home and sketch entry frequently framed world origin rather than useful local geometry; manual fit was needed. A fit requested immediately after sketch entry sometimes needed repeating after the transition settled. |
| Section | Toggle visibly clipped walls/boss and exposed floor holes; Esc restored full model. No visible plane offset/flip controls were found in the exercised state. Arbitrary sections were not tested. |
| Display | Wireframe exposed internal outlines; restored Shaded + edges. Selected face tint remained visible in wireframe. |
| Context menu | Body right-click exposed Rename, Hide, Save as Component. Rename worked; final body named UX Housing. Component export not invoked. |
| Recovery | Three relaunches offered the test project's dated autosave; Restore worked each time. First recovery lost the un-autosaved fillet; second recovered the boss sketch but not later boss/holes; third recovered the completed 12-feature housing before Offset Face. |

### High-priority remaining/new issues

#### E1 — High: persistent native stalls after successful operation publication

Three distinct incidents occurred in the rebuilt bundle:

| Incident | Trigger | Native evidence before lost access | Recovery |
|---|---|---|---|
| F | Commit fillet after typing 100 and clamping to 11.999 on the 12 mm plate | 16:46:20 UTC: ExecutePlan and AcceptPrepared successful; regeneration `published`, 26 ms, failedSteps 0, needsRepair 0 | `noWindowsAvailable`, repeated timeout, direct reattach timeout; app/worker terminated after capture; autosave restored plate |
| H | Third mounting-hole commit in first housing attempt | 16:53:19 UTC: regeneration `published`, 81 ms, failedSteps 0, needsRepair 0 | Persistent timeout; captured then terminated; autosave restored housing plus boss sketch |
| O | Commit 1 mm planar Offset Face after resetting from invalid Radius mode | 17:08:57 UTC: regeneration `published`, 156 ms, failedSteps 0, needsRepair 0 | Persistent timeout; captured then terminated; latest autosave restored the 12-feature housing |

Only identified test app/worker PIDs were terminated. Source was not inspected and no fix attempted. A separate `noWindowsAvailable` during the snap test recovered on the next state query without restart; it must not be conflated with the persistent incidents.

The first process sample places the main thread in WebKit URL-scheme handling → Tauri webview lookup/`webviews_lock` → mutex wait. That is a useful native-stall lead, not a complete deadlock diagnosis. Successful worker publication does not prove successful frontend completion; equally, a Computer Use error alone does not prove a kernel crash. Here, repeated timeouts plus process samples justify treating native responsiveness as a release-blocking investigation.

Evidence copies, captured before relaunch could truncate the active log:

- [Fillet runtime log](/private/tmp/OneCAD-UX-postfix-fillet-2026-09-11.jsonl) and [process sample](/private/tmp/OneCAD-UX-postfix-fillet-sample-2026-09-11.txt).
- [Hole runtime log](/private/tmp/OneCAD-UX-postfix-hole-2026-09-11.jsonl) and [process sample](/private/tmp/OneCAD-UX-postfix-hole-sample-2026-09-11.txt).
- [Offset runtime log](/private/tmp/OneCAD-UX-postfix-offset-2026-09-11.jsonl) and [process sample](/private/tmp/OneCAD-UX-postfix-offset-sample-2026-09-11.txt).

These paths are temporary local evidence, not durable repository artifacts. Preserve them before OS cleanup. Recommended acceptance: repeated native commits across fillet/hole/offset with UI responsiveness checks, retained logs, and explicit frontend-completion evidence—not worker success alone.

#### E2 — High: floating region confirmation can change the modeled result

The same annular sketch produced different results depending on confirmation interaction. One visible region became two after clicking the overlapping floating confirmation control; Enter preserved one. The first result had two Extrude history rows and a filled-looking center; the repeat produced one row and a hollow center.

This is a directly observed interaction discrepancy, not a proven source-level event-bubbling cause. Suspected input reaching geometry behind the chip should be investigated. Regardless of mechanism, confirmation must never silently expand selected geometry.

**Acceptance:** confirm the identical selected region by pointer, accessibility activation, and Enter; selection count, highlighted regions, operation count, and final solid must agree. Keep the chip off selectable profiles and block pointer events from passing into the modeling canvas.

#### E3 — Medium: undo works, but transaction granularity and command naming remain poor

Automatic sketch visibility added a separate recovery step after extrusion. Generic `Add Operation` labels do not tell the user which modeling action will disappear. Searching `redo` displayed Undo Add Operation first and Redo Add Operation second, with Undo selected by default. The screenshot confirms the ranking even though accessibility exposed only the selected list row.

**Improvement:** group automatic visibility with the modeling action, use semantic feature labels, rank exact Undo/Redo intent first, and show disabled reasons/depth consistently. Do not let a no-op Undo dirty a previously clean empty document. Retest keyboard Enter and text-field undo separately with a physical keyboard because injected-key behavior remains uncertain.

#### E4 — Medium: unsupported mode leaves an active tool without its controls

Radius and Diameter were offered on a planar Offset Face target. Radius produced the phrase “cylindrical operative set,” removed all parameter chips, and left the tool active. Pattern invocation without a suitable selection similarly left an active state that did not recover just by selecting a valid body.

**Improvement:** disable inapplicable modes with plain-language reasons (“Select a cylindrical face to set its radius”); preserve the last valid preview and panel; allow target correction without switching tools. Every active tool must retain an obvious Cancel and its next required action.

#### E5 — Medium: important model geometry is repeatedly obscured or offscreen

The region chip covered the annulus center; hole controls covered the hole and the standards dropdown covered adjacent geometry; the Offset Face strip spanned the face; measurement labels overlapped each other and their measured edges. Face-sketch entry sometimes positioned most of the selected face above or beyond the viewport, requiring Zoom to fit. Inline history editing widened the inspector enough to reveal horizontal overflow and push action icons toward the window edge.

These are workflow costs, not cosmetic preferences: they hide targets, weaken preview inspection, and can change what a click hits. See the consistency audit below for specific remedies.

#### E6 — Medium: retained visual or metadata states can mislead

- After undo/re-pick/redo, the logical selection cleared but a blue edge remained. Controlled hover-away is required to distinguish stale visual state from an intentionally retained hover highlight; no identity-failure claim is made here.
- After autosave restoration, selecting the same boss sketch reported Fully constrained / DOF 0, whereas its pre-restart state was DOF 6. No constraint edits had been deliberately applied. This may be unhydrated display metadata; it is not proof that geometry constraints changed. Display “not evaluated” until current solver state is available rather than showing reassuring zero by default.
- Undoing body pattern removed copies but briefly left a nameless Solid body inspector and body-dependent tools available. A new explicit selection cleared the mismatch. Removed objects should not leave actionable stale inspector state.

#### E7 — Medium/Low: inconsistent phase wording and repeated setup

Slot width input still advertises length; radius constraints replace a width concept without explaining the relationship; chamfer secondary inputs use different naming/unit conventions from primary fields; new Hole activations reset the chosen diameter. These problems increase memory burden precisely when users are switching between CAD tasks.

**Improvement:** stage-specific prompts, semantic labels with units, explicit width/radius mapping, and either retained last-used hole parameters or an obvious repeat-last-hole command. Constraint labels should use readable terms such as “Vertically aligned points.”

### UI placement and cross-tool consistency audit

The user explicitly requested validation that controls stay out of the way and remain coherent across modeling tools. This is a separate verdict from whether an operation generated geometry.

| UI surface | What is good | Friction observed | Recommended common rule |
|---|---|---|---|
| Main toolbars | Consistent icon styling, active blue state, grouped alternatives, shortcut tooltips visible after interaction | Mostly icon-only; selecting Construction expands/recenters the sketch toolbar and shifts controls; disabled states do not always expose reasons directly | Stable tool slots; optional labels; reserve space for mode indicators; explain unavailable commands beside the active workflow |
| Extrude | Named Depth (mm), clear Add/Cut/New Body controls, accessible Confirm/Cancel, useful drag preview | Options include multiple nested groups; operation mode hides behind a compact chip; region Confirm can cover the selectable region | Fixed primary-value + operation + confirm/cancel order; persistent operation badge; non-overlapping placement |
| Fillet / Chamfer | Shared tool family; angle/reference support; semantic primary field | Radius changes to Distance; secondary inputs read Second distance / Chamfer angle rather than the same unit format; Flip reference has no persistent explicit side state; clamp explanation absent in observed path | Named unit-bearing fields; visible reference A/B highlight; persistent range warning beside the adjusted value |
| Shell | Minimal thickness controls, simple preview, clear completion | Chip sits inside the cavity being inspected; limited room for validation context | Small movable primary chip plus stable inspector fallback |
| Hole | Readable standard values, semantic diameter/depth, simple/type tabs | Very wide strip at the hole location; standards dropdown covers boss/neighboring face; each activation resets size; placement dimensions not shown | Keep hole center visible; dock secondary/type/standard controls; expose repeat parameters and coordinate/datum placement |
| Offset Face | Distance-type choices and tangent-follow option exposed | Long strip blankets the target; irrelevant modes enabled; invalid choice removes the entire panel | Filter by selected geometry; retain panel on errors; use plain target requirements |
| Pattern | Excellent total/source-retained/new-body summary; axis/count/spacing understandable | Preview extends offscreen; tool-first invalid state needs manual reset; feature-pattern gap still blocks common CAD work | Keep the summary; offer Preview fit without losing current camera; recover when a suitable target is picked |
| Sketch numeric controls | Type-to-enter and Tab sizing worked; live dimensions and constraints visible | Width/length/radius terminology changes across stages; mount/focus timing affects rapid entry; dense markers obscure small entities | Phase-specific prompts and labels, predictable focus transfer, selected-entity annotation priority |
| Measurement | Correct Ø/R values and center-distance deltas | Labels sit on rims, overlap one another, and cover center-distance text; single-selection fixed panel displayed only MEASUREMENT heading while useful value stayed on canvas | Populate fixed panel for single and paired selection; collision avoidance; pin/hide annotations independently |
| History / inspector | Full timeline now visible and explicitly unfiltered; value editing discoverable | Narrow rows squeeze values/icons; inline edit produces horizontal overflow; generic repeated feature names; dependency list becomes long and weakly differentiated | Stable parameter area, responsive rows, explicit feature names and operation type, consistent scrolling; no hidden action buttons |
| Navigation / section | TOP/RIGHT/Ortho/fit/cube orbit and reversible section work | Home/entry framing may prioritize origin; no section-depth controls visible; selected geometry can read as through-wall highlighting | Fit selection on sketch entry, preserve view context, distinguish hover/selection/occluded edges, expose section plane/offset/flip |
| Completion / recovery | New body / Joined / Cut created are meaningful improvements; autosave timestamp and Restore useful | Hole says only Hole; undo says Add Operation; autosave can lag several operations; failures lose interactive explanation | Semantic operation naming everywhere; visible saved/recoverable state; responsive failure panel with retain/retry/cancel choices |

**Recommended shared tool layout:** keep the near-geometry chip small: primary value, units, operation/mode, Confirm, Cancel in stable order. Put named secondary parameters, references, target lists, errors, and preview validity in a dockable fixed inspector. Offer a deliberate pin/dock control rather than having users discover workaround camera moves. Tool-specific content should vary; interaction grammar, focus rules, and recovery should not.

This recommendation is consistent with the professional-CAD principles discussed with sources in Addendum A—clear operation intent, explicit selection, predictable parameter editing, and recoverability. Shapr3D and Fusion were not operated in this retest, and no new feature-parity claim is made. Matching their appearance is less important than matching the user's ability to predict what the next click or Enter will do.

### Coverage still owed and next acceptance pass

- Controlled hover movement, hover timing/target highlights, physical vertex dragging, modifier-assisted orbit/pan, continuous key-repeat undo, and text-field/native keyboard shortcuts. The available Computer Use interface has no documented hover-only or modifier-held drag primitive. Cube dragging is not evidence of trackpad/pan parity.
- Exact Revolve-plus-four-holes regression; large/invalid fillet recovery after the native stall is fixed; small fillet commit without interruption; repeated boolean and Offset Face recovery.
- Existing-sketch Cancel rollback, rejected-discard cases, parameter/dependency regeneration after complex edits, and saved-project close/reopen. Autosave Restore was tested; explicit Save/Open was not.
- Counterbore/countersink final geometry, circular feature patterns (still unsupported), mirror/combine/move, arbitrary datum creation, variable-driven design, and all other unexercised tool variants. No blanket pass is claimed for them.
- Isolated snap capture/rounding arbitration and combined `Grid · Rounded` feedback, with all other aids controlled; dimensional verification of the full part.
- Window-size/DPI/accessibility scaling matrix. This pass found occlusion/overflow at one common window size; it is not a responsive-layout certification.

Next priorities: **(1)** resolve/reproduce the native main-thread stall using the preserved evidence, **(2)** ensure pointer/AX/Enter region confirmation yields identical geometry, **(3)** normalize active-tool recovery and control placement, **(4)** close physical-input and persistence gates. Final verdict: useful modeling capability and several verified UX fixes, but reliability and interaction consistency still prevent professional-CAD readiness.

## Addendum F — implementation hardening baseline

The native F/H/O stall logs and process samples from Addendum E have been copied
from temporary storage to [durable repository QA evidence](docs/qa/evidence/ux-hardening-2026-09-11/README.md).
Their checksums, source incident identifiers, `HEAD`, and intentionally-dirty
worktree baseline are recorded there. The samples remain evidence of a stalled
native interaction after successful worker publication; they do not establish a
root cause or identify a lock owner.

[The issue matrix](docs/qa/evidence/ux-hardening-2026-09-11/issue-matrix.md)
maps every original/C/D/E finding and the agreed hardening gaps to one of:
implemented earlier but not independently native-verified, unverified, or open.
It is the implementation ledger for the delegated hardening program. No finding
has been marked fixed by this baseline work, and all earlier observations remain
unchanged.

### Implementation checkpoint — delegated hardening in progress

This checkpoint records implementation evidence only; it does not replace the
native findings in Addendum E or certify the application for release.

- Terra's interactive-UI boundary and single-flight confirmation were accepted
  after Astra review caught and corrected an active-drag UI-release regression.
  The pointer/AX/Enter annulus-equivalence test is still owed in the native app.
- Terra's selection cleanup and palette ranking were accepted. Full authoritative
  projections now reconcile absent body/element/sketch/feature references, while
  partial deltas intentionally do not clear optimistic active-tool targets.
  Palette title/leading-verb intent outranks incidental history-keyword matches.
- Astra independently ran the region-pick, `SketchController.select`, picker,
  rebind-pick, document-store, and palette suites: **122 passed**. The baseline
  `npx tsc --noEmit` check passed. These are targeted checks, not a full gate.
- Native Sol tracing reproduced a two-thread deadlock: the main thread waits on
  `webviews_lock` while a regen emitter synchronously waits for main-thread
  evaluation receive. The fix is an in-progress unlocked-rebind ticket. Its
  initial check/clippy/fence-1 and feature guard passed; `cargo tree
  --all-features` found no runtime-wry tracing dependency.

Native stress has not run. Solver currentness, atomic transactions, the shared
tool-presentation seam, complete FeaturePattern contracts/implementation, UI
packages, persistence, physical-input coverage, and every other open row in the
issue matrix remain pending. No full-green, native-closure, or professional-CAD
readiness claim follows from this checkpoint.

### Further implementation checkpoint — evidence and open review

- Astra independently ran active-tool presentation, model-chip, dimension-input,
  and controller chamfer-angle, offset-face, and hole suites: **183 passed**.
  This is focused automated evidence only.
- Core `feature_pattern` currently has **4 passing negative-validation tests**.
  They do not demonstrate mixed-chain geometry. The first worker/core
  implementation is under adversarial review for partial transforms, provenance,
  and strict-validation gaps, and is not accepted as a supported capability.
- The 320 px inspector frame (bounded 280–420 px) and an Extrude secondary
  parameter seam are in progress. Legacy overflow migration, chip relocation or
  docking, and consistent adoption across all tools remain open.
- At the user's request, app PID 33374 and worker PID 33395 were stopped with
  TERM and verified absent. No native validation has occurred since; this is not
  evidence for or against the proposed native deadlock fix.

The solver, atomic transaction, FeaturePattern, shared-tool seam, UI migration,
native stress, persistence, and physical-input gates remain open. No overall
completion, native closure, or full-green claim is made.

### Further implementation checkpoint — targeted worker, runtime and Cancel evidence

- Worker build/stage Release passed. The worker `feature_pattern` CTest passed
  **1/1**, but its safety review found an unresolved producer fallback. The
  feature is therefore not accepted; this test is not proof of safe mixed-chain
  geometry or reference provenance.
- Five frontend atomic/solver suites passed **211**. Runtime
  `document_runtime::tests` passed **116** with `ONECAD_REQUIRE_WORKER=1`, but
  those tests use the fake runtime backend and must not be described as
  native-worker geometry evidence. Two Extrude suites passed **77**.
- Hole UI implementation reported **87 tests** and TypeScript success from its
  coding agent; Astra's independent rerun was still in progress at this
  checkpoint.
- The current atomic new-sketch Cancel attempt remains under review: partial
  `NetStep` rollback and a missing mutation notification are defects. Existing-
  sketch, new-sketch, and refused-discard rollback regressions remain open.

Solver/atomic-operation code may have focused passing tests, but native
validation, broader patterns, dragging, shared-tool UI, persistence, full gates,
and the original UX acceptance matrix remain incomplete. No commit was made.

### Further implementation checkpoint — reviewed focused progress

- Atomic new-sketch Cancel's prior partial-rollback and missing-mutation defects
  were fixed and reviewed, including readonly foreign-gesture coverage. Astra
  independently ran `cargo --lib cancel_`: **11/11**, mock-client/sketch IPC/
  `SketchController.exit`: **157/157**, and latest select/exit/plane-pick/mock
  sketch coverage: **56/56**. These do not replace native rollback acceptance.
- Core FeaturePattern tests are **8/8** and wire tests **1/1**. The current
  worker CTest is **3/3**, limited to Hole→Chamfer plus malformed and repair
  cases. This is real worker/OCCT coverage for those bounded cases, unlike the
  fake-backend runtime tests recorded above; it is not proof of general mixed-
  chain adapters, transforms, provenance, or persistence.
- UI chips/Hole/presentation/Inspector focused suites were **117/117** before
  the current drawer refactor. All tool families now render through the
  inspector frame, but drawer-quality review, legacy-overflow migration, and
  geometry-aware chip relocation/docking are unfinished.
- The initial mesh-coalescing change was rejected in review for stale-install
  and ABA hazards. It requires a runtime/epoch backend ticket and explicit
  stale-generation regressions before it can support a responsiveness claim.

The app remains closed at the user's request. No final integration gates or
native acceptance ran after these changes; native stalls, pointer equivalence,
complete FeaturePattern breadth, tool placement, persistence, physical controls,
and all remaining UX matrix rows are still open.

### Further implementation checkpoint — drawer acceptance and new guards

- Astra independently reran chips, Hole, active-tool presentation, and
  Inspector suites: **123/123**, with TypeScript clean. Drawer and CountStepper
  bounded code/focused tests were accepted. An intermediate **120 pass / 1
  fail** stale-axis-label expectation was repaired before the current run; only
  the current 123-pass result is acceptance evidence. Native UI validation is
  still required.
- Rust strict FeaturePattern repair-parser coverage passed **1/1**. This is a
  narrow parser result and does not broaden the restricted FeaturePattern
  acceptance recorded above.
- Two additional risks are confirmed open and sequenced to Sol: `AcquireElementIds`
  can fall back to nearest-anchor recovery when an explicit TopoKey is invalid,
  which must become fail-closed; and Cancel/tool switching can lose published
  feedback while an old non-preview commit can reset a newly selected tool.

No final integration gate or native validation ran. The app remains closed, and
all prior release-blocking native, pattern-breadth, persistence and interaction
acceptance requirements remain unchanged.

### Further implementation checkpoint — worker provenance and scoped transport evidence

- The current worker Release build/stage passed. The built artifact uses actual
  OCCT **8.0.1** with pinned metadata `onecad-occt-8.0.1-b8f597c67781-kp1`;
  the local cache override was independently verified correct. C++
  deprecation/initializer warnings remain, so this is not a warning-free claim.
- Astra freshly ran CTest **5/5** in 1.39 seconds: `feature_pattern`,
  `element_identity_gate`, `harness_tessellate_acquire`, and canonical malformed
  and repair FeaturePattern fixtures. It is real worker/OCCT evidence only for
  that bounded scope. Pattern support remains restricted to same-host
  Hole→Chamfer and independently-child Sketch→Blind/NewBody/one-direction
  Extrude→straight Fillet; general mixed-chain patterns are not accepted.
- Independent frontend/transport checks include meshSync/ViewportEngine/
  tauriClient **209/209**, then meshSync **38/38** including acknowledgement
  rejection, and native-API watchdog/retirement **29/29**. Delayed-mesh **4/4**
  uses a fake delayed provider, not native OCCT. Explicit-invalid-key
  fail-closed acquisition, finite-anchor uniqueness, shared-topology dedupe,
  and producer-only pattern binding have focused automated coverage, still
  requiring native validation.
- Tool lifecycle remains open: a detached submitted promise must not falsely
  report failure. Sol is implementing that edge; existing results do not close
  feedback sequencing or render-completion correctness.

The app remains closed. No native stress or acceptance pass ran, and no final
integration verdict follows from these focused checks.

### Further implementation checkpoint — integration verifier boundary

`verify-modeling-contracts.mjs` passed with **39 rows, 18 operations and 15
tier-checked**. `verify-modeling-coverage.mjs` failed: both `KnownOperation`
and worker dispatch know `FeaturePattern`, but the coverage manifest has no
FeaturePattern row. This is an integration defect, not a fixed result. The
chained tracing guard did not execute after the coverage failure and is not
credited. No manifest change was made at this checkpoint.

Correction to the preceding verifier boundary: the chained tracing guard did
not run because coverage failed, as recorded. It was later run separately and
passed with runtime-wry tracing disabled. That separate pass does not turn the
earlier failed coverage integration into a general FeaturePattern acceptance.

### Further implementation checkpoint — bounded manifest and navigation evidence

Two deferred/hidden FeaturePattern manifest rows now represent only the actual
worker adapters: same-host Hole→Chamfer and independently-child Sketch→Blind
NewBody Extrude→straight Fillet. The manifest deliberately has no typed Rust,
frontend, or browser evidence for either row: typed Rust integration remains in
progress. The current verifiers passed: coverage **34 rows, 9 corpus cases, 20
registry operations**; contracts **41 rows, 19 operations, 15 tier-checked**.
This repairs registry tracking; the prior five bounded worker tests do not prove
general FeaturePattern support.

Astra also reviewed bounded SectionControls/Nav changes and independently ran
LayersMenu/NavPill **15/15**. Numeric precision, invalid alerts, shared state,
and navigation mapping were accepted in that limited scope. Shared-popover
focus/clamp behavior and native UI validation remain open. No native run, full
integration gate, or overall acceptance claim follows.

### Further implementation checkpoint — source-edit regeneration defect

The two deferred FeaturePattern contract rows were corrected to state that
producer binding is checked during nested execution and rejects the candidate
before publication; they no longer imply all invalid bindings are refused before
execution. The independent-child body policy now distinguishes the wire
child-body key (`body_<pattern-opId>:<instance-1>`) from the core canonical
persisted identity `split_child_uuid(pattern,k)`.

Independent real integration test `cargo feature_pattern_integration`
`independent_pattern_survives_edit_history_and_reopen` currently fails **0/1**.
Its first three body/identity-metrology checks pass, but changing source Extrude
from 10 to 16 leaves only the source and yields `FEATURE_PATTERN_PRODUCER_BIND`
at instance 1 Fillet input 0. Native_plan owns the correction and tools_plan
owns the test. The failure occurs during source-edit regeneration; its reopen
assertions were not reached. It is active, not ignored, and blocks any reopen
or general FeaturePattern acceptance.

### Further implementation checkpoint — failed-model rollback safety

Automatic failed-model rollback currently uses generic `client.undo`. The native
API acquires its runtime when the invocation executes, so a queued rollback can
target a replacement document or an interleaved top transaction despite frontend
result guards. This is a transaction-correctness defect, not a UI-only race.

Native_fix is implementing a backend-generated rollback receipt bound to the
runtime instance, exact undo entry, and revision, with a conditional rollback
command. There must be no blind Undo fallback: a null/no-op result without a
receipt must never undo prior work. Review has already identified premature
unlock, post-result dispose, rollback-error wording, direct-history re-edit and
popup-key paths for the integrated design. Frontend changes await the backend
seam and main independent runs. The 810 UI-lifecycle subagent tests are not
credited as a main gate. E1 native stalls, native acceptance, and all remaining
release-blocking gates stay open.

### Further implementation checkpoint — bounded integration passes and remaining guards

Main independently ran real-worker `feature_pattern_integration`: **2/2** passed
for same-host Hole→asymmetric-Chamfer and independently-child
Sketch/Extrude/Fillet source edits, history, raw persistence, and fresh-worker
reopen. Selected CTest passed **11/11**, including canonical malformed and
repair fixtures. This is bounded-adapter evidence only; it does not establish
general FeaturePattern support or native UI acceptance.

Rollback IPC/tauriClient plus Picker/ViewportEngine focused suites passed
**211** across four files. Main also ran **812** model-tool tests across 45
files before later review corrections; shared Popover/Nav/Layers/shortcuts had
earlier passed **81**. None is a final stable-tree integration gate.

Two review gaps remain active: the backend receipt must not arm for a no-op
transaction or a previous undo top, and numeric validation can currently reset
the controller retained-failure block. The receipt cargo test encountered an
active-edit signature mismatch, so it is not credited as a stable-tree run. The
failure-preservation path requires an independent typed validation block. The
native app remains closed and no native acceptance ran.

### Further implementation checkpoint — focused reruns, browser boundary and session fencing

Main independently ran rollback-receipt focused tests **5/5**, same-document
replacement **1/1**, TypeScript, and frontend **145**. Native_fix subsequently
found and corrected a depth-only undo-cap issue; the final independent rerun is
still owed. This is not a rollback-transaction closure.

The latest focused history/Inspector/chips/presentation/edgeShell run passed
**217/217**. Earlier, model-tools/chips passed **889** tests across 46 files,
but an Inspector suite failed to load while Terra was editing a HistoryList parse
path. Accordingly there is no integrated UI-green claim. A native screenshot
also reproduced an E5 regression: the old hard-coded 264 px
CornerCluster/GridScaleChip inset overlays the new 320 px Inspector. A shared
inset correction is underway.

Browser evidence remains deliberately split. The initial 48-test run was fully
blocked at browser launch (Chromium Mach bootstrap permission and WebKit abort;
provenance `2026-09-11T19-47-10-301Z`). A max-fail-1 retry produced one pass,
one failure and 46 not run, isolating a collision between history buttons named
only `Extrude` and toolbar labels. The exact regression later passed **2/2** on
Chromium and WebKit with retries disabled. That fixes evidence for the narrow
collision only; it does not make the broad browser workflow green.

A new cross-session pick risk is proven: `SnapshotPublisher` resets its counters
on reopen, allowing the same `(docId, snapshot, generation)` to recur across
sessions. Native_fix is adding authoritative `runtimeSession` fencing to
snapshot/projection/change events, bootstrap replay, and backend mesh/promote
expected tokens. Candidate enumeration has been implemented and separately
reviewed, while chooser UI and full normal-selection behavior remain open. C3 is
also extending the bounded FeaturePattern chain from 0 through N instances, but
is not accepted. The application remains closed and all native acceptance is
pending.

### Further implementation checkpoint — evidence correction

The E5 CornerCluster/GridScaleChip-over-Inspector screenshot in the preceding
checkpoint was a **Playwright mock-browser** screenshot, not a native screenshot.
It remains valid browser-lane evidence for the hard-coded 264 px versus 320 px
layout regression, but it provides no native UI acceptance evidence. The exact
history-accessibility regression still passed **2/2** across Chromium and WebKit
with retries disabled; the available last-run metadata records only `passed` and
does not provide a trustworthy timestamp, so no `2026-09-11T19-54-44` run ID is
claimed. The app remains closed and native acceptance remains pending.

### Further implementation checkpoint — bounded worker and measured shell evidence

Main independently passed FeaturePattern core **9/9**, selected CTest **3/3**,
and real-worker integration **2/2** spanning four sources. This remains
bounded-adapter evidence; C3's 0-through-N instance extension is not accepted.

Inspector store/component tests passed **44/44**. Chromium measured nine layouts
across three window sizes and 280/320/420 px Inspector widths, plus collapsed 32;
the cube/grid clearance measured 12 px. The mock-browser screenshot
`/tmp/onecad-layout-1024-420.png` still shows the toolbar crossing the Inspector
header/cube and model clipping. ui_plan is implementing a measured-work-area
fix.

The approved user-visible golden-shell contract uses `display: contents`
measurement wrappers around shell SlotHosts. They preserve layout/order while
deriving the actual occupied panel, toolbar and corner rectangles; toolbar wraps
instead of horizontal scrolling. A legacy `undefined === undefined` currentness
defect is reported fixed by an agent with 38 tests, but is preliminary rather
than main evidence, and accessibility/error-wording correction remains active.
No native or full-gate claim follows; the app stays closed.

### Further implementation checkpoint — jsdom measurement baseline

The test harness now supplies an inert `ResizeObserver` only when jsdom lacks
one, exposing the normal `observe`, `unobserve`, and `disconnect` shape. It does
not alter production measurement or timeouts; lifecycle tests retain explicit
custom observers. The narrow StartScreen suite passed **20/20**.

Before this test-infrastructure correction, the main four-target run recorded
**61 pass / 19 fail**: three StartScreen failures and 12 uncaught missing
`ResizeObserver` errors. The other 16 failures are promotion failures assigned
to native audit. The four-target rerun is still due after the active heavy lane;
this is not a full-suite or native acceptance claim.

### Allocation and evidence correction — 2026-09-11 (append-only)

Ownership is explicit: Luna handles simple fixes, test updates and docs; Terra handles moderate UI; Sol handles complex concurrency/geometry; main Astra reviews and accepts only; maximum three coding streams.

Main independent evidence records promoter + StartScreen six suites **240/240**, native receipt **6/6** including the cap, currentness three suites **55/55**, chooser + ViewportRoot + Popover three suites **32/32** before browser work. Main also measured bounded Revolve core **9/9** plus real-worker **4/4**. Mounted layout was checked at 1024/1156/1440 with Inspector 420, toolbar wrapping and corner clearance; `/tmp/onecad-mounted-layout-1024.png` is a MOCK screenshot, not native evidence.

The interrupted full-unit history (**61 pass / 19 fail**) remains historical and does not establish a stable full gate or native claim. A26 tests and shared Add5 integration are agent-preliminary only. The chooser browser did not open at 430×400; diagnostics remain pending, so no browser or native pass is claimed.

### Continuation evidence — 2026-09-11 (append-only)

Main independently passed **7 files / 182 tests**: mock publication/import/promote, chips, HtmlOverlayDriver, ViewportRoot and ViewportEngine, all before the latest drag changes. Worker Release build/stage passed with OCCT deprecation warnings; `ctest -R feature_pattern` passed **3/3**; real-worker `feature_pattern_integration` passed **5/5**, including shared Add5 on the corrected enum/host ledger.

Chooser remains blocked/defective: the publication is current, but the installed mesh has undefined provenance; the exact candidate entry is present and candidates are filtered. The revision-fixture mismatch is fixed, but is not proven as the root cause. Native app was not launched and full gates are not claimed. Latest drag agent **83 passed** and dev-demo **2 passed** are preliminary only. Native event/restart review remains rejected pending ABA, queued-restart, `getProjection` lifecycle/order, and coalescer corrections.

### Final hardening checkpoint — 2026-09-11 (append-only)

Allocation remains Luna=simple fixes/test updates/docs, Terra=moderate UI, Sol=complex concurrency/geometry, Astra=review/acceptance only, maximum three coding streams. Main independently verified promoter + StartScreen **240/240**, native receipt **6/6** including cap, currentness **55/55**, chooser + ViewportRoot + Popover **32/32**, bounded Revolve core **9/9**, real-worker **4/4**, six frontend files **230 tests**, three UI suites **129 tests**, and fresh chooser Chromium + WebKit **2/2** retries 0 after stale-server restart. Worker patterns and fence checks passed as recorded; full gates remain open and native app was not relaunched. Preserve **61 pass / 19 fail** history, preliminary A26/shared Add5 labels, initial mock provenance observation, and separate Cut/Rust evidence. No native acceptance claim.

### Final main gate evidence — 2026-09-11 (append-only)

Full CTest passed **196/196** ([durable log](docs/qa/evidence/ux-hardening-2026-09-11/full-ctest.log); source `/tmp/onecad-ux-full-ctest.log`). The prior full Vitest RED was **5869 pass / 9 fail / 78 skipped** (**5956 total**); all nine stale-contract/async-cleanup failures are fixed tests-only. Current frozen-source main rerun passed **332 files / 5880 passed / 78 skipped / 5958 total** in **31.36s** ([durable log](docs/qa/evidence/ux-hardening-2026-09-11/full-vitest-final.log); source `/tmp/onecad-ux-full-vitest-final.log`); main `npx tsc` passed. Final clippy was running and is not claimed. Latest subscriber-failure isolation is covered by full unit. Native GUI was not relaunched; full browser and full Rust workspace remain owed. General pattern breadth/UI, typed-target and D-camera remain open. Luna remains on simple tasks/docs, Sol on complex work, Astra final review; no percentage bump.

Main subsequently passed `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` (49.21s; [durable log](docs/qa/evidence/ux-hardening-2026-09-11/final-clippy.log); source `/tmp/onecad-ux-final-clippy.log`). `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` is still running; no result claimed here.

The worker-required full workspace run then recorded **475 pass / 1 fail** in the first `lib` target; [durable RED log](docs/qa/evidence/ux-hardening-2026-09-11/final-cargo-workspace-red.log). The bounded DTO correction is separately green: `ONECAD_REQUIRE_WORKER=1 cargo test --lib dto::tests` **22/22**.

Final worker-required Rust rerun exited 0: **1646 passed / 0 failed / 0 ignored / 0 filtered** across exactly 100 result lines, with no missing-worker skips ([durable log](docs/qa/evidence/ux-hardening-2026-09-11/final-cargo-workspace-rerun.log); source `/tmp/onecad-ux-final-cargo-workspace-rerun.log`). The earlier RED remains historical. Post-DTO fmt/clippy rerun status is not newly claimed; full browser, native GUI and remaining implementations remain open.

Post-DTO main `cargo fmt --all --check` and warning-free workspace clippy passed in **44.48s** ([durable log](docs/qa/evidence/ux-hardening-2026-09-11/final-clippy-after-dto.log); source `/tmp/onecad-ux-final-clippy-after-dto.log`). The final build remains in progress and is not claimed.

Final `bun run build` passed (tsc + Vite, 2.03s) with the existing >500kB chunk warning ([durable log](docs/qa/evidence/ux-hardening-2026-09-11/final-build.log); source `/tmp/onecad-ux-final-build.log`). Native remained closed.

### Main engineering review checkpoint — 2026-09-12 (append-only)

Implementation evidence and native validation remain separate. Main independently recorded UI + camera **11 files / 203 tests passed** with `npx tsc`, Stage B camera-engine **4 files / 101 tests passed** with `npx tsc` (`PreviewMesh.test` was nonexistent and excluded), required-worker `feature_pattern_integration` **7 passed / 0 failed / 0 ignored / 0 filtered**, selected CTest **13/13**, and full topology rebind **16 passed / 0 failed / 0 ignored / 0 filtered**. These are automated engineering checks, not a manual/native acceptance pass.

Critical open finding: `vfm5_teleport_on_the_ordinary_edit_lane_is_the_accepted_residual` explicitly asserts `boundDecoy=true`; its passing result is characterization evidence, not safety acceptance. General patterns remain bounded; authoritative producer ledger/adapters are in progress, with no producer-to-host fallback. Tree hover/Reveal and current typed-target checks are automated evidence only.

New open measurement-currentness finding: mass caching is keyed by `bodyId`; main code inspection identifies same-body regeneration/new-document async stale-result risk. This is not a native reproduction. The current app is stale/closed, native was not launched, and full integrated gates were not rerun. No percentage or all-fixed claim is added.

### Subsequent engineering checkpoint — 2026-09-12 (append-only)

Resolver v6 removes the post-edit stale-anchor exception. Main worker-required topology tests passed **16/16**, now including a fail-closed teleport regression (`NeedsRepair`, zero removed volume, `boundDecoy=false`). Clean rebuilds use no-edit provenance; real edits retain their actual dirty floor. Main FeaturePattern integration passed **7/7** after restoring Revolve producer history. Selected worker ladder/origin/pattern/canonical-fixture checks passed **5/5**. Broader mixed-pattern support remains in progress, not accepted by these bounded cases.

Main camera/sketch-entry tests passed **13 files / 184 tests**; a subsequently corrected test-only TypeScript fixture was independently checked with `npx tsc --noEmit` and plane-pick **9/9**. Explicit Fit Preview plus target/presentation tests passed **3 files / 25 tests**, followed by TypeScript. These do not prove physical navigation, target clearance or native window behavior.

Review found an additional readiness race: checking only document identity, or tagging a stale request only at driver start, can let old work supersede a replacement runtime's regeneration. The accepted correction checks the runtime session and enqueues under one lock, retaining the driver tag. Main readiness **7/7** and the corrected actual same-file reopen regression **1/1** passed. Earlier no-op callback tests were rejected as evidence; the final replacement tests use the production enqueue helper.

Measurement currentness, annotation pin/hide/collision layout, general-pattern breadth, explicit planar-face/datum framing, independent responsiveness monitoring and full native/integrated acceptance remain open. No new native pass or completion percentage is claimed.
