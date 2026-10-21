# Native control availability

This source-coupled inventory implements ONECAD-304 and ONECAD-314. Plane owns the
implementation scope and workflow acceptance. `tool_registry.rs` owns placement;
`pill::view` owns context applicability and `pill::activate` rechecks that same
availability before dispatch. The overflow uses these same functions. Neither
backend operation records nor visible disabled controls establish native delivery.

The readiness columns separate existing code, connected native action, automated
coverage, and native workflow acceptance. “Not qualified” means no new native
acceptance evidence is supplied by this inventory. Automated coverage references
executable tests, not a claim that an unrun gate passed.

| Control | Placement / visibility | Code and real route | Availability / owner | Automated coverage | Native accepted |
|---|---|---|---|---|---|
| Select; Point, Line, Rectangle, Circle, Arc, Slot | Stable sketch pill, including selected geometry | `SketchAction::SetTool`; existing stage previews | Live sketch admission; ONECAD-314 | `tool_registry`, `pill::layout` | Not qualified |
| Dimension; Horizontal, Vertical, Fix | Sketch pill | `constraint_authoring::activate` | Existing typed operand applicability; ONECAD-286 / 285 | Constraint authoring and dimension delivery suites | Not qualified |
| Constraints; Parallel, Perpendicular, Equal, Midpoint, Distance, Diameter and remaining relations | Full Constraints menu | `activate_relation` / existing `SketchAction` mutations | Existing typed operand applicability; ONECAD-286 | `constraints_menu`, constraint authoring suites | Not qualified |
| Construction, Delete | Selected sketch pill | `sketch_repair` | Existing repair/selection checks; ONECAD-314 | Repair suites, pill layout | Not qualified |
| Delete body/sketch | Explorer context menu; Delete / Backspace | `plan_delete_document_item` / `delete_document_item` | Idle real document; confirms authoritative producer/dependency closure; text owns editing keys | `document_item_deletion`, `item_delivery_tests`; strict host inventory | Native CGEvent menu/key/history/save/reopen checked; physical input open |
| Polygon, Ellipse | Sketch overflow, disabled | No native creation route | ONECAD-292 | Registry icon/disposition checks | No |
| Trim, Extend | Sketch overflow, disabled | No native modification route | ONECAD-293 | Registry disposition checks | No |
| Sketch Fillet, Offset, Mirror | Sketch overflow, disabled | No native modification route | ONECAD-294 | Registry disposition checks | No |
| Project; Spline | Sketch overflow, disabled | No native creation route | ONECAD-296 / 298 | Registry disposition checks | No |
| Sketch support | Modeling pill | `choose_sketch_support` | Real document, idle, live worker and viewport, no section; ONECAD-295 | Support selection suites | Not qualified |
| Extrude | Profile pill and model overflow | `Action::BeginExtrude` | Closed profile, idle, live worker and viewport, no section; ONECAD-273 | Existing profile/plate delivery suites | Not qualified |
| Move / Rotate | Body pill and model overflow | `begin_transform` | Selected body, idle, live worker and viewport, no section; ONECAD-273 | Transform suites | Not qualified |
| Saved feature Edit | Feature pill / existing history editor | `inspector_history::edit_selected_feature` | Existing numeric editable parameters, live modeling context; independent of new-feature availability | History editor delivery suite | Not qualified |
| Revolve; Fillet; Chamfer; Shell; Hole; Offset Face | Model overflow, disabled for creation | Backend records exist; native operand/preview creation flows incomplete | ONECAD-220 / 316 / 316 / 317 / 318 / 319 (operands: ONECAD-315) | Registry disposition checks; existing saved-edit paths retained | No creation qualification |
| Boolean | Model overflow, disabled | Backend exists; native target/tool preview absent | ONECAD-253 | Registry disposition checks | No |
| Linear/Circular body pattern; Feature pattern | Model overflow, disabled | Backend exists; native guard / expansion incomplete | ONECAD-221 / 222 / 29 | Registry disposition checks | No |
| Sweep, Loft, Datum plane, Offset plane, Generators, Draft, Material | Model overflow, disabled | No connected native creation route | ONECAD-314 inventory; owning follow-on scope remains in Plane | Registry disposition checks | No |
| Measure | Model overflow, disabled | Native measurement absent | ONECAD-245 | Registry disposition checks | No |
| Isolate, Hide body | Nav / model overflow, disabled | Temporary/persistent native visibility absent | ONECAD-320 | Registry disposition checks | No |
| Roll back, Suppress, Duplicate | Model overflow, disabled | No connected native history route | ONECAD-321 | Registry disposition checks | No |
| Import | File menu, native menu, model pill | Existing `document_request(Request::Import)`; opens the file as a new document tab | Existing replacement/confirmation policy; ONECAD-314 | File lifecycle tests | Not qualified |
| Insert STEP | Native File menu, after Import | `document_request(Request::InsertStep)` → `api::insert_step`; adds an `ImportStep` record at the rollback cursor of the active document, no new tab | Open document, idle editor (no sketch, tool or busy command); refuses if the document changes before commit; owner pending, related ONECAD-428 / 314 | `step_insert` real-worker suite; `document_io_tests` insert routing | Not qualified |
| Section | Nav, model pill, Display / More settings | Existing `viewport_options::contents` section controls | Live viewport; section mutation disabled during sketch/model tool; ONECAD-314 | Display popup interaction tests | Not qualified |
| Home view, Fit | Nav and native bindings | Existing camera / fit request | Live viewport | Existing input/camera suites | Not qualified |
| Snap | View cluster | Existing `sketch_snap` transient preference | Ready sketch | Existing snapping suites | Not qualified |
| Grid | View cluster | Support-grid renderer visibility | See ONECAD-309 implementation; no fake layer action | Support-grid suites | Not qualified |
| Layers | Nav, disabled | Gallery-only state | Native layer controls unavailable; ONECAD-314 | Source inspection | No |
| Search commands | File menu, disabled; no shortcut hint | No native route | ONECAD-277 | Source inspection | No |
| Shaded, Shaded with edges, Wireframe; X-ray | Display menu | Existing renderer options | X-ray requires `FLOAT32_BLENDABLE` | `viewport_options` interaction tests | Not qualified |
| Hidden line | Display menu, disabled | No renderer implementation; never aliases X-ray | ONECAD-314 | Display tests | No |
| Library, Extensions, workspace destinations | Home / workspace navigation, disabled where no real route | Production facade gate stays off | ONECAD-300–303 / 92 | Home/source checks | No |
| Insert / Make component; Flange, Dogbone, Check geometry | Model overflow, disabled | Gallery-only routes remain isolated | ONECAD-300–303 / 92 | Registry checks; production dispatch guard | No |
| Activity | Persistent compact affordance, Home and editor | Bounded typed `Diagnostics`, expandable selectable/copyable detail | Preserves runtime and sketch-visit attribution; presentation dismissal does not erase errors; ONECAD-280 | Actual egui-rendered outcomes and popup interaction tests | Not qualified |

Toolbar shortcut hints are omitted because those tool keys are not registered.
File/history and native camera menu hints retain their real registered bindings.
The workspace popup advertises no unbound shortcuts. At small widths the tool pill
pins Select, the active tool, Point and More; remaining tools use a bounded overflow popup. No original design-screen
fidelity claim is made without the missing authoritative design artifacts.
