/*
 * Shared presentation catalog for constraint kinds — the glyph + label both
 * constraint UIs (toolbar + context chips) render. Glyphs are plain Unicode
 * (reused from the badge vocabulary, `badgeLayout.ts GLYPH`); no icon-path entries
 * exist for constraints in `@/icons/paths`, and text glyphs keep the surface
 * token-only (no raw colors). The record covers all 18 kinds so the context chips
 * can label anything the applicability matrix emits (incl. H-/V-Distance).
 */
import type { SketchConstraintType } from "@/ipc/types";

export const CONSTRAINT_PRESENTATION: Record<SketchConstraintType, { glyph: string; label: string }> = {
  Coincident: { glyph: "⌖", label: "Coincident" },
  Horizontal: { glyph: "H", label: "Horizontal" },
  Vertical: { glyph: "V", label: "Vertical" },
  Fixed: { glyph: "⚓", label: "Fixed" },
  Midpoint: { glyph: "M", label: "Midpoint" },
  OnCurve: { glyph: "⌒", label: "On curve" },
  Parallel: { glyph: "∥", label: "Parallel" },
  Perpendicular: { glyph: "⟂", label: "Perpendicular" },
  Tangent: { glyph: "T", label: "Tangent" },
  Concentric: { glyph: "◎", label: "Concentric" },
  Equal: { glyph: "=", label: "Equal" },
  Distance: { glyph: "↔", label: "Distance" },
  HorizontalDistance: { glyph: "↔", label: "Horizontal distance" },
  VerticalDistance: { glyph: "↕", label: "Vertical distance" },
  Angle: { glyph: "∠", label: "Angle" },
  Radius: { glyph: "R", label: "Radius" },
  Diameter: { glyph: "⌀", label: "Diameter" },
  Symmetric: { glyph: "⋈", label: "Symmetric" },
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
];

/** Ordered dimensional kinds the toolbar surfaces (each opens the Dimension chip). */
export const DIMENSIONAL_TYPES: SketchConstraintType[] = ["Distance", "Radius", "Diameter", "Angle"];
