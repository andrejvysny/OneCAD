/*
 * RENDER_ORDER — the single painter's ladder for the viewport scene.
 *
 * Three.js splits objects into an OPAQUE list (rendered first, sorted by
 * renderOrder then front-to-back) and a TRANSPARENT list (rendered after ALL
 * opaque objects, sorted by renderOrder then back-to-front). renderOrder only
 * orders WITHIN a list — it can never lift an opaque object above a
 * transparent one. The viewport therefore follows two hard rules:
 *
 * 1. The grid (ground + sketch-plane) is opaque, painted first, and NEVER
 *    writes depth. It cannot occlude or z-fight anything drawn later —
 *    sketches and bodies always win. Bodies overpaint it; translucent fills
 *    blend over it.
 * 2. Everything that lies IN a sketch plane (tint, fills, curves, markers,
 *    ghosts) is coplanar and cannot be layered by the depth buffer at all
 *    (line vs. triangle rasterization produces per-pixel depth deltas →
 *    stipple punch-through). All of it renders in the transparent pass with
 *    depthWrite: false, layered purely by the ladder below. Depth TEST stays
 *    on, so solid bodies (opaque pass, depth-written, polygonOffset pushes
 *    faces back) still correctly occlude sketch content behind them.
 *
 * Ties are fine when the objects never overlap or the later look is
 * order-independent.
 */
export const RENDER_ORDER = {
  // ---- opaque pass ----
  /** Ground grid + sketch-plane grid. depthWrite MUST stay false. */
  GRID: -1,
  /** Origin triad — above the grid, depth-tested against bodies. */
  TRIAD: 0,
  // (bodies keep the three.js default of 0 and rely on depth.)

  // ---- transparent pass (drawn after every opaque object) ----
  /** Sketch-mode plane tint quad — the floor of the sketch surface. */
  SKETCH_TINT: -3,
  /** Origin-plane picker quads + outlines. */
  PLANE_PICK_FILL: -2,
  PLANE_PICK_OUTLINE: -1,
  /** Datum plane quads + outlines (DATUM W1). Shares the plane-picker tier: both
   *  are translucent reference quads and a datum is offset off the origin planes,
   *  so they never co-locate and the tie is order-independent. */
  DATUM_FILL: -2,
  DATUM_OUTLINE: -1,
  /** Static (model-mode) sketch region fills. */
  STATIC_FILL: 1,
  /** Body face highlight (hover/selection). */
  HIGHLIGHT_FACE: 2,
  /** Static sketch curves — above their fill. */
  STATIC_CURVES: 2,
  /** Body edge highlight. */
  HIGHLIGHT_EDGE: 3,
  /** Static sketch vertex dots. */
  STATIC_POINTS: 3,
  /** Selection/hover halo, BEHIND the entity's own semantic-color line (P1
   *  audit fix — selecting/hovering an entity must not erase its
   *  under/full/conflict color, only add a tint underneath it). Shares the
   *  numeric tier with STATIC_CURVES/HIGHLIGHT_FACE, which belong to a
   *  different object graph (model-mode static sketches / body faces) and
   *  never co-render with edit-mode `SketchObject` content, so there is no
   *  real collision. */
  SKETCH_CURVES_HALO: 2,
  /** Edit-mode committed entities + rubber-band preview. */
  SKETCH_CURVES: 3,
  /** Edit-mode endpoint/center markers. */
  SKETCH_POINTS: 4,
  /** Permanent dimension-line witness ticks + baseline + arrowheads, for a
   *  constrained edge (WP: sketch UX parity pass). */
  DIM_LINE: 5,
  /** Multi-region extrude/revolve pick fills. */
  REGION_FILL: 5,
  /** Trim doomed-piece ghost — above committed entities + markers. */
  TRIM_GHOST: 5,
  /** Dashed angle-preview arc (live angle chip ↔ its reference segment). */
  ANGLE_ARC_PREVIEW: 5,
  /** Snap guide lines / snap glyph marker. */
  SNAP_GUIDES: 5,
  SNAP_MARKER: 6,
  /** L1/L2 model-tool previews. */
  PREVIEW_MESH: 6,
  GHOST: 6,
  REVOLVE_SHELL: 6,
  REVOLVE_CANDIDATES: 7,
  DRAG_HANDLE: 7,
  /** Placement gizmo (WP-B W2). Shares the drag-handle tier: both are the armed
   *  tool's own grab surface and only ONE tool is armed at a time, so they can
   *  never co-exist and the tie is unobservable. */
  TRANSFORM_GIZMO: 7,
  REVOLVE_HOVER: 8,
} as const;
