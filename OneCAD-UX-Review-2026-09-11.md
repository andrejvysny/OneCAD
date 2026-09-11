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
