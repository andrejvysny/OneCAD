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

/** Ordered dimensional kinds the toolbar surfaces (each opens the Dimension chip). */
export const DIMENSIONAL_TYPES: SketchConstraintType[] = ["Distance", "Radius", "Diameter", "Angle"];
