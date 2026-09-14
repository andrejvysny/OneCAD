/*
 * FROZEN editor-shell mount contract — the order in which the editor mounts its
 * children, as shipped before the Platform refactor.
 *
 * ORDER IS LOAD-BEARING. These are absolutely-positioned siblings over the
 * viewport; DOM order decides stacking within a z-index band. A past defect had
 * tool chips rendering under the side panels and becoming unclickable, so this
 * list is a real invariant, not documentation.
 *
 * See ./README.md before editing.
 *
 * AMENDED 2026-08-09 (MODULAR-PLATFORM wave) — a deliberate, user-visible
 * change, recorded in TODO.md as the README requires. Six shell contributions
 * joined the editor:
 *   MissingExtensionBanner / WorkspacePlaceholder  flow strips under the title
 *     bar (`shell.top`), so they take layout space instead of floating over the
 *     toolbar the way the notification-slot pills do
 *   CommandPalette / ExtensionsManager /
 *   CustomizeWorkspaceSheet / MissingExtensionDialog
 *     modal overlays in the new `shell.overlay` region, which is LAST because a
 *     modal has to cover every region above it
 * Nothing already in this list moved relative to anything else.
 *
 * AMENDED 2026-08-12 (Component Library WP-1.4) — recorded in TODO.md.
 * `LibraryPanel` joined `Slots.ShellLeft` right after `ModelTreePanel`
 * (priority 110 vs. `ModelTreePanel`'s 100). It does NOT add a second visible
 * sidebar: both panels occupy the SAME `left:0` footprint and each renders
 * `null` unless `sidebarTabStore.activeTab` names it (a VS Code-style tab
 * strip both render at their own top) — see `SidebarTabHeader`'s doc
 * comment. `LibraryPanel` is still a real, separately-registered
 * contribution (its OWN entry in the registry, its OWN mount-order slot), so
 * it belongs in this contract like any other panel.
 *
 * AMENDED 2026-08-13 (Component Library WP-B2) — recorded in TODO.md.
 * `SaveAsComponentHost` joined `Slots.ShellOverlay` at the END, after the four
 * shell modals. It is the "Save as Component" dialog's mount point and renders
 * `null` whenever no body is being authored, so nothing above it moved and
 * nothing new is visible until the user opens it. Overlay LAST is the same
 * rule the region itself follows: a modal has to cover every region above it.
 *
 * AMENDED 2026-08-14 (UI/UX pass) — recorded in TODO.md. `LibraryPanel` LEFT
 * `Slots.ShellLeft` — the library browser is now a full-size `LibraryModal`,
 * opened from a toolbar tool rather than docked in the sidebar. `VariablesPanel`
 * (modeling) took its exact slot/priority (110, right after `ModelTreePanel`),
 * keeping the same tab-strip mechanism `SidebarTabHeader` documents.
 * `LibraryModalHost` joined `Slots.ShellOverlay` AFTER `SaveAsComponentHost`
 * (priority 150): it is opened from the toolbar, so at mount time it is never
 * the most-recently-requested overlay, and it renders `null` until the
 * "Library" tool activates it — same "nothing new is visible until opened"
 * reasoning as `SaveAsComponentHost` above.
 *
 * AMENDED 2026-08-15 (Sketcher UX cleanup, Track A3) — recorded in TODO.md.
 * `SketchConstraintToolbar` LEFT the registry — its persistent floating pill
 * (a third stacked sketch-mode toolbar row) is retired. Its button grid moved
 * verbatim into `ConstraintMenu`, a trigger + popover mounted INSIDE
 * `SketchChromeBar` (no new panel id, no new registry entry — it is a plain
 * child component, not a contribution). Nothing else in this list moved.
 *
 * AMENDED 2026-09-11 (UX review shared work area) — recorded in TODO.md.
 * `ToolbarPrimary`/`ToolbarContextual` and the left/right/bottom shell regions
 * now have `display: contents` measurement wrappers. The wrappers add DOM
 * nodes but no layout boxes, preserve contribution order and component
 * identity, and let floating chrome use the panels' actual rendered bounds.
 *
 * AMENDED 2026-09-14 (onecad.assistant module) — a deliberate, user-visible
 * change, recorded in TODO.md as the README requires. `AssistantPanel` joined
 * `Slots.ShellLeft` after `VariablesPanel` (priority 120 vs. 110), as a THIRD
 * occupant of the same `left:0` footprint the model tree and the variables
 * table already share: it renders `null` unless `sidebarTabStore.activeTab`
 * names it, exactly as those two do, so it adds no second visible sidebar. It
 * is additionally gated on `settingsStore.assistantEnabled`, which is OFF by
 * default — mount order is unconditional (the contribution registers either
 * way), but a user who has not opted in sees no Assistant tab and no panel.
 * Nothing already in this list moved relative to anything else.
 */
export const EDITOR_MOUNT_ORDER_CONTRACT: readonly string[] = [
  "TitleBar",
  "MissingExtensionBanner",
  "WorkspacePlaceholder",
  "ViewportRoot",
  "ConstraintBadgeLayer",
  "SelectionDimensionLabels",
  "LiveDimChips",
  "ConstraintContextChips",
  "ModelToolChips",
  "MeasureOverlay",
  "RepairMarkerOverlay",
  // `MeasurePanel` left this list on 2026-09-13: it renders as the Measurement
  // inspector section (priority 90, `inspectorSections.ts`) while Measure is
  // active, not as its own shell contribution — recorded decision in TODO.md.
  "FloatingToolbar",
  "SketchChromeBar",
  "ModelTreePanel",
  "VariablesPanel",
  "AssistantPanel",
  "InspectorPanel",
  "RepairBanner",
  "TimelineStoppedBanner",
  "CornerCluster",
  "NavPill",
  "StatusBar",
  "CommandPalette",
  "ExtensionsManager",
  "CustomizeWorkspaceSheet",
  "MissingExtensionDialog",
  "SaveAsComponentHost",
  "LibraryModalHost",
];
