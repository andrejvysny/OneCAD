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
| Sketch support | Modeling pill; resolved datum chooser and Explorer datum rows | `choose_sketch_support`; `new_sketch_on_datum` | Named world plane, exact planar body face or resolved datum; real document, idle, live worker and viewport, no section; ONECAD-295 | Support selection suites; `datum_ui::tests` | Not qualified |
| Extrude | Profile pill and model overflow | `Action::BeginExtrude` | Closed profile, idle, live worker and viewport, no section; ONECAD-273 | Existing profile/plate delivery suites | Not qualified |
| Move / Rotate | Body pill and model overflow | `begin_transform` | Selected body, idle, live worker and viewport, no section; ONECAD-273 | Transform suites | Not qualified |
| Saved feature Edit | Feature pill / existing history editor | `inspector_history::edit_selected_feature`; shared feature card and full-prefix replay preview | Numeric editable parameters and Boolean, Mirror body or Loft choices; live modeling context; independent of new-feature availability | History editor delivery suite; `loft` interaction tests | Not qualified |
| Revolve | Model overflow | `operation_start::begin_revolve`; sketch-line axis chooser or staged body-edge axis | Finalized profile with a sketch line, or profile plus axis edge; live modeling context; ONECAD-220 / 315 | `real_modeling_creation`, `revolve_ui` | Not qualified |
| Fillet; Chamfer; Offset Face | Edge / face pill and model overflow | `operation_start::begin_edge_op` / `begin_offset_face`; prepared closure, shared preview and commit | Edges or faces of one body; live modeling context; ONECAD-316 / 319 / 315 | `real_edge_ops`; `model_operation_delivery_tests` | Not qualified |
| Shell; Hole | Model overflow | `operation_start::begin_shell` / `begin_hole`; shared feature card, preview and commit | Removal faces of one body for Shell; one planar face for Hole; live modeling context; ONECAD-317 / 318 / 315 | `real_modeling_creation`, modeling options tests | Not qualified |
| Boolean | Model overflow | `modeling_ui::begin_body_operation`; target/tool chooser, union/subtract/intersect and keep tools | Ordered selection of target plus tool bodies; live modeling context; ONECAD-253 | `real_modeling_creation`, modeling options tests | Not qualified |
| Linear/Circular body pattern; Mirror body | Model overflow | `modeling_ui::begin_body_operation`; shared choice controls, preview and commit | One visible healthy source body; live modeling context; ONECAD-221 / 222 / 314 | `real_modeling_creation`, modeling options tests | Not qualified |
| Loft | Model overflow | `loft_ui::begin`; smooth/ruled solid New Body; editable ordered sections; shared preview and commit | 2–32 finalized closed regions without holes on parallel, distinct planes in forward or reverse offset order; ONECAD-385 / 315 | `loft_ops` real-worker lifecycle; `loft` interaction tests; `loft_ui::tests` | Not qualified |
| Datum plane; Offset plane | Model overflow and Sketch support chooser | Both commands open `choose_offset_plane`; `datum_ui` creates an offset datum through shared application services | XY/XZ/YZ base and finite signed offset; frozen resolved sketch support; live modeling context | `datum_ui::tests`, sketch entry delivery suites | Not qualified |
| Gear generator | Model overflow | `modeling_ui::begin_body_operation`; spur gear recipe in a world frame | No geometry selection required; module, height, pressure angle, tooth count, world center and optional bores; live modeling context | `real_modeling_creation`, modeling options tests | Not qualified |
| Sweep; Feature pattern | Model overflow, disabled | Sweep has no worker executor; FeaturePattern preview lacks authoritative source expansion | ONECAD-314 / 29 | Registry disposition checks; worker refusal coverage | No |
| Draft; Material | Model overflow, disabled | No connected native creation route | ONECAD-314 inventory; owning follow-on scope remains in Plane | Registry disposition checks | No |
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
| Library: browse, search, category/source filter, sort, detail, Try parameters (validated draft, Preview), thumbnails, Reindex | Start sidebar "Component library" and source sub-items; Start page | `library_catalog` lane -> `search_library_components`, `component_detail`, `reindex_library`, `component_preview_mesh`; ISO 4762 table from `onecad_library::tables` | Real catalog from the persisted library index; Preview needs a running geometry worker and refuses with its error otherwise; independent of `PREVIEW_FACADES` | `library_catalog::tests`, `library_pages::tests`, `library_delivery_tests` (worker and Metal gated) | Not qualified |
| Library: Insert into open model; model pill Insert component | Library detail and ISO table pages; pill opens the library page | Insert arms `placement_gesture`: `PrepareDraft` on the Library lane, then a `PlaceComponent` operation in the shared reducer. Hover snaps to faces, holes and rims through the `ClassifyTarget` lane (ground plane otherwise) with the worker-resolved ghost; click or Enter places one `PlaceComponent` (Linked) or `DetachComponent` (Copy), Shift+click repeats, `A` flips, Tab cycles attachments, Esc cancels | Enabled only with an open editor document and a valid parameter draft; the hover reason names what is missing | `placement_gesture::tests`; `plate_delivery_tests::placement` (worker and Metal, synthetic input); `library_pages::tests` | No: real route, not physically qualified |
| Library: New component; Edit source; Duplicate | Library detail page, disabled with a hover reason | No native route | Authoring controls name their absence | `library_pages::tests` (no success text painted) | No |
| Component instances: Explorer COMPONENTS rows (Linked / Copy and status badges), Inspector instance panel with parameter edit, per-key and all Reset | Explorer section; Inspector Properties for a selected instance | `component_instances` lane -> `component_instances`, `set_component_params`, `reset_component_overrides`; commit sends the full effective map, one undo step | Idle real document; the library must still serve the definition to edit; refused values show the draft rules, never a partial send | `component_instances::tests`, `plate_delivery_tests::instances` (worker and Metal gated) | Not qualified |
| Component instances: Detach (confirmation), Review updates / Update selected / Update all applicable | Inspector buttons; Explorer section action; review dialog | `detach_component`, `plan_component_upgrades`, `apply_component_upgrades` (atomic, one undo, stale plan re-planned) | Detach keeps geometry as a Copy; dropped overrides and mate attachments are shown before approval; before/after geometry preview is not available | Same suites | Not qualified |
| Component instances: Edit placement | Inspector, disabled | No native route; re-arming the placement gesture on an existing record is a later step | "Re-placing an existing instance arrives in a later step" | Source inspection | No |
| Create Component: Make component (pill) and New component (library) on the selected body; Static / Parametric, details, exposed variables and configurations, attachments picked on the body, save | Modal wizard over the editor; pill More tools and the library header | `component_wizard` lane -> `attachment_from_pick`, `save_as_component` on the ordered service actor; catalog reindexes after a save | Needs an open model and one selected body; client-side id/version/configuration checks mirror the library; multi-solid offers Union solids; a taken id@version asks for a version bump; no thumbnail is captured | `component_wizard::tests`, `plate_delivery_tests::wizard` (worker and Metal gated) | Not qualified |
| Extensions, workspace destinations | Home / workspace navigation, disabled where no real route | Production facade gate stays off | ONECAD-300–303 / 92 | Home/source checks | No |
| Insert / Make component; Flange, Dogbone, Check geometry | Model overflow, disabled | Gallery-only routes remain isolated | ONECAD-300–303 / 92 | Registry checks; production dispatch guard | No |
| Activity | Persistent compact affordance, Home and editor | Bounded typed `Diagnostics`, expandable selectable/copyable detail | Preserves runtime and sketch-visit attribution; presentation dismissal does not erase errors; ONECAD-280 | Actual egui-rendered outcomes and popup interaction tests | Not qualified |

The connected modeling routes above expose bounded controls, not every backend
mode. Loft refuses holes, open curves, point or face sections, guide rails, end
conditions and Boolean result modes. To-face/two-direction extrusion, asymmetric
chamfer reference-face picking, total-wall face offsets, multipoint/threaded hole
authoring and advanced datum tooling still require additional native controls.
Gear creation does not expose face seating or helical geometry. See the
[native modeling acceptance procedure](../../qa/native-modeling-acceptance.md)
for exact scenarios, recoverable repair cases and remaining physical macOS/iPad
mouse, touch and Pencil qualification, including compact layouts. Connected
routes and automated tests do not establish that physical acceptance passed.

Toolbar shortcut hints are omitted because those tool keys are not registered.
File/history and native camera menu hints retain their real registered bindings.
The workspace popup advertises no unbound shortcuts. At small widths the tool pill
pins Select, the active tool, Point and More; remaining tools use a bounded overflow popup. No original design-screen
fidelity claim is made without the missing authoritative design artifacts.
