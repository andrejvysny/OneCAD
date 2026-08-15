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
  "MeasurePanel",
  "FloatingToolbar",
  "SketchChromeBar",
  "SketchConstraintToolbar",
  "ModelTreePanel",
  "VariablesPanel",
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
