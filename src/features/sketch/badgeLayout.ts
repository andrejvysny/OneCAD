/*
 * Constraint badge layout (PURE) — maps a solved sketch to the glyphs the
 * ConstraintBadgeLayer renders (SCHEMA §7.3 constraint kinds). Each badge gets a
 * plane-coord anchor ON its entity plus a SCREEN-px standoff; the engine
 * projects the anchor and applies the standoff per frame via HtmlOverlayDriver.
 *
 * GLYPH STANDOFF (adversarial-review M3 follow-up; screen-constant since
 * SKETCH_UX_AUDIT #5's residual). A non-dimensional badge's raw anchor sits ON
 * the entity (a line's midpoint, a circle's centre) — with no clearance, a
 * `Fixed` glyph at a line's midpoint out-ranked the line ITSELF in
 * `elementsFromPoint`, silently eating a select-tool click meant for the curve
 * underneath (e2e/acceptance.spec.ts's delete-round-trip step). The standoff
 * that fixes it is a SCREEN distance, not a plane-space one: see
 * {@link GLYPH_STANDOFF_PX} for the value, the sign convention, and why the
 * mm-baked version it replaces could not hold at more than one zoom level.
 * DIMENSIONAL badges keep the smaller {@link DIMENSION_STANDOFF_PX} and the
 * opposite side — they are a deliberate click target in every tool.
 */
import type { SketchConstraint, SketchEntity, SketchSession, ConstraintPosition } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { CONSTRAINT_PRESENTATION } from "./constraintCatalog";
import { formatUnitless } from "@/units/format";
import { formatDimensionValue } from "./dimensionFormat";
import { pointCoordOf } from "@/tools/sketch/sketchTopology";

export interface ConstraintBadge {
  id: string;
  glyph: string;
  kind: SketchConstraint["type"];
  at: Point2;
  /** Dimensional constraints render an editable DimensionInput chip. */
  editable: boolean;
  /**
   * SIGNED screen-px clearance from `at` to this badge's near edge, handed
   * straight to `HtmlOverlayDriver`'s `offsetPx`. See
   * {@link GLYPH_STANDOFF_PX} / {@link DIMENSION_STANDOFF_PX}.
   */
  standoffPx: number;
  value?: number;
  /**
   * Position among badges sharing the same (quantized) anchor point — 0 for
   * the first, incrementing for each co-anchored sibling. The layer uses this
   * to stagger co-anchored badges into a row instead of stacking them.
   */
  offsetIndex: number;
  /**
   * The other end of the axis this badge sits BESIDE (`HtmlOverlayDriver`'s
   * `axisFrom`) — a line endpoint for a line-anchored badge, a point on the
   * boundary for a centre-anchored one. Undefined for a point-only badge
   * (Coincident/Fixed), which has no direction to tether a leader line to.
   * This is what makes a badge visually read as "belonging to that entity"
   * at a busy shared vertex, instead of floating at a fixed screen offset
   * that assumes an orientation the entity may not have.
   */
  axisFrom?: Point2;
}

const mid = (a: [number, number], b: [number, number]): Point2 => ({ x: (a[0] + b[0]) / 2, y: (a[1] + b[1]) / 2 });

/**
 * The (u,v) coord of an entity's named point (Start/End/Center/Midpoint).
 *
 * Thin `Point2` adapter over `sketchTopology.pointCoordOf` (SKETCH-V2 P0.5),
 * which owns the (type × position) table this used to re-implement. One
 * behaviour change: a free `Point`'s `Center` now resolves to its coordinate
 * instead of null, because `Point.Start`/`Point.Center` are two addresses for
 * one point and the alias makes that explicit.
 */
export function entityPointCoord(e: SketchEntity, position: ConstraintPosition): Point2 | null {
  const p = pointCoordOf(e, position);
  return p ? { x: p[0], y: p[1] } : null;
}

/** Representative anchor for a badge on an entity (line midpoint / circle center). */
export function entityAnchor(e: SketchEntity): Point2 | null {
  if (e.type === "Line" && e.p0 && e.p1) return mid(e.p0, e.p1);
  if ((e.type === "Circle" || e.type === "Arc" || e.type === "Ellipse") && e.center)
    return { x: e.center[0], y: e.center[1] };
  if (e.type === "Point" && e.p0) return { x: e.p0[0], y: e.p0[1] };
  return null;
}

/**
 * The other end of the axis a badge on `e` sits BESIDE — an endpoint for a
 * Line (the axis IS the line itself, so a leader from it to the midpoint
 * anchor runs ALONG the line, putting the perpendicular offset off to the
 * side), or a point on the boundary for a circular entity (radius direction
 * from its centre anchor). Undefined for a Point — it has no direction.
 */
function axisFromFor(e: SketchEntity): Point2 | undefined {
  if (e.type === "Line" && e.p0) return { x: e.p0[0], y: e.p0[1] };
  if (e.center) {
    const r = e.type === "Ellipse" ? e.majorR : e.radius;
    if (r) return { x: e.center[0] + r, y: e.center[1] };
  }
  return undefined;
}

/**
 * `axisFrom` for a badge anchored at ONE SPECIFIC named point of `e` (a
 * Coincident badge, anchored at whichever endpoint the two entities share) —
 * the entity's OTHER endpoint, so the perpendicular offset runs away from the
 * shared point along the entity's own axis instead of degenerating to a
 * zero-length axis when `axisFromFor`'s fixed endpoint happens to BE the
 * anchor itself (a Line's Start position, coincident at its own p0).
 */
function axisFromAwayFrom(e: SketchEntity, position: ConstraintPosition): Point2 | undefined {
  if (e.type === "Line" && e.p0 && e.p1) {
    const [x, y] = position === "End" ? e.p0 : e.p1;
    return { x, y };
  }
  return axisFromFor(e);
}

/**
 * Screen-px clearance from a GLYPH badge's entity to the badge's near edge —
 * `HtmlOverlayDriver` applies it per frame, perpendicular to the PROJECTED
 * axis, so it is the same distance at every zoom.
 *
 * WHY SCREEN, NOT MM. This replaces a 10mm plane-space bake on `at` itself,
 * which could only be right at one zoom: it projected to ~47px at the
 * sketch-entry camera settle (measured — a face-on XY sketch, viewport 722px
 * tall, camera distance 97.51 at fov 76 ⇒ 4.74 px/mm) and to ~117px after one
 * wheel-zoom in, while the driver's own screen-space nudge stayed a fixed 22px
 * the OTHER way (rotating in plane space then projecting is not the same
 * operation as projecting then rotating in screen space, and for the sketch
 * camera the two are exact opposites). Net on-screen standoff therefore ran
 * from ~24px at entry to ~94px zoomed in — crowding the curve at one end (that
 * 24px centre left only ~14px of clearance to the badge's own edge, inside the
 * corridor below) and drifting away from it at the other. Both effects are gone
 * once the standoff IS a screen distance.
 *
 * MAGNITUDE. 16px is twice `SketchController.pickTol`'s 8px (SNAP_PX) select
 * corridor around the curve, and — because the driver's `offsetPx` is the
 * clearance to the NEAR EDGE, not the centre — it holds whatever the badge's
 * own box measures. Measured after the change: near edge 16px, badge centre
 * 27px, identical at both zooms above (the mm bake read 24px at entry), so the
 * default-zoom look survives within ~3px.
 *
 * SIGN. Negative = the side OPPOSITE the driver's default (+90° screen)
 * perpendicular, which for the sketch camera — always looking at the plane
 * from its +normal side (`CadOrbitControls.viewAlongNormal`) — is the side the
 * old plane-space bake projected to. Keeping it is not nostalgia: DIMENSIONAL
 * chips use the positive side, so a glyph and a dimension on the SAME entity
 * land on opposite sides of it instead of stacking.
 */
export const GLYPH_STANDOFF_PX = 16;

/**
 * The same clearance for a DIMENSIONAL chip, on the driver's default side.
 * Smaller than {@link GLYPH_STANDOFF_PX}: a value pill reads as a label of the
 * curve it sits beside, and it is a click target in every tool rather than
 * something that must stay clear of one.
 */
export const DIMENSION_STANDOFF_PX = 10;

function badgeFor(c: SketchConstraint, byId: Map<string, SketchEntity>): ConstraintBadge | null {
  const first = byId.get(c.entities[0]);
  if (!first) return null;

  switch (c.type) {
    case "Horizontal":
    case "Vertical":
    case "Coincident": {
      const position = c.positions?.[0] ?? "Start";
      const at = c.type === "Coincident" ? entityPointCoord(first, position) : entityAnchor(first);
      // A Coincident badge sits AT the shared point, so it would otherwise sit
      // directly on top of the vertex marker there — offset it along the
      // entity's own axis, away from that point, same as every other badge.
      const axisFrom = c.type === "Coincident" ? axisFromAwayFrom(first, position) : axisFromFor(first);
      if (!at) return null;
      return {
        id: c.id,
        glyph: CONSTRAINT_PRESENTATION[c.type].glyph,
        kind: c.type,
        at,
        editable: false,
        standoffPx: -GLYPH_STANDOFF_PX,
        offsetIndex: 0,
        axisFrom,
      };
    }
    case "Parallel":
    case "Perpendicular":
    case "Tangent":
    case "Concentric":
    case "Equal":
    case "Midpoint":
    case "OnCurve":
    case "Symmetric":
    case "Fixed": {
      const at = entityAnchor(first);
      if (!at) return null;
      return {
        id: c.id,
        glyph: CONSTRAINT_PRESENTATION[c.type].glyph,
        kind: c.type,
        at,
        editable: false,
        standoffPx: -GLYPH_STANDOFF_PX,
        offsetIndex: 0,
        axisFrom: axisFromFor(first),
      };
    }
    case "Distance":
    case "HorizontalDistance":
    case "VerticalDistance":
    case "Angle":
    case "Radius":
    case "Diameter": {
      const at = entityAnchor(first);
      if (!at) return null;
      const value = c.value ?? 0;
      // An Angle is DEGREES and must never see the length display unit; every
      // other dimensional kind is a length in mm and renders in it (WP-C2).
      const glyph =
        c.type === "Angle" ? `${formatUnitless(value)}°` : formatDimensionValue(value);
      return {
        id: c.id,
        glyph,
        kind: c.type,
        at,
        editable: true,
        value,
        standoffPx: DIMENSION_STANDOFF_PX,
        offsetIndex: 0,
        axisFrom: axisFromFor(first),
      };
    }
    default:
      return null;
  }
}

/** Quantize an anchor to a stable grouping key — co-anchored badges (same
 *  entity midpoint/center) round-trip float coords identically, but this
 *  guards against float noise between two badges that are "the same" anchor. */
const ANCHOR_QUANT = 1e-4;
export function anchorKey(at: Point2): string {
  const q = (n: number) => Math.round(n / ANCHOR_QUANT) * ANCHOR_QUANT;
  return `${q(at.x)},${q(at.y)}`;
}

/** Assigns `offsetIndex` to badges sharing an anchor (U9) so
 *  ConstraintBadgeLayer can stagger them into a row instead of stacking them.
 *  Deterministic: badges are sorted by id within each anchor group before
 *  indexing, independent of constraint iteration order. */
function assignOffsets(badges: ConstraintBadge[]): ConstraintBadge[] {
  const groups = new Map<string, ConstraintBadge[]>();
  for (const b of badges) {
    const key = anchorKey(b.at);
    const group = groups.get(key);
    if (group) group.push(b);
    else groups.set(key, [b]);
  }
  for (const group of groups.values()) {
    group.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    group.forEach((b, i) => {
      b.offsetIndex = i;
    });
  }
  return badges;
}

export function layoutBadges(session: SketchSession | null): ConstraintBadge[] {
  if (!session) return [];
  const byId = new Map(session.entities.map((e) => [e.id, e]));
  const out: ConstraintBadge[] = [];
  for (const c of session.constraints) {
    const badge = badgeFor(c, byId);
    if (badge) out.push(badge);
  }
  return assignOffsets(out);
}
