/*
 * Shared presentation catalog for constraint kinds — the SINGLE glyph + icon +
 * label source for every constraint UI (toolbar, context chips, sketch canvas
 * badges via badgeLayout.ts, and the inspector's ConstraintList). No other
 * module defines its own table. The record covers all 20 kinds so any consumer
 * can label anything the applicability matrix emits (incl. H-/V-Distance and the
 * point-pair axis alignments).
 *
 * TWO representations. `icon` is the drawn two-tone symbol from the CAD icon
 * family and is what every DOM surface renders, INCLUDING the canvas badge
 * layer (`ConstraintBadgeLayer` unified onto it — Sketcher UX cleanup, Track
 * B2; badges already sat in a fixed-size box with fixed offsetIndex staggering,
 * not measured text, so the swap needed no layout change). `glyph` is a plain
 * text fallback kept for cheap non-DOM contexts (tests, logs) — no renderer
 * reads it anymore.
 */
import type { IconName } from "@/icons/paths";
import type { SketchConstraintType } from "@/ipc/types";

export const CONSTRAINT_PRESENTATION: Record<
  SketchConstraintType,
  { glyph: string; icon: IconName; label: string }
> = {
  Coincident: { glyph: "•", icon: "constraintCoincident", label: "Coincident" },
  Horizontal: { glyph: "H", icon: "constraintHorizontal", label: "Horizontal" },
  Vertical: { glyph: "V", icon: "constraintVertical", label: "Vertical" },
  Fixed: { glyph: "⚓", icon: "constraintFix", label: "Fixed" },
  Midpoint: { glyph: "M", icon: "constraintMidpoint", label: "Midpoint" },
  OnCurve: { glyph: "⌒", icon: "constraintPointOnCurve", label: "On curve" },
  Parallel: { glyph: "∥", icon: "constraintParallel", label: "Parallel" },
  Perpendicular: { glyph: "⟂", icon: "constraintPerpendicular", label: "Perpendicular" },
  Tangent: { glyph: "T", icon: "constraintTangent", label: "Tangent" },
  Concentric: { glyph: "◎", icon: "constraintConcentric", label: "Concentric" },
  Equal: { glyph: "=", icon: "constraintEqual", label: "Equal" },
  Distance: { glyph: "↔", icon: "constraintDistance", label: "Distance" },
  HorizontalDistance: { glyph: "H↔", icon: "constraintDistanceH", label: "Horizontal distance" },
  VerticalDistance: { glyph: "V↔", icon: "constraintDistanceV", label: "Vertical distance" },
  Angle: { glyph: "∠", icon: "constraintAngle", label: "Angle" },
  Radius: { glyph: "R", icon: "constraintRadius", label: "Radius" },
  Diameter: { glyph: "⌀", icon: "constraintDiameter", label: "Diameter" },
  Symmetric: { glyph: "⋈", icon: "constraintSymmetric", label: "Symmetric" },
  // SNAP P3. Same visual FAMILY as the line-only H/V (they assert the same
  // direction), distinct LABELS because they constrain a point pair rather than
  // an entity — the accessible name is what tells a reader which one they have.
  HorizontalPoints: {
    glyph: "H··",
    icon: "constraintHorizontal",
    label: "Horizontal point alignment",
  },
  VerticalPoints: { glyph: "V··", icon: "constraintVertical", label: "Vertical point alignment" },
};

/** Ordered geometric kinds the toolbar surfaces (design item 4). */
export const GEOMETRIC_TYPES: SketchConstraintType[] = [
  "Coincident",
  "Horizontal",
  "Vertical",
  "Parallel",
  "Perpendicular",
  "Concentric",
  "Symmetric",
  "Fixed",
  "OnCurve",
  "Tangent",
  "Equal",
  "Midpoint",
  "HorizontalPoints",
  "VerticalPoints",
];

/**
 * Ordered dimensional kinds the toolbar surfaces (each opens the Dimension
 * chip). H-/V-Distance were wire-encodable and applicable (two-point
 * selection, `constraintApplicability.ts`) but missing here, making them
 * unreachable from the Add-constraint menu (audit A11g) — added next to
 * `Distance`, the rest of the same "two targets, a length" family.
 */
export const DIMENSIONAL_TYPES: SketchConstraintType[] = [
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Radius",
  "Diameter",
  "Angle",
];

/**
 * "Why not" reason for a disabled Add-constraint row (design item 4b) — a
 * short imperative sentence naming the selection shape each kind accepts.
 * Static, not derived at runtime: `evaluateApplicability`'s selection-shape
 * rules (constraintApplicability.ts, see its header for the full matrix) are
 * a fixed fact about each kind, never a function of the current selection, so
 * a lookup table is the honest representation — no need to restructure the
 * evaluator to ask it "why not" for a shape it was never offered.
 */
export const CONSTRAINT_REQUIREMENT: Record<SketchConstraintType, string> = {
  Coincident: "Select two points",
  Horizontal: "Select a line",
  Vertical: "Select a line",
  Fixed: "Select a point",
  Midpoint: "Select a point and a line",
  OnCurve: "Select a point and a curve",
  Parallel: "Select two lines",
  Perpendicular: "Select two lines",
  Tangent: "Select two curves",
  Concentric: "Select two circles or arcs",
  Equal: "Select two lines, or two circles or arcs",
  Distance: "Select two points or lines",
  HorizontalDistance: "Select two points",
  VerticalDistance: "Select two points",
  Angle: "Select two lines",
  Radius: "Select a circle or arc",
  Diameter: "Select a circle or arc",
  Symmetric: "Select two points and a line",
  HorizontalPoints: "Select two points",
  VerticalPoints: "Select two points",
};
