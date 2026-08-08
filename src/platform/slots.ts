/*
 * UI slots (ADR-0003).
 *
 * A CLOSED set. A contribution names a slot; the host decides what that means on
 * this platform and at this window size. That indirection is what lets the same
 * contribution be a docked panel on desktop and something else on iPad without
 * the contributor doing anything — and what keeps addons out of raw DOM.
 *
 * Adding a slot is a deliberate platform change, not a convenience: every slot is
 * a promise about layout the host has to keep on every platform.
 */

export const Slots = {
  /** Application chrome above the viewport (title bar strip). */
  ShellTop: "shell.top",
  /** Docked left region (model tree today). */
  ShellLeft: "shell.left",
  /** Docked right region (inspector today). */
  ShellRight: "shell.right",
  /** Docked bottom strip (status bar today). */
  ShellBottom: "shell.bottom",

  /** The main tool palette. */
  ToolbarPrimary: "toolbar.primary",
  /** Tool/selection-dependent controls (sketch chrome, constraint bar). */
  ToolbarContextual: "toolbar.contextual",
  /** Spill target when the primary toolbar does not fit. */
  ToolbarOverflow: "toolbar.overflow",

  /** DOM overlays that track scene content (chips, badges, markers, measure UI). */
  ViewportOverlay: "viewport.overlay",
  /** In-scene contributions (Three.js objects), attached via the viewport context. */
  ViewportLayer: "viewport.layer",
  /** Contextual (right-click) surface over the viewport. */
  ViewportContext: "viewport.context",
  /**
   * Persistent controls anchored to the viewport FRAME rather than to scene
   * content — view orientation, display options. Distinct from `viewport.overlay`
   * because these do not move with the model and sit above the docked panels.
   */
  ViewportChrome: "viewport.chrome",

  /** Non-blocking banners and notices across the top of the work area. */
  ShellNotification: "shell.notification",

  /** A section inside the inspector panel. */
  InspectorSection: "inspector.section",

  /** Adornment on a project-tree row. */
  TreeDecorator: "tree.decorator",
  /** Contextual actions on a project-tree row. */
  TreeContext: "tree.context",

  /** A segment of the status bar. */
  StatusSection: "status.section",
} as const;

export type SlotId = (typeof Slots)[keyof typeof Slots];

export const ALL_SLOTS: readonly SlotId[] = Object.values(Slots);

export function isSlotId(value: string): value is SlotId {
  return (ALL_SLOTS as readonly string[]).includes(value);
}
