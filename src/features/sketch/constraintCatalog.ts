/*
 * Shared presentation catalog for constraint kinds — the SINGLE glyph + icon +
 * label source for every constraint UI (toolbar, context chips, sketch canvas
 * badges via badgeLayout.ts, and the inspector's ConstraintList). No other
 * module defines its own table. The record covers all 18 kinds so any consumer
 * can label anything the applicability matrix emits (incl. H-/V-Distance).
 *
 * TWO representations, deliberately. `icon` is the drawn two-tone symbol from
 * the CAD icon family and is what the DOM surfaces render. `glyph` is the text
 * fallback, still used by the canvas badge layer: `badgeLayout.ts` places
 * badges from MEASURED TEXT WIDTH, so swapping those to icons means moving that
 * layer to fixed-size boxes and re-verifying collision/placement. Until that
 * happens both live here rather than in two competing tables.
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
];

/** Ordered dimensional kinds the toolbar surfaces (each opens the Dimension chip). */
export const DIMENSIONAL_TYPES: SketchConstraintType[] = ["Distance", "Radius", "Diameter", "Angle"];
