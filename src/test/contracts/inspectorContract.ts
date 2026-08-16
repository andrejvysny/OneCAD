/*
 * FROZEN inspector-section contract — the ordered section labels the inspector
 * renders per selection state, as shipped before the Platform refactor.
 *
 * Section ORDER is the invariant the refactor threatens: once sections become
 * registered contributions, ordering could silently follow registration time
 * instead of an explicit priority. See ./README.md before editing.
 */

/*
 * AMENDED 2026-08-14 (UI/UX pass) — an explicit user-visible change, recorded
 * in `TODO.md`, not a refactor going green: the document-level "Variables"
 * section (added 2026-08-13, WP-VE.2) moved OUT of the right inspector into
 * its own left-sidebar tab (`VariablesPanel`, replacing the old sidebar
 * Library tab — the library browser is now a full-size modal). It was the
 * only section not about the current selection, so removing it un-freezes
 * nothing about the selection-driven sections below — they keep their prior
 * relative order.
 */

/*
 * AMENDED 2026-08-16 (Render P1, material assignment UX) — an explicit
 * user-visible change, to be recorded in `TODO.md`'s Render P1 gate record, not
 * a refactor going green: `onecad.render` contributes a "Material" section for
 * a body and for a promoted face (`modules/render/ui/register.ts`, priority 120).
 * It sits between modeling's Appearance (100) and its History (200) — "what
 * colour is this?" and "what is this made of?" are the same question asked
 * twice, so they belong adjacent, and History stays below both. NOTHING already
 * in these lists moved relative to anything else, and no other selection state
 * gained a section: an edge, a sketch, a region and a feature wear no material,
 * exactly as they carry no Appearance block.
 */

/** Ordered `SectionLabel` texts per inspector state. */
export const INSPECTOR_SECTIONS_CONTRACT: Readonly<Record<string, readonly string[]>> = {
  /** Model mode, nothing selected — no selection-driven section applies. */
  empty: [],
  /** A solid body. */
  body: ["Appearance", "Material", "History"],
  /** A promoted face (body + elementId known). */
  face: ["Appearance", "Material", "History"],
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
