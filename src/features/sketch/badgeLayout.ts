/*
 * Constraint badge layout (PURE) — maps a solved sketch to the glyphs the
 * ConstraintBadgeLayer renders (SCHEMA §7.3 constraint kinds). Each badge gets a
 * plane-coord anchor (the engine projects it to screen via HtmlOverlayDriver);
 * screen-space nudging (so the glyph floats off the geometry) is CSS, not here.
 *
 * GLYPH STANDOFF (adversarial-review M3 follow-up). A non-dimensional badge's
 * raw anchor sits ON the entity (a line's midpoint, a circle's centre) — with
 * no world-space clearance, a `Fixed` glyph at a line's midpoint out-ranked
 * the line ITSELF in `elementsFromPoint`, silently eating a select-tool click
 * meant for the curve underneath (e2e/acceptance.spec.ts's delete-round-trip
 * step). `glyphStandoff` shifts a glyph badge's `at` a fixed few mm off that
 * raw anchor, perpendicular to its own axis — see {@link GLYPH_STANDOFF_MM}
 * for the exact value and its derivation. DIMENSIONAL badges are NOT shifted:
 * they already float off the geometry (`BADGE_OFFSET_PX` in
 * ConstraintBadgeLayer.tsx) and stay a deliberate click target in every tool.
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
 * Plane-space (mm) clearance a GLYPH badge's anchor is nudged off the raw
 * entity point, perpendicular to its own axis.
 *
 * Sized to DOMINATE `HtmlOverlayDriver`'s own screen-space nudge (glyph
 * badges pass `axisFrom`+`BADGE_OFFSET_PX` to `overlay.register` for the
 * leader-line/perpendicular treatment — see ConstraintBadgeLayer.tsx), NOT
 * merely to add to it: rotating a vector in PLANE space then projecting it is
 * NOT the same operation as projecting it then rotating in SCREEN space —
 * for a camera view where they disagree, this bake and the driver's own
 * nudge land on OPPOSITE sides instead of compounding (measured: a 4mm bake,
 * on `e2e/acceptance.spec.ts`'s default framing, netted only ~4px of ACTUAL
 * on-screen clearance instead of the ~19px the plane-space math alone
 * predicted — the driver's own fixed 20px counter-nudge, in this specific
 * view, ran almost exactly the other way and cancelled nearly all of it).
 * The driver's contribution is a FIXED CSS magnitude regardless of camera —
 * `BADGE_OFFSET_PX` (10) near-edge clearance + half the 20px badge box (10)
 * — so a bake that clears `16px target + 20px worst-case opposition = 36px`
 * on its own is safe in EVERY view, not just the ones where the two happen
 * to agree. At the same measured ~4.74 px/mm (`badgeLayout.test.ts` pins
 * this against a live-camera probe of that same test's rectangle), 10mm ⇒
 * ~47px alone, ~27px worst-case net — comfortably past both thresholds.
 */
const GLYPH_STANDOFF_MM = 10;

/**
 * Nudge a glyph badge's raw anchor `at` — AND `axisFrom` by the identical
 * plane-space delta — {@link GLYPH_STANDOFF_MM} off the entity, perpendicular
 * to the axis. Shifting BOTH points (not just `at`) keeps `at − axisFrom`
 * (and therefore whatever `HtmlOverlayDriver` derives from it) EXACTLY as it
 * was before this bake — see {@link GLYPH_STANDOFF_MM} for why that
 * independence, rather than trying to align with it, is the point. Returns
 * both points unmoved when there is no axis to take a direction from (a bare
 * `Point` has none) — the driver's own (small, ≤20px) fallback still applies
 * on top of that case, same as before this change.
 */
function glyphStandoff(
  at: Point2,
  axisFrom: Point2 | undefined,
): { at: Point2; axisFrom: Point2 | undefined } {
  if (!axisFrom) return { at, axisFrom };
  const ax = at.x - axisFrom.x;
  const ay = at.y - axisFrom.y;
  const len = Math.hypot(ax, ay);
  if (len < 1e-9) return { at, axisFrom };
  const dx = (ay / len) * GLYPH_STANDOFF_MM;
  const dy = (-ax / len) * GLYPH_STANDOFF_MM;
  return {
    at: { x: at.x + dx, y: at.y + dy },
    axisFrom: { x: axisFrom.x + dx, y: axisFrom.y + dy },
  };
}

function badgeFor(c: SketchConstraint, byId: Map<string, SketchEntity>): ConstraintBadge | null {
  const first = byId.get(c.entities[0]);
  if (!first) return null;

  switch (c.type) {
    case "Horizontal":
    case "Vertical":
    case "Coincident": {
      const position = c.positions?.[0] ?? "Start";
      const rawAt = c.type === "Coincident" ? entityPointCoord(first, position) : entityAnchor(first);
      // A Coincident badge sits AT the shared point, so it would otherwise sit
      // directly on top of the vertex marker there — offset it along the
      // entity's own axis, away from that point, same as every other badge.
      const rawAxisFrom = c.type === "Coincident" ? axisFromAwayFrom(first, position) : axisFromFor(first);
      if (!rawAt) return null;
      const { at, axisFrom } = glyphStandoff(rawAt, rawAxisFrom);
      return {
        id: c.id,
        glyph: CONSTRAINT_PRESENTATION[c.type].glyph,
        kind: c.type,
        at,
        editable: false,
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
      const rawAt = entityAnchor(first);
      if (!rawAt) return null;
      const { at, axisFrom } = glyphStandoff(rawAt, axisFromFor(first));
      return {
        id: c.id,
        glyph: CONSTRAINT_PRESENTATION[c.type].glyph,
        kind: c.type,
        at,
        editable: false,
        offsetIndex: 0,
        axisFrom,
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
