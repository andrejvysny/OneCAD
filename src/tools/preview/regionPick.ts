/*
 * Region pick hit-testing (PURE math, plane (u,v) space).
 *
 * A finished sketch may yield several closed regions; the user then picks WHICH one
 * to extrude/revolve. Each region carries `previewTriangles` — a flat (u,v) vertex
 * list + triangle indices, authored in plane coordinates exactly like the fill mesh
 * (see SketchStaticLayer / RegionPickLayer). This module answers "which region's
 * triangulation contains plane point (u,v)?" with a barycentric point-in-triangle
 * test; the controller feeds it the pointer's (u,v) (screen ray ∩ sketch plane) to
 * resolve hover + click. Kept THREE-free so it unit-tests without a renderer.
 */
import type { SketchRegion } from "@/ipc/types";

/** Inclusivity slack for shared edges / near-degenerate hits (plane units²-scaled). */
const EPS = 1e-9;

/**
 * True when (px,py) lies inside — or on an edge of — triangle abc (any winding).
 * A near-zero-area (degenerate) triangle never contains a point.
 */
export function pointInTriangle(
  px: number,
  py: number,
  ax: number,
  ay: number,
  bx: number,
  by: number,
  cx: number,
  cy: number,
): boolean {
  const d = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy); // 2·signed area
  if (Math.abs(d) < EPS) return false; // degenerate sliver — no interior
  const a = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / d;
  const b = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / d;
  const c = 1 - a - b;
  return a >= -EPS && b >= -EPS && c >= -EPS;
}

/**
 * The regionId whose `previewTriangles` contain plane point (u,v), or null when the
 * point is over no region. Regions are tested in order and the first hit wins
 * (closed profiles are disjoint, so overlap only happens exactly on a shared edge).
 */
export function regionAtPoint(regions: SketchRegion[], u: number, v: number): string | null {
  for (const r of regions) {
    const t = r.previewTriangles;
    if (!t || t.positions.length < 6 || t.indices.length < 3) continue;
    const p = t.positions;
    for (let i = 0; i + 2 < t.indices.length; i += 3) {
      const ia = t.indices[i] * 2;
      const ib = t.indices[i + 1] * 2;
      const ic = t.indices[i + 2] * 2;
      if (pointInTriangle(u, v, p[ia], p[ia + 1], p[ib], p[ib + 1], p[ic], p[ic + 1])) {
        return r.regionId;
      }
    }
  }
  return null;
}
