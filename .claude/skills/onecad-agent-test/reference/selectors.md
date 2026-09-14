# OneCAD selectors

Toolbar tools have **no testid**: target them by `{role:"button", name:<label>}` (`aria-label`;
`aria-pressed` reflects the active tool; a disabled tool has `aria-disabled` and its tooltip shows
the reason). The toolbar itself is `{role:"toolbar", name:"Tools"}`. Tooltips are `role="tooltip"`
with text `"<label> (<shortcut>)"`, shown on mouse enter (no delay) — a `pointer_hover` with
`dwellMs ≥ 300` is enough.

## Model-mode tools (label → shortcut)
Select V · New sketch S · Datum plane D · Extrude E · Revolve R · Fillet / Chamfer F · Combine B ·
Shell K · Offset face ⇧O · Hole ⇧H · Linear pattern P · Circular pattern C · Mirror M · Move T ·
Measure ? · (Gear ⇧G via generators menu `generators-menu-trigger`).

## Sketch-mode tools
Select V · Line L · Rectangle R · Center rectangle ⇧R · Circle C · Ellipse O · Arc A · 3-point arc ⇧A ·
Polygon G · Slot S · Point P · Dimension D · Trim T · Extend ⇧T · Mirror M · Fillet F · Offset ⇧O.
Families share one slot (Rectangle/Center rectangle, Circle/Ellipse, Arc/3-point arc): the
non-default member is in the slot's flyout `{role:"button", name:"<Family> options"}` → `{role:"menuitem", name:...}`.

## Shell / chrome
| testid | What |
|---|---|
| `document-title` | title bar name (drag region — move only, never press) |
| `file-menu-rename`, `rename-document-*` | rename dialog |
| `command-palette`, `command-palette-input`, `palette-button` | ⌘K palette |
| `status-hint`, `regen-busy`, `sketch-dof`, `fov`, `grid-scale` | status bar |
| `navigation-help`, `corner-cluster`, `tasks-chip`, `layers-menu` | shell chrome |
| `workspace-switcher`, `workspace-customize`, `customize-*` | workspaces |
| `error-boundary`, `contribution-error`, `repair-banner`, `projection-banner`, `timeline-stopped-banner`, `missing-extension-*` | error surfaces |

## Viewport
`viewport-canvas` (container; the `<canvas>` inside has no attributes and fills the same rect),
`geometry-pending`, `geometry-cached`, `revolve-empty-hint`. Plane-pick hover chip:
`css:"[data-plane-pick-label]"`. Section: `section-toggle`, `section-flip`, `section-offset-input`,
`section-plane-xy|xz|yz`.

## Tool chips (active operation)
`operation-hud`, `chip-confirm`, `chip-cancel`, `chip-return`, `chip-drag-handle`, `chip-dock`,
`chip-mode-badge`, `chip-result-summary`, `chip-region-count`, `chip-sketch-label`,
`tool-validation`, `tool-validation-use-suggested`; Extrude: `chip-draft-input`, `chip-symmetric`;
Edge ops: `chip-edgeop-badge`, `chip-chamfer-angle`, `chip-chamfer-d2`, `chip-chamfer-flip`;
Revolve: `chip-revolve-badge`, `chip-revolve-axis-hint`; Boolean: `chip-boolean-badge`, `chip-bool-swap`;
Hole: `chip-hole-badge`, `chip-hole-std`, `chip-hole-std-panel`, `chip-hole-depth`, `chip-hole-thread`;
Pattern: `chip-pattern-badge`, `pattern-count`; Mirror: `chip-mirror-badge`, `chip-mirror-fuse`;
Offset: `chip-offset-badge`, `chip-offset-tangent`; Transform: `chip-transform-badge`,
`chip-transform-align`, `chip-transform-copy`; Datum: `chip-datum-base`; Gear: `chip-gear-summary`.

## Inspector / history
`inspector-panel`, `inspector-drawer-toggle`, `inspector-drawer-content`, `inspector-resize-handle`,
`active-tool-inspector`, `active-tool-targets`, `inspector-target-element`, `inspector-target-sketch`,
`inspector-targets-missing`, `history-count`, `history-row-<id>` (aria-label "Select feature N: label"),
`history-menu-edit|suppress|delete|delete-confirm|reattach|roll-here|roll-end`, `history-roll-to-end`,
`history-rollback-banner`, `history-rollback-marker`, `feature-diagnostic-section`, `feature-edit-retry`,
`operation-diagnostics`, `variables-*`, `variable-new-name|value`, `gear-properties-*`, `fit-preview`.

## Sketch chrome
`sketch-construction-banner`, `constraint-badges`, `constraint-context-chips`,
`selection-dimension-labels`, `dimension-expr-hint`, `dimension-expr-preview`, `sketch-error-pulse`,
`projection-update`, `projection-detach`. Chrome bar buttons: `Cancel` (discards), `Finish`
(same as Enter). Constraint chords in sketch: ⇧H/⇧V/⇧C/⇧E/⇧P/⇧M.

## Start screen, library, measure
`New project` / `Import` buttons, `Search projects` (aria-label), `template-card`,
`project-card-rename|delete|delete-confirm|reveal`, `start-extensions`; library:
`library-import-components`, `library-rebuild-index`, `library-ingest-*`, `save-as-component-*`,
`save-as-template-*`, `component-*`, `detail-*`; measure: `measure-panel`, `measure-distance`,
`measure-angle`, `measure-plane-pair`, `measurement-annotation-controls`.
