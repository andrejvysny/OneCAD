/*
 * mockRegions — MOCK region detection (single closed loop, or circles/ellipses).
 *
 * Split out of `mockSketch.ts` (bundle-splitting): this is the ONLY
 * entry-reachable importer of three's `ShapeUtils`/`Vector2`, so keeping it a
 * separate module lets `localSolver.finishSketch` load it via dynamic `import()`
 * and keeps three.js out of the app entry chunk on the real (tauri) lane, which
 * never runs this path. `mockClient.ts` still imports it statically — the mock
 * client itself is evicted from production builds by a build-time alias
 * (`vite.config.ts`), so that's fine.
 */
import type { SketchEntity, SketchRegion } from "./types";
import { ShapeUtils, Vector2 } from "three";
import {
  closedCurveOf,
  distanceToBoundary,
  fanTriangles,
  mockRegionId,
  orderedClosedLoop,
  pointInPolygon,
  signedArea,
  type ClosedCurve,
} from "./mockSketch";

// ── Region detection (MOCK — single closed loop, or circles) ─────────────────
//
// LIMITS: detects (a) each SELF-CLOSED curve (Circle or Ellipse) as its own
// region and (b) a SINGLE closed loop of lines/arcs (every vertex degree 2,
// connected). No self-intersection handling. The real worker computes proper
// regions via LoopDetector (SCHEMA §7.4 SketchRegions). regionId here is a mock
// hash, NOT the normative FNV-1a-64 over 16-byte UUIDs the worker/Rust agree on.
//
// NESTING (mock subset): a self-closed curve whose interior lies wholly INSIDE
// the closed loop creates TWO selectable planar cells: the disc, plus the
// enclosing loop with it as a hole. This matches CAD profile semantics: clicking
// the disc extrudes a cylinder; clicking the surrounding material extrudes a
// tube. One that is separated from, or crosses, the loop stays its own region.
//
// An Ellipse is treated exactly like a Circle here (W3 P3): it is closed, its
// sampled polygon has area πab, and it is star-shaped about its centre — the two
// properties the fill + nesting code actually depend on.

/**
 * Ear-clip triangulation of a closed loop with optional holes, in plane-local
 * (u,v). Output layout matches the real worker lane (SolverLane ear_clip):
 * positions are the outer ring's vertices followed by each hole ring's vertices
 * (no synthetic hub vertex), indices are ear-clip triples normalized CCW.
 *
 * Topology contract (SCHEMA §7.4): every hole-bridge diagonal earcut introduces
 * is shared by exactly two triangles, so the only single-use edges are the real
 * outer and hole boundaries — the property `prismPreview.profileFromRegion`
 * recovers the rings from. Valid for ANY simple polygon: the previous centroid
 * fan was exact only for star-shaped loops and filled OUTSIDE the boundary on
 * concave ones (fill/edge mismatch on screen, and `regionAtPoint` hit-testing
 * clicks outside the profile).
 */
function loopTriangles(
  outerIn: [number, number][],
  holesIn: [number, number][][],
): { positions: number[]; indices: number[] } {
  const outer = signedArea(outerIn) < 0 ? [...outerIn].reverse() : outerIn;
  const holes = holesIn.map((h) => (signedArea(h) > 0 ? [...h].reverse() : h));
  const contour = outer.map(([x, y]) => new Vector2(x, y));
  const holeContours = holes.map((h) => h.map(([x, y]) => new Vector2(x, y)));
  const faces = ShapeUtils.triangulateShape(contour, holeContours);

  // Flatten AFTER triangulateShape: it trims a duplicated closing point in
  // place, and earcut's indices address the trimmed rings.
  const positions: number[] = [];
  for (const ring of [contour, ...holeContours]) for (const p of ring) positions.push(p.x, p.y);

  const indices: number[] = [];
  for (const [a, b, c] of faces) {
    const cross =
      (positions[b * 2] - positions[a * 2]) * (positions[c * 2 + 1] - positions[a * 2 + 1]) -
      (positions[b * 2 + 1] - positions[a * 2 + 1]) * (positions[c * 2] - positions[a * 2]);
    // Normalize winding to CCW; keep zero-area slivers as-is (dropping one would
    // change edge-use counts and fabricate a boundary in ring recovery).
    if (cross >= 0) indices.push(a, b, c);
    else indices.push(a, c, b);
  }
  return { positions, indices };
}

export function detectRegions(entities: SketchEntity[]): SketchRegion[] {
  const regions: SketchRegion[] = [];
  const live = entities.filter((e) => !e.construction);
  const loop = orderedClosedLoop(live);

  // A self-closed curve (circle OR ellipse) wholly inside the closed loop is both a
  // selectable disc cell and a hole boundary of the enclosing cell. "Wholly inside"
  // = centre inside AND the whole curve clear of the boundary, so a crossing one
  // stays independent. `reach` (radius / semi-major) makes that clearance test
  // conservative for a rotated ellipse without sampling the boundary distance.
  const holes: ClosedCurve[] = [];
  const free: ClosedCurve[] = [];
  for (const e of live) {
    const curve = closedCurveOf(e);
    if (!curve) continue;
    const nested =
      loop !== null &&
      pointInPolygon(curve.center, loop.points) &&
      distanceToBoundary(curve.center, loop.points) > curve.reach;
    (nested ? holes : free).push(curve);
  }

  for (const c of [...free, ...holes]) {
    regions.push({
      regionId: mockRegionId([c.entity.id]),
      outerLoop: [c.entity.id],
      holes: [],
      previewTriangles: { ...fanTriangles(c.center, c.polygon), holesSubtracted: 0 },
    });
  }

  if (loop) {
    regions.push({
      // A cell's identity includes its material boundary, not only its outer
      // wire. Adding/removing a hole must invalidate an armed profile.
      regionId: mockRegionId([...loop.ids, ...holes.map((hole) => hole.entity.id)]),
      outerLoop: loop.ids,
      holes: holes.map((h) => [h.entity.id]),
      previewTriangles: {
        ...loopTriangles(
          loop.points,
          holes.map((h) => h.polygon),
        ),
        holesSubtracted: holes.length,
      },
    });
  }

  return regions;
}
