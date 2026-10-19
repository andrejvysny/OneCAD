# Handoff: OneCAD editor shell, selection-driven toolbar & core modeling flow

Target repo: `andrejvysny/OneCAD` (branch `master`) — native Rust, egui/eframe,
`onecad-widgets`, shared `onecad-render`, and the production OCCT/PlaneGCS worker.

## Native implementation guidance

The shell and widget library are published in Git. Use the HTML screens and
component sheet as visual references, not application code or evidence of working
features. The former React/Zustand/Three.js/Tauri instructions are retired.

- Keep presentation in `crates/onecad-widgets`; compose it with application data
  in `crates/onecad-ui`. Use typed `onecad-interaction` actions and shared
  `onecad-app` services for authoring. Only `onecad-app` mutates documents.
- Native theme assets live in `assets/native`; baked SVG masters and the widget
  texture cache implement logical-size master selection and physical rasterization.
- `onecad-render` uses the host's shared wgpu device and queue. Mock scene scripts
  do not replace real worker geometry, checked MESH1, or persistent references.
- Run Rust widget/interaction/host tests and required real-worker tests. Native
  macOS input, IME, Retina resize, rendering and shutdown need actual acceptance;
  screenshots and backend success alone do not establish it.
- Follow root `AGENTS.md`, the [native host guide](../../../../crates/onecad-ui/README.md)
  and [workflow contract](../native-workflow.md). Plane owns implementation status
  and future work. Disabled tools and illustrative workspaces in these mocks do
  not establish working Tauri parity or authorize new feature scope.

## Overview
OneCAD keeps one editor shell in every workspace (Design, Assembly, Simulation, Drawing, extension workspaces). The viewport is full-bleed; Explorer and Inspector float above it and can be fully hidden. There is **one toolbar pill** whose contents follow the selection: with nothing selected it shows only actions that need no selection; when something is selected it shows only the actions that apply to it, behind a removable selection label.

## About the design files
The files in this folder are **design references created in HTML** — prototypes
showing intended look and behaviour, not production code. Recreate their intent
with existing native widgets and theme tokens. The `.dc.html` files open directly
in a browser (they load `support.js`, `cad-icons.js`, `cad-scene.js` from the same
folder). `cad-scene.js` draws a 2D bracket diagram for the mocks only; the real
viewport uses `onecad-render`.

## Fidelity
**High-fidelity.** Colours, type, spacing, radii, shadows and states are final. Match them exactly using tokens.

## Files
- `OneCAD Final.dc.html` — final screens, ids in brackets:
  - `#01a` Start screen
  - `#02a` Editor, nothing selected
  - `#03a` Face selected — **interactive**: panel toggles + ⌘1/⌘2
  - `#03b` Both panels hidden
  - `#04a` Sketch, nothing selected (constraint list open) → `#04b` sketch line selected
  - `#05a` Extrude feature card
  - `#06a` Fillet / Chamfer
  - `#07a` History tab, rolled back, inline edit
- `OneCAD Components.dc.html` — component sheet with variants + specs, grouped `#g00` Foundations … `#g07` Icons. Each card id (`#c-pill`, `#c-tree`, …) is referenced below.
- `cad-icons.js` — 150-icon colour isometric set (source of truth for icon geometry and palette). `<cad-icon name="extrude" size="24">`.
- `cad-scene.js` — mock viewport diagrams only.
- `support.js` — runtime for the `.dc.html` files.

## Screens

### 01a Start (`StartScreen`)
- Window 1280×800. Title bar 44px `#F6F7F8`, search field 260×28 right.
- Sidebar 220px, `#FAFAFB`, right border `#E2E4E8`, padding 16/10. Brand row (28px icon well, "OneCAD" 15/600). Nav items 32px, r6, icon 18, 13px; active `#E1EBFB` / `#1D4FA8` 500. Items: Home, Recent, Starred, Templates, Component library; bottom: Extensions, Settings.
- Main padding 32/40, gap 28. Row of 3 ActionTiles (h92, r10): **New part** (accent fill, white text, "Sketch and model a single part · ⌘N"), **New assembly**, **Open or import** (".onecad, STEP, STL, 3MF · ⌘O").
- "Recent" 15/600 + Grid/List segmented. 4-col grid gap 16 of RecentCards (thumb 150, r10, name 13/600, meta 11.5 `#8A919C`; hover border `#B9CDF2` + `0 3px 10px rgba(30,60,120,.1)`).
- "Start from a template" chips h32 r6.

### 02a Editor — nothing selected
- Title bar (44px): traffic-light inset · Home · File ▾ · flex · **document name centred** 13/600 `#3A3F46` + dirty dot 7px `#8A919C` + " — Workshop" 12 `#8A919C` · flex · undo/redo 28px · command search 220×28 ("Search commands", `⌘K`). No workspace switcher and no Generators menu in the title bar any more.
- Viewport fills the rest above a 28px status bar.
- **PanelToggle (Explorer)** at left 10 / top 10, 44×44 (`#c-ptoggle`).
- **WorkspaceButton** at left 62 / top 10, 196×44 (`#c-ws`): 32px icon well `#EEF3FC`, "Design" 13/600, "Workspace · ⌃1" 11 `#8A919C`, chevron.
- **Explorer** FloatingPanel at left 10 / top 62 / bottom 10, width 248: Filter field; sections BODIES / SKETCHES / DATUMS (section label 11/500 +0.07em `#8A919C`); TreeRows; footer 36px "3 extensions active".
- **ToolPill** top 10, centred over the free viewport (see rules). Idle contents: Select (active) | Sketch, Datum plane, Offset plane | Generators (gear), Insert component, Import | Measure, Section.
- **ViewCluster** right 14 / top 62 (below the Inspector toggle at right 10 / top 10): view cube 72, display mode, grid (on), snap — 36px square buttons r8.
- **NavPill** left 268 / bottom 10: home, fit, isolate (disabled w/o body selection), section, layers.
- Status bar: "Ready" · "1 body · 2 sketches" … Persp/Ortho · `X 42.10  Y 18.00  Z 8.00` mm (mono).

### 03a Selection — face
- Viewport face highlighted (fill `#BCD3F7`, edge `#2E6FE0` 2px).
- Explorer row "Bracket" selected (`#EEF3FC`, inset 1px `#CBDCF7`, meta "1 face").
- ToolPill becomes: SelectionLabel "Face · Bracket · planar ×" | Sketch, Extrude (push/pull), Offset face, Hole, Draft, Shell | Material, Measure | Isolate, Hide, More. Border `#CBDCF7`.
- **Inspector** FloatingPanel right 10 / top 10 / bottom 10, width 288. Header row: segmented Properties | History + in-panel toggle 32×32. Title block (36px icon well, "Face 7" 14/600, "Planar · on Bracket"). GEOMETRY rows (Area 3 093.14 mm², Perimeter 306.83 mm, Normal +Z, Offset Z 8.00 mm), BODY rows (Material select "Aluminium 6061", Mass 96.85 g, Volume 35 870 mm³). Footer hint "⇧-click adds faces · Esc clears".
- ViewCluster moves to right 308 / top 10 while the Inspector is open.

### 03b Panels hidden
- Both panels hidden; only their 44px toggles remain (Explorer toggle + WorkspaceButton stay; Inspector toggle at right 10 / top 10 with a 7px accent dot meaning "selection waiting").
- Pill recentres at 50%; ViewCluster right 14 / top 62; NavPill left 10.

### 04a Sketch — nothing selected
- Viewport tint `#F4F7FC`, grid `#D5DEEC` / major `#B7C6DE`, camera normal to the plane. Reference geometry `#6B7A8F`, under-constrained `#2D7FF9`, fully constrained `#1F9D55`, selected `#D97706`.
- Pill tint `#F0F5FE` border `#CBDCF7`: Select | Line (active `#D6E4FA`), Rectangle, Circle, Arc, Slot, Polygon, Spline, Point | Dimension, **Constraints ▾** | Trim, Extend, Sketch fillet, Offset, Project.
- **SessionStrip** under the pill (top 62, h38, `#E9F1FD`, border `#CBDCF7`, r10): sketch icon, "Sketch 3" 12.5/600 `#1D4FA8`, "on Face 7", DOF "3 DOF left" `#B26B10` with under-constrained icon, Cancel, **Finish sketch** (primary).
- **Constraint MenuList** (open) below the strip, width 236: GEOMETRIC — Coincident C, Horizontal H, Vertical V, Parallel ⇧P, Perpendicular ⇧L, Tangent T, Concentric, Equal =, Midpoint M, Symmetric, Point on curve, Fix F; DIMENSIONAL — Distance D, Angle A, Radius / diameter R. Rows not applicable to the current selection are disabled (40% opacity).
- Explorer: "Bracket" muted "reference"; "Sketch 3" editing row (`#E1EBFB`, 500) with child rows: Rectangle · 4 lines, Circle ⌀6, Centre line, 7 constraints (indent 34, left rule `#E2E4E8`, 28px rows). **Keep this child detail — user-approved.**
- Status bar tinted `#F0F5FE`: "Sketch | Line — click start point · Esc to stop" … `X 70.00  Y 44.00` mm.

### 04b Sketch — line selected
- Pill swaps to SelectionLabel (orange variant "Line · 30.00 mm") | Horizontal (applied = active), Vertical (disabled), Parallel, Perpendicular, Equal, Midpoint, Fix, Distance | Construction, Delete. Border `#F0C79A`.
- Session strip unchanged. Status: "1 line selected · ⇧-click to add" … "Under-constrained".

### 05a Extrude
- Preview geometry orange (`#F7A548` 35–55% fill, `#D97A16` stroke); drag arrow + knob on the cap.
- Pill: SelectionLabel "Region · Sketch 3" | **Extrude (running: `#FDEBD6`, inset `#F0C79A`)**, Revolve, Sweep, Loft | Measure, More.
- ValueTag at the handle: "14.00 mm" (h26, r6, 1.5px `#D97A16`, mono 12/600 `#B5600A`).
- **FeatureCard** beside the handle (w280, r12, card shadow): header 44 (icon, "Extrude", key "E", ×) · grid label col 76, row gap 10: Profile chip "Sketch 3 · region 1 ×"; Direction segmented One side / Symmetric / Two; Extent select Distance; Distance NumberField 14.00 mm (focused); Operation icon group Join / Cut / Intersect / New body + caption "Join with Bracket"; Draft 0.0° · footer Cancel / OK ↵.
- Status: green dot "Preview ready · 38 ms" · "Drag arrow · Tab cycles fields · ↵ OK · Esc cancel".

### 06a Fillet / Chamfer
- Edge highlight orange 3px; fillet strip preview orange.
- Pill: "Edge · Bracket · 80.00 mm" | Fillet (running), Chamfer | Measure, Isolate, More.
- Card (w276): Type segmented Fillet / Chamfer (mirrors drag direction: outward rounds, inward chamfers), Edges chip "1 edge ×" + add, Radius 6.00 mm (focused), Tangent chain toggle on. ValueTag "R 6.00".

### 07a History
- Inspector History tab: "7 FEATURES" + overflow. HistoryRows h36: index mono 11 `#A6ACB5`, icon 20, label, meta mono. Rows: Base profile, Extrude 1 (8 mm), Upright profile, **Extrude 2 editing inline** (NumberField 78px, 45.00 mm, focused), Hole 1 (⌀10 ×2), Hole 2 (⌀18), **RollbackMarker** (14px accent handle + 2px accent line + "Rollback"), Fillet 1 (rolled back, 45% opacity).
- Info box `#F6F7F8`: "Changes regenerate everything below. Features that can no longer resolve are flagged here, not deleted."
- Pill: "Extrude 2 · feature" | Edit, Roll back to here, Suppress, Duplicate | Delete, More.
- SessionStrip (white): "Rolled back to Hole 2 — new features insert here" + "Roll to end".

## ToolPill rules (core behaviour)
- Exactly one pill. Never a second toolbar row.
- Contents = tools whose scope matches the **current selection kind**: `none | face | edge | region | body | sketchEntity(line/arc/point…) | feature`. Unscoped tools appear only in `none`.
- When selection is non-empty, prepend a SelectionLabel (kind + context + ×). × or Esc clears selection and restores the idle set.
- Running command: its button gets the orange "running" state; Esc cancels, ↵ commits.
- In sketch mode the pill is tinted; sketch-entity selection uses the orange SelectionLabel variant.
- Disabled tools keep their slot at 40% and the Tooltip shows `ToolAvailability.reason`.
- Pill horizontal centre = centre of free viewport. Inspector open → shift left by 64px; else 50%. (Explorer state doesn't affect it because the WorkspaceButton always occupies the top-left.)
- Implement as data: extend `ToolDefinition` with `selectionScopes: SelectionKind[]` and `group`; the host filters, groups and orders (`priority`) — modules don't render toolbar UI.

## Interactions & state
- `ui.explorerOpen` (default true), `ui.inspectorOpen` (default true) — persisted per workspace. ⌘1 / ⌘2 toggle. Inspector auto-shows on selection only if the user has not hidden it; a hidden Inspector with a selection shows the dot on its toggle.
- Workspace menu: Design ⌃1, Assembly ⌃2, Simulation ⌃3, Drawing ⌃4, then "FROM EXTENSIONS", then Customize workspace…, Manage extensions…. Switching re-lenses pill groups, Explorer sections and Inspector — shell never re-mounts.
- Tooltips appear after 400 ms; include shortcut.
- NumberField: accepts expressions (`=d1/2`, violet text `#7A3FD0`), ↑/↓ step, ⇧ ×10, unit stays inside; invalid → danger border + message below ("Distance must be greater than 0").
- Feature card: Tab cycles fields; dragging the handle updates the focused field live; preview status shown in the status bar.
- History: double-click value → inline NumberField; drag marker to roll back; failed features get `#FDECEC` row + failed icon; suppressed = strike-through.
- Transitions: panels slide 160 ms ease-out (translateX + opacity); pill content cross-fades 120 ms; no bounce.

## Design tokens
Native values belong in `assets/native/tokens.css` and the widget theme tokens
(light shown; author dark equivalents).
- Accent `#2E6FE0`, hover `#2660C6`; selection bg `#E1EBFB`, text `#1D4FA8`, border `#CBDCF7`, soft sel `#EEF3FC`.
- Meaning: created/preview `#F7A548` / stroke `#D97A16` / text `#B5600A` / running bg `#FDEBD6` border `#F0C79A`; sketch `#2D7FF9`; done/OK `#1F9D55` (text `#157347`); warn `#B26B10`; danger `#C23A33` (surface `#FDECEC`, border `#F4C9C9`).
- Surfaces: floating `rgba(255,255,255,.97)`; titlebar `#F6F7F8`; statusbar `#FBFBFC`; viewport `#EEF0F3`; sketch viewport `#F4F7FC`; sketch chrome `#E9F1FD`; sketch toolbar `#F0F5FE`; well `#F0F2F5`; chip `#F4F5F7`.
- Borders `#E2E4E8`, strong `#D8DBE0`, subtle `#EDEEF1`.
- Ink `#1B1D21`, `#2A2E34`, tree `#33383F`, `#4A5158`, `#5A626D`, `#8A919C`, `#9AA1AB`, `#A6ACB5`.
- Type: system UI (`-apple-system, BlinkMacSystemFont, "Segoe UI"`); mono `ui-monospace, Menlo` for all numbers. Scale 15/600 · 14/600 · 13 · 12.5 · 12 · 11/500 +0.07em caps.
- Radii 6 (field/button/row) · 8 (tool/square button) · 10 (strip/card/popover) · 12 (floating panel, pill, feature card).
- Heights 24 tooltip · 28 chip/search · 30 field/button · 32 tree row · 34 tool · 36 square/history row · 44 title bar/panel toggle/workspace button · 28 status bar.
- Shadows: pill `0 2px 10px rgba(0,0,0,.08)`; panel `0 6px 20px rgba(0,0,0,.08), 0 1px 2px rgba(0,0,0,.04)`; card/menu `0 14px 40px rgba(0,0,0,.16)`; tooltip `0 2px 6px rgba(0,0,0,.2)`; focus ring `0 0 0 3px rgba(46,111,224,.15)`.
- Spacing 4 · 6 · 8 · 10 · 12 · 16 · 20 · 24. Floating chrome 10px from edges, 8px apart.

## Assets
- Icons: user-supplied "OneCAD Icon Set — Colour Isometric" (150 icons), extracted into `cad-icons.js`. Colour carries meaning: orange = creates geometry, blue = sketch/modify, red = constraint/danger, green = select/OK. Palettes for light and dark are in the file (`PAL`).
- Panel toggle glyph: simple sidebar outline (inline SVG in the files) — add to the icon set as `panelLeft` / `panelRight`.
- No raster images. Recent-project thumbnails come from the document's saved preview.

## Not yet designed
Generators, Component Library, Assembly, Simulation, Drawing, Extensions manager, Settings, ⌘K palette, error/onboarding/export states. Build them from the same components; ask before inventing new ones.
