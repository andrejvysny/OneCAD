/*
 * Shared presentation catalog for constraint kinds — the SINGLE glyph + label
 * source for every constraint UI (toolbar, context chips, sketch canvas
 * badges via badgeLayout.ts, and the inspector's ConstraintList). No other
 * module defines its own glyph table; text glyphs keep the surface
 * token-only (no raw colors). The record covers all 18 kinds so any consumer
 * can label anything the applicability matrix emits (incl. H-/V-Distance).
 */
import type { SketchConstraintType } from "@/ipc/types";

export const CONSTRAINT_PRESENTATION: Record<SketchConstraintType, { glyph: string; label: string }> = {
  Coincident: { glyph: "•", label: "Coincident" },
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
  HorizontalDistance: { glyph: "H↔", label: "Horizontal distance" },
  VerticalDistance: { glyph: "V↔", label: "Vertical distance" },
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
