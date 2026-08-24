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
 *    depthWrite: false, layered purely by the ladder below.
 * 3. Depth TEST is on for STATIC (model-mode) sketch content, so solid bodies
 *    (opaque pass, depth-written, polygonOffset pushes faces back) occlude a
 *    sketch behind them — and OFF for every material of the ACTIVE sketch
 *    session (audit item #1, reversing what rule 2 used to say). In sketch mode
 *    the camera looks down the plane normal, so a body sitting between the eye
 *    and a coplanar sketch plane hid the user's very first stroke completely.
 *    depthTest: false + the transparent pass (which renders after ALL opaque
 *    objects) paints the active sketch over bodies unconditionally; the tiers
 *    below still order it against itself. ACCEPTED CONSEQUENCE: orbiting
 *    mid-session shows the active sketch through solids (x-ray) — the standard
 *    CAD behaviour, and the price of the stroke being visible at all. Picking
 *    is untouched: sketch hit-testing is plane math, not a raycast, and body
 *    picking reads neither renderOrder nor depthTest.
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
  /** Live closed-region fill of the ACTIVE sketch session (audit item #2) —
   *  the floor of the edit-mode stack, under its own curves and markers.
   *  Shares STATIC_FILL's number deliberately: item #9 hides every static
   *  region fill while a session is active, so the two can never co-render. */
  ACTIVE_FILL: 1,
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
   *  under/full/conflict color, only add a tint underneath it).
   *
   *  Shares the numeric tier with STATIC_CURVES, and they DO co-render: audit
   *  item #9 dims the other sketches mid-session, it does not hide them. The tie
   *  is therefore resolved by draw order (scene-graph order within the tier),
   *  which is unspecified — and acceptable here only because the loser is still
   *  legible either way: the static curve is at 0.35 emphasis and the halo is a
   *  wide, low-contrast underlay, so neither can hide the other's shape, and the
   *  entity's own SKETCH_CURVES line (tier 3) paints over both regardless. Do
   *  not put anything OPAQUE on tier 2. HIGHLIGHT_FACE is a body-face overlay
   *  and never coplanar with an edit-mode sketch, so it is a true non-collision. */
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
  /**
   * Snap guide lines, then the snap glyph marker (SNAP §10.9).
   *
   * Their OWN tiers, above every sketch curve and below the marker. They used
   * to share tier 5 with the trim ghost, the region fills and the dimension
   * lines; those DO co-render with a snap guide during an armed draw tool, and
   * a tie there is resolved by draw order — i.e. by nothing the reader can see
   * or predict. The guide must be legible ON TOP of the geometry it aligns to,
   * and the marker on top of the guide.
   */
  SNAP_GUIDES: 6,
  SNAP_MARKER: 7,
  /** L1/L2 model-tool previews. Model-mode only — a snap guide belongs to an
   *  armed SKETCH tool, so the shared numbers never co-render. */
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
