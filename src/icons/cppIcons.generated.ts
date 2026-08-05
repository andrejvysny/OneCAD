/*
 * GENERATED FILE — DO NOT EDIT.
 *
 * Source of truth: OneCAD-CPP `resources/icons/*.svg` (88 masters) and
 * the spec they are drawn against, mirrored here as `src/icons/DESIGN.md`.
 * Regenerate with:
 *
 *   node scripts/gen-icons.mjs
 *
 * Transformations applied by the generator:
 *   - `ic_foo_bar.svg` -> key `fooBar`
 *   - the masters' literal accent color -> `tone: "accent"`, which the renderer
 *     paints with `var(--color-icon-accent)`. No color literal survives into
 *     this file, which is what keeps the hex gate clean.
 *   - `fill=<color> stroke="none"` -> `filled: true`
 *   - `<g>` attribute wrappers flattened onto their children
 *   - per-element `stroke-width` -> `sw`, a RATIO of the master's root width,
 *     so each call site's own `strokeWidth` stays the master knob
 */

import type { IconDef } from "./types";

export const CPP_ICONS = {
  arc: [
    { d: "M4 18 A11 11 0 0 1 20 8" },
    { d: "M4 18 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M20 8 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  body: [
    { d: "M6 11 L10 8 L18 8 L14 11 Z", tone: "accent", filled: true },
    { d: "M6 11 H14 V20 H6 Z", sw: 0.875 },
    { d: "M6 11 L10 8 H18 L14 11", sw: 0.875 },
    { d: "M18 8 V17 L14 20", sw: 0.875 },
  ],
  booleanIntersect: [
    { d: "M9 9 H15 V15 H9 Z", tone: "accent", filled: true },
    { d: "M4 9 H15 V20 H4 Z", sw: 0.875 },
    { d: "M9 4 H20 V15 H9 Z", sw: 0.875 },
  ],
  booleanSubtract: [
    { d: "M4 9 H15 V20 H4 Z", sw: 0.875 },
    { d: "M11 4 H20 V15 H11 Z", tone: "accent", sw: 0.875, dash: "2.6 2" },
  ],
  booleanUnion: [
    { d: "M4 9 H15 V20 H4 Z", sw: 0.875 },
    { d: "M9 4 H20 V15 H9 Z", sw: 0.875 },
    { d: "M14.5 8 V12 M12.5 10 H16.5", tone: "accent", sw: 0.8 },
  ],
  chamfer: [
    { d: "M5 4 V11" },
    { d: "M13 19 H20" },
    { d: "M5 11 L13 19", tone: "accent" },
  ],
  chamferSolid: [
    { d: "M6 11 H11 L14 14 V20 H6 Z", sw: 0.875 },
    { d: "M6 11 L10 8 H15 L18 11", sw: 0.875 },
    { d: "M14 14 L18 11 V17 L14 20", sw: 0.875 },
    { d: "M11 11 L14 14", tone: "accent", sw: 0.875 },
  ],
  circle: [
    { d: "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0" },
    { d: "M12 12 L20 12", tone: "accent", sw: 0.8 },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  close: [
    { d: "M6 6 L18 18" },
    { d: "M18 6 L6 18" },
  ],
  constraintAngle: [
    { d: "M4 20 H20" },
    { d: "M4 20 L17 7" },
    { d: "M13 20 A9 9 0 0 0 10.4 13.6", tone: "accent", sw: 0.8 },
  ],
  constraintCoincident: [
    { d: "M6.8 12 a3.3 3.3 0 1 0 6.6 0 a3.3 3.3 0 1 0 -6.6 0" },
    { d: "M10.6 12 a3.3 3.3 0 1 0 6.6 0 a3.3 3.3 0 1 0 -6.6 0" },
    { d: "M12 12 m-1.7 0 a1.7 1.7 0 1 0 3.4 0 a1.7 1.7 0 1 0 -3.4 0", tone: "accent", filled: true },
  ],
  constraintConcentric: [
    { d: "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0" },
    { d: "M8 12 a4 4 0 1 0 8 0 a4 4 0 1 0 -8 0" },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  constraintDiameter: [
    { d: "M5 12 a7 7 0 1 0 14 0 a7 7 0 1 0 -14 0" },
    { d: "M5 12 H19", tone: "accent" },
    { d: "M8 9 L5 12 L8 15", tone: "accent", sw: 0.75 },
    { d: "M16 9 L19 12 L16 15", tone: "accent", sw: 0.75 },
  ],
  constraintDistance: [
    { d: "M4 20 L20 4", tone: "accent" },
    { d: "M4 20 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", filled: true },
    { d: "M20 4 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", filled: true },
  ],
  constraintDistanceH: [
    { d: "M5 5 V15" },
    { d: "M19 5 V15" },
    { d: "M5 10 H19", tone: "accent" },
    { d: "M8 7.5 L5 10 L8 12.5", tone: "accent" },
    { d: "M16 7.5 L19 10 L16 12.5", tone: "accent" },
  ],
  constraintDistanceV: [
    { d: "M5 5 H15" },
    { d: "M5 19 H15" },
    { d: "M10 5 V19", tone: "accent" },
    { d: "M7.5 8 L10 5 L12.5 8", tone: "accent" },
    { d: "M7.5 16 L10 19 L12.5 16", tone: "accent" },
  ],
  constraintEqual: [
    { d: "M4 8 H10" },
    { d: "M14 8 H20" },
    { d: "M7 5.5 V10.5", tone: "accent", sw: 0.8 },
    { d: "M17 5.5 V10.5", tone: "accent", sw: 0.8 },
    { d: "M4 16 H10" },
    { d: "M14 16 H20" },
  ],
  constraintFix: [
    { d: "M12 4.4 m-1.8 0 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0", tone: "accent", filled: true },
    { d: "M12 6.2 V13" },
    { d: "M4 13 H20" },
    { d: "M5 13 L3 16 M9 13 L7 16 M13 13 L11 16 M17 13 L15 16 M21 13 L19 16", sw: 0.7 },
  ],
  constraintHorizontal: [
    { d: "M4 12 H20" },
    { d: "M4 12 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", filled: true },
    { d: "M20 12 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", filled: true },
    { d: "M9 8 H15", tone: "accent", sw: 0.8 },
  ],
  constraintMidpoint: [
    { d: "M4 12 H20" },
    { d: "M4 12 m-1.3 0 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0", filled: true },
    { d: "M20 12 m-1.3 0 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0", filled: true },
    { d: "M12 8.5 V15.5", tone: "accent", sw: 0.8 },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  constraintParallel: [
    { d: "M6 4 L3 20" },
    { d: "M15 4 L12 20" },
    { d: "M17.5 10 L19.5 12 L17.5 14 M20 10 L22 12 L20 14", tone: "accent", sw: 0.75 },
  ],
  constraintPerpendicular: [
    { d: "M9 3 V19" },
    { d: "M4 19 H20" },
    { d: "M9 15 H13 V19", tone: "accent", sw: 0.8 },
  ],
  constraintPointOnCurve: [
    { d: "M3 17 A13 13 0 0 1 21 8" },
    { d: "M11.5 11.1 m-2 0 a2 2 0 1 0 4 0 a2 2 0 1 0 -4 0", tone: "accent", filled: true },
  ],
  constraintRadius: [
    { d: "M6 14 a6 6 0 1 0 12 0 a6 6 0 1 0 -12 0" },
    { d: "M12 14 m-1.3 0 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0", tone: "accent", filled: true },
    { d: "M12 14 L16.24 9.76", tone: "accent" },
    { d: "M14.1 10 L16.24 9.76 L16 11.9", tone: "accent", sw: 0.75 },
  ],
  constraintSymmetric: [
    { d: "M12 3 V21", tone: "accent", dash: "2 2.4" },
    { d: "M4 12 H9" },
    { d: "M15 12 H20" },
    { d: "M7 9.5 L9.5 12 L7 14.5", tone: "accent", sw: 0.8 },
    { d: "M17 9.5 L14.5 12 L17 14.5", tone: "accent", sw: 0.8 },
  ],
  constraintTangent: [
    { d: "M9 13 a5 5 0 1 0 10 0 a5 5 0 1 0 -10 0" },
    { d: "M3 8 H21" },
    { d: "M14 8 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
  ],
  constraintVertical: [
    { d: "M12 4 V20" },
    { d: "M12 4 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", filled: true },
    { d: "M12 20 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", filled: true },
    { d: "M16 9 V15", tone: "accent", sw: 0.8 },
  ],
  delete: [
    { d: "M4 7 H20" },
    { d: "M9 7 V5 H15 V7" },
    { d: "M6 7 L7 20 H17 L18 7" },
    { d: "M10 11 V16 M14 11 V16", tone: "accent", sw: 0.8 },
  ],
  dimension: [
    { d: "M4 6 V14" },
    { d: "M20 6 V14" },
    { d: "M4 10 H20", tone: "accent" },
    { d: "M7 7.5 L4 10 L7 12.5", tone: "accent" },
    { d: "M17 7.5 L20 10 L17 12.5", tone: "accent" },
  ],
  draft: [
    { d: "M7 20 L10 5 H14 L17 20 Z", sw: 0.875 },
    { d: "M10 5 V20", tone: "accent", sw: 0.875, dash: "2 2" },
    { d: "M10 13 A7 7 0 0 0 8.6 17", tone: "accent", sw: 0.7 },
  ],
  duplicate: [
    { d: "M4 4 H14 V6 M4 4 V16 H6" },
    { d: "M9 9 H20 V20 H9 Z" },
    { d: "M14.5 12.5 V16.5 M12.5 14.5 H16.5", tone: "accent", sw: 0.8 },
  ],
  edit: [
    { d: "M4 20 L4.8 16 L15 5.8 L18.2 9 L8 19.2 Z" },
    { d: "M14 6.8 L17.2 10", tone: "accent", sw: 0.8 },
  ],
  ellipse: [
    { d: "M3 12 a9 6 0 1 0 18 0 a9 6 0 1 0 -18 0" },
    { d: "M6.5 12 H17.5", tone: "accent", sw: 0.7 },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  extend: [
    { d: "M4 15 H11" },
    { d: "M19 5 V20" },
    { d: "M11 15 H18", tone: "accent" },
    { d: "M15 12 L18.5 15 L15 18", tone: "accent" },
  ],
  extrude: [
    { d: "M9 12 H17 V20 H9 Z" },
    { d: "M9 12 L12 9 H20 L17 12" },
    { d: "M17 12 L20 9 V17 L17 20" },
    { d: "M4 20 V9", tone: "accent" },
    { d: "M1.8 11 L4 8.8 L6.2 11", tone: "accent" },
  ],
  eyeOff: [
    { d: "M6.5 6.7 C4.6 7.9 3.1 9.7 2 12 C5 17.5 8.5 19 12 19 C13.4 19 14.8 18.8 16.1 18.2" },
    { d: "M9.6 5.4 C10.4 5.1 11.2 5 12 5 C15.5 5 19 6.5 22 12 C21.1 13.6 20.2 14.9 19.2 15.9" },
    { d: "M10.2 10.2 A2.6 2.6 0 0 0 13.9 13.8" },
    { d: "M4 4 L20 20", tone: "accent" },
  ],
  eyeOn: [
    { d: "M2 12 C5 6.5 8.5 5 12 5 C15.5 5 19 6.5 22 12 C19 17.5 15.5 19 12 19 C8.5 19 5 17.5 2 12 Z" },
    { d: "M12 12 m-2.6 0 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0", tone: "accent", filled: true },
  ],
  fillet: [
    { d: "M5 4 V12" },
    { d: "M12 19 H20" },
    { d: "M5 12 A7 7 0 0 0 12 19", tone: "accent" },
  ],
  filletSolid: [
    { d: "M6 11 H14 V20 H6 Z", sw: 0.875 },
    { d: "M6 11 L10 8 H18 L14 11", sw: 0.875 },
    { d: "M18 8 V17 L14 20", sw: 0.875 },
    { d: "M11 11 A3 3 0 0 1 14 14", tone: "accent", sw: 0.875 },
  ],
  fit: [
    { d: "M4 8 V4 H8" },
    { d: "M16 4 H20 V8" },
    { d: "M20 16 V20 H16" },
    { d: "M8 20 H4 V16" },
    { d: "M9 9 H15 V15 H9 Z", tone: "accent", sw: 0.8 },
  ],
  folder: [
    { d: "M3 7 H9 L11 9.5 H21 V19 H3 Z" },
    { d: "M3 7 H9 L11 9.5", tone: "accent", sw: 0.8 },
  ],
  grid: [
    { d: "M4 4 H10 V10 H4 Z", tone: "accent", filled: true },
    { d: "M4 4 H20 V20 H4 Z" },
    { d: "M10 4 V20 M16 4 V20 M4 10 H20 M4 16 H20", sw: 0.7 },
  ],
  help: [
    { d: "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0" },
    { d: "M9.3 9.2 A2.8 2.8 0 0 1 14.6 10.3 C14.6 12.4 12 12.3 12 14.3", tone: "accent", sw: 0.8 },
    { d: "M12 17.4 m-1.1 0 a1.1 1.1 0 1 0 2.2 0 a1.1 1.1 0 1 0 -2.2 0", tone: "accent", filled: true },
  ],
  home: [
    { d: "M3 11 L12 4 L21 11" },
    { d: "M5 10 V20 H19 V10" },
    { d: "M10 20 V14 H14 V20", tone: "accent", sw: 0.8 },
  ],
  import: [
    { d: "M4 14 V20 H20 V14" },
    { d: "M12 3 V14", tone: "accent" },
    { d: "M8 10 L12 14 L16 10", tone: "accent" },
  ],
  isolate: [
    { d: "M4 4 H20 V20 H4 Z", dash: "2.6 2.2" },
    { d: "M9 9 H15 V15 H9 Z", tone: "accent", filled: true },
  ],
  line: [
    { d: "M5 18 L19 6" },
    { d: "M5 18 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M19 6 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  lock: [
    { d: "M6 11 H18 V20 H6 Z" },
    { d: "M8.5 11 V8 A3.5 3.5 0 0 1 15.5 8 V11" },
    { d: "M12 14 V17", tone: "accent", sw: 0.9 },
  ],
  loft: [
    { d: "M8 5 a4 1.6 0 1 0 8 0 a4 1.6 0 1 0 -8 0" },
    { d: "M5 19 a7 2 0 1 0 14 0 a7 2 0 1 0 -14 0" },
    { d: "M8 5 L5 19", tone: "accent", sw: 0.8 },
    { d: "M16 5 L19 19", tone: "accent", sw: 0.8 },
  ],
  material: [
    { d: "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0" },
    { d: "M8.5 8 A3.2 3.2 0 0 0 8 11.5", tone: "accent", sw: 0.8 },
  ],
  menu: [
    { d: "M4 7 H20" },
    { d: "M4 12 H20" },
    { d: "M4 17 H20" },
  ],
  mirror: [
    { d: "M12 3 V21", tone: "accent", dash: "2 2.4" },
    { d: "M4 7 L8.5 12 L4 17 Z" },
    { d: "M20 7 L15.5 12 L20 17 Z" },
  ],
  move: [
    { d: "M12 3 V21" },
    { d: "M3 12 H21" },
    { d: "M9 6 L12 3 L15 6", tone: "accent" },
    { d: "M9 18 L12 21 L15 18", tone: "accent" },
    { d: "M6 9 L3 12 L6 15", tone: "accent" },
    { d: "M18 9 L21 12 L18 15", tone: "accent" },
  ],
  offset: [
    { d: "M3 12 a9 6 0 1 0 18 0 a9 6 0 1 0 -18 0" },
    { d: "M7 12 a5 3.2 0 1 0 10 0 a5 3.2 0 1 0 -10 0", tone: "accent", sw: 0.8 },
  ],
  orbit: [
    { d: "M12 12 m-3 0 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0" },
    { d: "M4 12 a8 3.4 0 1 0 16 0 a8 3.4 0 1 0 -16 0", tone: "accent", sw: 0.8, transform: "rotate(-28 12 12)" },
    { d: "M18.4 6.6 L20.4 7.4 L19.2 9.3", tone: "accent", sw: 0.8 },
  ],
  orthoPersp: [
    { d: "M3 8 H10 V17 H3 Z" },
    { d: "M14 6 L21 8.5 V15.5 L14 18 Z", tone: "accent" },
  ],
  overflow: [
    { d: "M12 4 a2 2 0 1 0 0 4 a2 2 0 1 0 0 -4", filled: true },
    { d: "M12 10 a2 2 0 1 0 0 4 a2 2 0 1 0 0 -4", filled: true },
    { d: "M12 16 a2 2 0 1 0 0 4 a2 2 0 1 0 0 -4", filled: true },
  ],
  pan: [
    { d: "M12 4 V20" },
    { d: "M4 12 H20" },
    { d: "M9.5 6.5 L12 4 L14.5 6.5", tone: "accent" },
    { d: "M9.5 17.5 L12 20 L14.5 17.5", tone: "accent" },
    { d: "M6.5 9.5 L4 12 L6.5 14.5", tone: "accent" },
    { d: "M17.5 9.5 L20 12 L17.5 14.5", tone: "accent" },
  ],
  patternCircular: [
    { d: "M4 14 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0", tone: "accent", sw: 0.8, dash: "2.4 2" },
    { d: "M10.6 14 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
    { d: "M10 4 H14 V8 H10 Z", sw: 0.875 },
    { d: "M17.5 9 H21.5 V13 H17.5 Z", sw: 0.75 },
    { d: "M2.5 9 H6.5 V13 H2.5 Z", sw: 0.75 },
  ],
  patternLinear: [
    { d: "M3 6 H7 V14 H3 Z", sw: 0.875 },
    { d: "M10 6 H14 V14 H10 Z", sw: 0.75 },
    { d: "M17 6 H21 V14 H17 Z", sw: 0.75 },
    { d: "M3 19 H20", tone: "accent", sw: 0.75 },
    { d: "M17 16.5 L20 19 L17 21.5", tone: "accent", sw: 0.75 },
  ],
  plane: [
    { d: "M4 10 L13 6 L20 11 L11 15 Z" },
    { d: "M12 10.5 V3", tone: "accent", sw: 0.8 },
    { d: "M12 10.5 m-1.3 0 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0", tone: "accent", filled: true },
  ],
  polyline: [
    { d: "M4 16 L10 8 L14 14 L20 5" },
    { d: "M4 16 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
    { d: "M20 5 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
    { d: "M10 8 m-1.2 0 a1.2 1.2 0 1 0 2.4 0 a1.2 1.2 0 1 0 -2.4 0", tone: "accent", filled: true },
    { d: "M14 14 m-1.2 0 a1.2 1.2 0 1 0 2.4 0 a1.2 1.2 0 1 0 -2.4 0", tone: "accent", filled: true },
  ],
  projectPlaceholder: [
    { d: "M4 5 H20 V19 H4 Z" },
    { d: "M4 15 L9 10.5 L13 14 L16 11.5 L20 15" },
    { d: "M8.5 9 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  pushpull: [
    { d: "M4 7 H11 V17 H4 Z", sw: 0.875 },
    { d: "M11 12 H21", tone: "accent", sw: 0.875 },
    { d: "M18 9 L21 12 L18 15", tone: "accent", sw: 0.875 },
    { d: "M14 9 L11 12 L14 15", tone: "accent", sw: 0.875 },
  ],
  rectangle: [
    { d: "M5 6 H19 V18 H5 Z" },
    { d: "M5 6 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M19 18 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  redo: [
    { d: "M19 11 H10 A5 5 0 1 0 15 20" },
    { d: "M16 7 L20 11 L16 15", tone: "accent" },
  ],
  revolve: [
    { d: "M8 8 a5 1.8 0 1 0 10 0 a5 1.8 0 1 0 -10 0", sw: 0.875 },
    { d: "M8 8 V17", sw: 0.875 },
    { d: "M18 8 V17", sw: 0.875 },
    { d: "M8 17 a5 1.8 0 0 0 10 0", sw: 0.875 },
    { d: "M13 3 V21", tone: "accent", sw: 0.875, dash: "2 2.4" },
    { d: "M4.6 5.4 a3.4 1.3 0 1 1 5.6 -0.9", tone: "accent", sw: 0.8 },
    { d: "M3.4 3 L4.4 5.6 L6.6 4.4", tone: "accent", sw: 0.8 },
  ],
  rotate: [
    { d: "M20 12 A8 8 0 1 1 15.5 4.8", tone: "accent" },
    { d: "M15.5 1 L16 5 L12 5.2", tone: "accent" },
    { d: "M12 12 m-1.6 0 a1.6 1.6 0 1 0 3.2 0 a1.6 1.6 0 1 0 -3.2 0", filled: true },
  ],
  save: [
    { d: "M5 4 H16 L20 8 V20 H5 Z" },
    { d: "M8 4 V9 H15 V4" },
    { d: "M8 20 V14 H16 V20", tone: "accent", sw: 0.8 },
  ],
  scale: [
    { d: "M4 9 V4 H9" },
    { d: "M20 15 V20 H15" },
    { d: "M16 8 L8 16", tone: "accent" },
    { d: "M12 8 H16 V12", tone: "accent" },
    { d: "M12 16 H8 V12", tone: "accent" },
  ],
  search: [
    { d: "M11 11 m-7 0 a7 7 0 1 0 14 0 a7 7 0 1 0 -14 0" },
    { d: "M16 16 L21 21", tone: "accent" },
  ],
  select: [
    { d: "M5 4 L5 18 L9 14.5 L11.5 20 L13.5 19 L11 13.5 L16 13.5 Z" },
    { d: "M5 4 L5 11", tone: "accent" },
  ],
  settings: [
    { d: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z", sw: 0.95 },
    { d: "M9 12 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0", tone: "accent", sw: 0.95 },
  ],
  shell: [
    { d: "M6 12 H14 V20 H6 Z", sw: 0.875 },
    { d: "M6 12 L10 9 H18 L14 12", sw: 0.875 },
    { d: "M18 9 V17 L14 20", sw: 0.875 },
    { d: "M7.4 11.6 L13.6 11.6 L16.6 9.4 L11 9.4 Z", tone: "accent", sw: 0.75 },
  ],
  sketch: [
    { d: "M4 20 H20", tone: "accent" },
    { d: "M20 20 m-1.3 0 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0", tone: "accent", filled: true },
    { d: "M16.5 3.5 a2.12 2.12 0 0 1 3 3 L7.5 18 L3.5 19 L4.5 15 Z" },
    { d: "M14.5 5.5 L17.5 8.5" },
  ],
  snap: [
    { d: "M12 3 V8 M12 16 V21 M3 12 H8 M16 12 H21" },
    { d: "M12 12 m-2.6 0 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0", tone: "accent", filled: true },
  ],
  sweep: [
    { d: "M5 19 C5 10 15 13 19 5", tone: "accent" },
    { d: "M16 4 L19.4 5 L18.2 8.2", tone: "accent" },
    { d: "M3 17 H7 V21 H3 Z" },
  ],
  trim: [
    { d: "M3.9 7 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0" },
    { d: "M3.9 17 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0" },
    { d: "M8.7 8.7 L20 17", tone: "accent" },
    { d: "M8.7 15.3 L20 7", tone: "accent" },
    { d: "M13.2 12 m-1 0 a1 1 0 1 0 2 0 a1 1 0 1 0 -2 0", filled: true },
  ],
  undo: [
    { d: "M5 11 H14 A5 5 0 1 1 9 20" },
    { d: "M8 7 L4 11 L8 15", tone: "accent" },
  ],
  unlock: [
    { d: "M6 11 H18 V20 H6 Z" },
    { d: "M8.5 11 V8 A3.5 3.5 0 0 1 15.5 8" },
    { d: "M12 14 V17", tone: "accent", sw: 0.9 },
  ],
  viewFront: [
    { d: "M6 10 H14 V19 H6 Z", tone: "accent", filled: true },
    { d: "M6 10 L11 7 H19 L14 10", sw: 0.875 },
    { d: "M19 7 V16 L14 19", sw: 0.875 },
    { d: "M6 10 H14 V19 H6 Z", sw: 0.875 },
  ],
  viewIso: [
    { d: "M6 10 H14 V19 H6 Z", sw: 0.875 },
    { d: "M6 10 L11 7 H19 L14 10", sw: 0.875 },
    { d: "M19 7 V16 L14 19", sw: 0.875 },
    { d: "M14 10 V19", sw: 0.875 },
    { d: "M14 10 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],
  viewRight: [
    { d: "M14 10 L19 7 V16 L14 19 Z", tone: "accent", filled: true },
    { d: "M6 10 H14 V19 H6 Z", sw: 0.875 },
    { d: "M6 10 L11 7 H19 L14 10", sw: 0.875 },
    { d: "M19 7 V16 L14 19", sw: 0.875 },
  ],
  viewTop: [
    { d: "M6 10 L11 7 L19 7 L14 10 Z", tone: "accent", filled: true },
    { d: "M6 10 H14 V19 H6 Z", sw: 0.875 },
    { d: "M6 10 L11 7 H19 L14 10", sw: 0.875 },
    { d: "M19 7 V16 L14 19", sw: 0.875 },
  ],
  zoomIn: [
    { d: "M10 10 m-6 0 a6 6 0 1 0 12 0 a6 6 0 1 0 -12 0" },
    { d: "M14.5 14.5 L20 20" },
    { d: "M10 7 V13 M7 10 H13", tone: "accent", sw: 0.8 },
  ],
  zoomOut: [
    { d: "M10 10 m-6 0 a6 6 0 1 0 12 0 a6 6 0 1 0 -12 0" },
    { d: "M14.5 14.5 L20 20" },
    { d: "M7 10 H13", tone: "accent", sw: 0.8 },
  ],
  paperPencil: [
    { d: "M6 3 H13 L18 8 V21 H6 Z" },
    { d: "M13 3 V8 H18" },
    { d: "M9 17 L11 15 L17 9 L19 11 L13 17 L11 19 L8.7 19.3 Z", tone: "accent", sw: 0.8 },
  ],
  stack: [
    { d: "M12 3 L21 8 L12 13 L3 8 Z", tone: "accent" },
    { d: "M3 12 L12 17 L21 12" },
    { d: "M3 16 L12 21 L21 16" },
  ],
} as const satisfies Record<string, IconDef>;

export type CppIconName = keyof typeof CPP_ICONS;
