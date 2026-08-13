/*
 * FROZEN inspector-section contract — the ordered section labels the inspector
 * renders per selection state, as shipped before the Platform refactor.
 *
 * Section ORDER is the invariant the refactor threatens: once sections become
 * registered contributions, ordering could silently follow registration time
 * instead of an explicit priority. See ./README.md before editing.
 */

/*
 * AMENDED 2026-08-13 (WP-VE.2) — an explicit user-visible change, recorded in
 * `TODO.md`, not a refactor going green: modeling gained a DOCUMENT-LEVEL
 * "Variables" section. It is the only section that is not about the current
 * selection, so it renders throughout Model mode and sits LAST (priority 500,
 * after Dependencies) — below everything that IS about what the user picked.
 * Sketch mode is unchanged: a sketch dimension is a solver value with no
 * `Scalar` behind it and nothing there can be bound to a variable.
 */

/** Ordered `SectionLabel` texts per inspector state. */
export const INSPECTOR_SECTIONS_CONTRACT: Readonly<Record<string, readonly string[]>> = {
  /** Model mode, nothing selected — the document-level Variables table only. */
  empty: ["Variables"],
  /** A solid body. */
  body: ["Appearance", "History", "Variables"],
  /** A promoted face (body + elementId known). */
  face: ["Appearance", "History", "Variables"],
  /** An edge — no appearance section, faces only. */
  edge: ["History", "Variables"],
  /** A sketch, which additionally carries the DOF-driven Constraints block. */
  sketch: ["History", "Constraints", "Variables"],
  /** A filled sketch region — same sections as its owning sketch. */
  sketchRegion: ["History", "Constraints", "Variables"],
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
