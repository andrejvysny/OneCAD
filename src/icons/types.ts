/*
 * Shape of one icon in the registry.
 *
 * An icon is an ORDERED list of elements, not a single path: the OneCAD icon
 * family is two-tone, and the second tone is carried by whole elements (an
 * accent-stroked arrow, an accent-filled face) rather than by any per-path
 * color string. Painting order is array order.
 */

export interface IconElement {
  /** SVG path data, in the shared 24x24 grid. */
  d: string;
  /**
   * ACCENT tone -> var(--color-icon-accent). Absent means PRIMARY tone, which
   * renders as `currentColor` so the icon still inherits its context's text
   * color. Exactly one idea per icon carries the accent (see DESIGN.md §3).
   */
  tone?: "accent";
  /**
   * Filled element (a solid dot, a shaded face) instead of a stroked one.
   * Mirrors the source SVGs' `fill=<color> stroke="none"` pairing.
   */
  filled?: boolean;
  /**
   * Stroke width as a RATIO of the FAMILY's primary weight (DESIGN.md §2), not
   * an absolute, and not relative to this icon's own root.
   *
   * Absolutes would fight the call sites, which already tune `strokeWidth` per
   * context (1.5 at 11px, 2.4 at 12px, 1.7 in the toolbar). Normalising per
   * icon would go too far the other way: the busiest glyphs (the solid-body
   * group, the view cubes) are authored a step lighter on purpose, and dividing
   * each by its own root would erase that. Against the shared base, both
   * survive — the caller scales everything, and relative weights hold.
   *
   * Absent = 1 (full primary weight).
   */
  sw?: number;
  /** stroke-dasharray, in 24-grid units (scale-free). */
  dash?: string;
  /** Element-local transform. */
  transform?: string;
}

/** One icon: its elements, in paint order. */
export type IconDef = readonly IconElement[];
