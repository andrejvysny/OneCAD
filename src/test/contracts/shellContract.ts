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
 */
export const EDITOR_MOUNT_ORDER_CONTRACT: readonly string[] = [
  "TitleBar",
  "ViewportRoot",
  "ConstraintBadgeLayer",
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
  "InspectorPanel",
  "RepairBanner",
  "TimelineStoppedBanner",
  "CornerCluster",
  "NavPill",
  "StatusBar",
];
