/*
 * FROZEN inspector-section contract — the ordered section labels the inspector
 * renders per selection state, as shipped before the Platform refactor.
 *
 * Section ORDER is the invariant the refactor threatens: once sections become
 * registered contributions, ordering could silently follow registration time
 * instead of an explicit priority. See ./README.md before editing.
 */

/** Ordered `SectionLabel` texts per inspector state. */
export const INSPECTOR_SECTIONS_CONTRACT: Readonly<Record<string, readonly string[]>> = {
  /** Model mode, nothing selected. */
  empty: [],
  /** A solid body. */
  body: ["Appearance", "History"],
  /** A promoted face (body + elementId known). */
  face: ["Appearance", "History"],
  /** An edge — no appearance section, faces only. */
  edge: ["History"],
  /** A sketch, which additionally carries the DOF-driven Constraints block. */
  sketch: ["History", "Constraints"],
  /** A filled sketch region — same sections as its owning sketch. */
  sketchRegion: ["History", "Constraints"],
  /**
   * Sketch mode. The Constraints section is unconditional — an empty sketch
   * shows the label with a "No constraints yet." body rather than hiding it, so
   * the panel does not reflow as the first constraint appears.
   */
  sketchMode: ["Constraints"],
};

/**
 * A selected feature renders History first; "Depends on" / "Used by" follow and
 * are data-driven (present only when the record has lineage), so only the
 * deterministic prefix is frozen here.
 */
export const INSPECTOR_FEATURE_PREFIX_CONTRACT: readonly string[] = ["History"];
