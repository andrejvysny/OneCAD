/*
 * Prism preview geometry (PURE math, plane-local coords) — the shared core of
 * the two-level extrude preview (NEW_SPEC §15).
 *
 * A finished sketch region carries a triangulation in plane (u,v) whose LAYOUT
 * differs by producer: the mock lane emits ear-clip triples for loop cells
 * (`loopTriangles`) but a centroid-hub fan for self-closed curves
 * (`fanTriangles`), while the real worker lane (SolverLane ear_clip) always
 * emits the outer-loop polygon vertices with ear-clip indices and NO hub. The
 * profile ring is therefore derived from triangulation topology (boundary
 * edges), never from a layout convention. We lift that flat profile into a 3D prism authored
 * in PLANE-LOCAL coordinates (x=u, y=v, z=w along the plane normal):
 *
 *   - L1 (engine): a UNIT prism (cap at w=1) built once; a group carrying the
 *     plane basis scales `z` by the live depth, so the drag runs zero-allocation.
 *   - L2 (mock backend): the exact prism at the real depth, transformed to world
 *     and encoded as MESH1 (see mockMeshes.makeExtrudeBodyMesh).
 *
 * Both consume the SAME topology from here, so L1 and the exact result agree.
 */
import type { SketchRegion } from "@/ipc/types";

/** Boundary rings (u,v) + cap triangulation, extracted from a region. */
export interface PrismProfile {
  /** OUTER boundary ring in plane (u,v), CCW, no repeated closing point. */
  ring: [number, number][];
  /**
   * Hole boundary rings, each CW (opposite the outer ring) so the shared wall
   * builder emits inward-facing triangles without a special case. Empty when the
   * region has no holes, or when the producer does not subtract them.
   */
  holes: [number, number][][];
  /** Cap triangulation: `positions` = flat (u,v) pairs, `indices` = triples. */
  cap: { positions: number[]; indices: number[] };
}

/** (u,v) bounds + centroid of a region's profile — sizes the handle + the mock body. */
export interface RegionBounds {
  minU: number;
  maxU: number;
  minV: number;
  maxV: number;
  centroidU: number;
  centroidV: number;
}

/**
 * Extract the prism profile from a region's `previewTriangles`.
 *
 * The two producers emit DIFFERENT triangulation layouts:
 *   - mock lane: a fan — positions[0..1] is a centroid hub, the rest is the
 *     ordered boundary ring;
 *   - real worker lane (SolverLane ear_clip): positions are the outer-loop
 *     polygon vertices in loop order with NO hub, indices are ear-clip triples.
 * So the ring must be derived from the triangulation TOPOLOGY, not a layout
 * convention: boundary edges (used by exactly one triangle) chained into a loop
 * and oriented CCW. Works for both layouts. Returns null when the region has no
 * usable triangulation (nothing to extrude).
 */
export function profileFromRegion(region: SketchRegion): PrismProfile | null {
  const tris = region.previewTriangles;
  if (!tris || tris.positions.length < 6 || tris.indices.length < 3) return null;
  // A partial fill is not the selected planar cell. Showing/extruding it would
  // silently add material inside a declared hole, so fail closed.
  if ((tris.holesSubtracted ?? 0) < region.holes.length) return null;

  // 1) Boundary edges: count undirected edge uses; keep the DIRECTED form of
  //    the single-use ones (triangle winding gives the loop direction).
  const uses = new Map<string, { a: number; b: number; count: number }>();
  for (let i = 0; i + 2 < tris.indices.length; i += 3) {
    const t = [tris.indices[i], tris.indices[i + 1], tris.indices[i + 2]];
    for (let k = 0; k < 3; k++) {
      const a = t[k];
      const b = t[(k + 1) % 3];
      const key = a < b ? `${a}|${b}` : `${b}|${a}`;
      const e = uses.get(key);
      if (e) e.count++;
      else uses.set(key, { a, b, count: 1 });
    }
  }
  const next = new Map<number, number>();
  for (const e of uses.values()) {
    if (e.count === 1) next.set(e.a, e.b);
  }
  if (next.size < 3) return null;

  // 2) Chain the directed boundary edges into loops. The largest-|area| loop is
  //    the OUTER boundary; the rest are holes. A hole-subtracting producer (the
  //    real SolverLane fill) yields one loop per hole here — which is exactly why
  //    its bridge segments must stay INTERIOR to the triangulation (see
  //    worker/src/loop/PolygonFill.h): a bridge counted as a boundary edge would
  //    chain into a spurious loop and fabricate a wall that is not there.
  const visited = new Set<number>();
  const loops: { indices: number[]; area: number }[] = [];
  for (const start of next.keys()) {
    if (visited.has(start)) continue;
    const loop: number[] = [];
    let cur: number | undefined = start;
    while (cur !== undefined && !visited.has(cur)) {
      visited.add(cur);
      loop.push(cur);
      cur = next.get(cur);
    }
    if (cur !== start || loop.length < 3) continue; // open chain / degenerate
    let area = 0;
    for (let i = 0; i < loop.length; i++) {
      const [ax, ay] = pointAt(tris.positions, loop[i]);
      const [bx, by] = pointAt(tris.positions, loop[(i + 1) % loop.length]);
      area += ax * by - bx * ay;
    }
    loops.push({ indices: loop, area: area / 2 });
  }
  if (loops.length === 0) return null;

  let outerIdx = 0;
  for (let i = 1; i < loops.length; i++) {
    if (Math.abs(loops[i].area) > Math.abs(loops[outerIdx].area)) outerIdx = i;
  }
  const outer = loops[outerIdx];
  if (outer.area < 0) outer.indices.reverse(); // prismLocal assumes a CCW ring

  const holes: [number, number][][] = [];
  for (let i = 0; i < loops.length; i++) {
    if (i === outerIdx) continue;
    const h = loops[i];
    if (h.area > 0) h.indices.reverse(); // holes wind CW, opposite the outer ring
    holes.push(h.indices.map((idx) => pointAt(tris.positions, idx)));
  }
  if (holes.length !== region.holes.length) return null;

  const ring: [number, number][] = outer.indices.map((idx) => pointAt(tris.positions, idx));
  return { ring, holes, cap: { positions: [...tris.positions], indices: [...tris.indices] } };
}

function pointAt(positions: number[], idx: number): [number, number] {
  return [positions[idx * 2], positions[idx * 2 + 1]];
}

/** Polygon area centroid of the ring (layout-agnostic — no fan-hub assumption). */
export function ringCentroid(ring: [number, number][]): [number, number] {
  let a2 = 0; // 2×signed area
  let cx = 0;
  let cy = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x0, y0] = ring[i];
    const [x1, y1] = ring[(i + 1) % ring.length];
    const cross = x0 * y1 - x1 * y0;
    a2 += cross;
    cx += (x0 + x1) * cross;
    cy += (y0 + y1) * cross;
  }
  if (Math.abs(a2) < 1e-12) {
    // Degenerate polygon — fall back to the vertex mean.
    let mx = 0;
    let my = 0;
    for (const [x, y] of ring) {
      mx += x;
      my += y;
    }
    return [mx / ring.length, my / ring.length];
  }
  return [cx / (3 * a2), cy / (3 * a2)];
}

/** (u,v) bounds + centroid of a profile. */
export function profileBounds(profile: PrismProfile): RegionBounds {
  let minU = Infinity,
    maxU = -Infinity,
    minV = Infinity,
    maxV = -Infinity;
  for (const [u, v] of profile.ring) {
    if (u < minU) minU = u;
    if (u > maxU) maxU = u;
    if (v < minV) minV = v;
    if (v > maxV) maxV = v;
  }
  // Area centroid of the ring — cap.positions[0] is a centroid hub ONLY in the
  // mock fan layout; on the real (ear-clip) lane it is an arbitrary corner.
  const [centroidU, centroidV] = ringCentroid(profile.ring);
  return { minU, maxU, minV, maxV, centroidU, centroidV };
}

/** A faceted prism in plane-local coords (x=u, y=v, z=w along normal). */
export interface PrismLocal {
  /** Flat (x,y,z) vertex triples. */
  positions: number[];
  /** Flat (x,y,z) per-vertex normals (same length as positions). */
  normals: number[];
  /** Faces in emission order; triangles index `positions`. bottom, top, side. */
  faces: { triangles: [number, number, number][] }[];
  /** Edge polylines as arrays of point indices into `positions`. */
  edges: number[][];
}

/**
 * Build the faceted prism (bottom cap `-z`, top cap `+z`, side walls) at `depth`.
 * Caps reuse the fan cap verts (shared, ±z normals); side walls get their own
 * duplicated ring verts (crease split, radial normals). Ring winding is assumed
 * CCW in (u,v), so the top cap fan is +z and the bottom is the reversed winding.
 */
export function prismLocal(profile: PrismProfile, depth: number): PrismLocal {
  const capPairs = profile.cap.positions.length / 2; // every cap vertex
  const positions: number[] = [];
  const normals: number[] = [];

  const pushV = (x: number, y: number, z: number, nx: number, ny: number, nz: number) => {
    positions.push(x, y, z);
    normals.push(nx, ny, nz);
  };

  // 1) bottom cap verts (z=0, normal -z), indices [0 .. capPairs-1]
  for (let i = 0; i < capPairs; i++) {
    pushV(profile.cap.positions[i * 2], profile.cap.positions[i * 2 + 1], 0, 0, 0, -1);
  }
  // 2) top cap verts (z=depth, normal +z), indices [capPairs .. 2*capPairs-1]
  const topBase = capPairs;
  for (let i = 0; i < capPairs; i++) {
    pushV(profile.cap.positions[i * 2], profile.cap.positions[i * 2 + 1], depth, 0, 0, 1);
  }

  // Bottom cap triangles = fan indices with REVERSED winding (normal -z).
  const bottomTris: [number, number, number][] = [];
  for (let i = 0; i + 2 < profile.cap.indices.length; i += 3) {
    const a = profile.cap.indices[i];
    const b = profile.cap.indices[i + 1];
    const c = profile.cap.indices[i + 2];
    bottomTris.push([a, c, b]);
  }
  // Top cap triangles = fan indices as-is, offset to the top verts (normal +z).
  const topTris: [number, number, number][] = [];
  for (let i = 0; i + 2 < profile.cap.indices.length; i += 3) {
    topTris.push([
      topBase + profile.cap.indices[i],
      topBase + profile.cap.indices[i + 1],
      topBase + profile.cap.indices[i + 2],
    ]);
  }

  // 3) side verts: per ring vertex a bottom+top duplicate with a radial normal
  //    (radiating from the ring's area centroid — layout-agnostic). Hole rings
  //    wind CW, so following ring order flips the triangle winding for free; the
  //    normal is negated so it points INTO the hole (away from the material).
  const sideTris: [number, number, number][] = [];
  const edges: number[][] = [];

  const addWall = (ring: [number, number][], outward: boolean): void => {
    const n = ring.length;
    if (n < 3) return;
    const [hubU, hubV] = ringCentroid(ring);
    const base = positions.length / 3;
    const sign = outward ? 1 : -1;
    for (let j = 0; j < n; j++) {
      const [u, v] = ring[j];
      const nu = u - hubU;
      const nv = v - hubV;
      const len = Math.hypot(nu, nv) || 1;
      const rx = (sign * nu) / len;
      const ry = (sign * nv) / len;
      pushV(u, v, 0, rx, ry, 0); // bottom dup, even index
      pushV(u, v, depth, rx, ry, 0); // top dup, odd index
    }
    for (let j = 0; j < n; j++) {
      const b0 = base + j * 2;
      const t0 = b0 + 1;
      const jn = (j + 1) % n;
      const b1 = base + jn * 2;
      const t1 = b1 + 1;
      sideTris.push([b0, b1, t1]);
      sideTris.push([b0, t1, t0]);
    }
    // Edges: bottom loop, top loop (closed), + verticals for small rings.
    const bottomLoop: number[] = [];
    const topLoop: number[] = [];
    for (let j = 0; j < n; j++) {
      bottomLoop.push(base + j * 2);
      topLoop.push(base + j * 2 + 1);
    }
    bottomLoop.push(base); // close
    topLoop.push(base + 1);
    edges.push(bottomLoop, topLoop);
    if (n <= 12) {
      for (let j = 0; j < n; j++) edges.push([base + j * 2, base + j * 2 + 1]);
    } else {
      edges.push([base, base + 1]); // a single seam for round profiles
    }
  };

  addWall(profile.ring, true);
  for (const hole of profile.holes) addWall(hole, false);

  return {
    positions,
    normals,
    faces: [{ triangles: bottomTris }, { triangles: topTris }, { triangles: sideTris }],
    edges,
  };
}

/**
 * A UNIT-depth prism (cap at z=1) as merged indexed geometry, plane-local. The
 * engine builds this ONCE and scales z by the live depth (zero per-frame alloc).
 * No normals — L1 is a flat translucent accent, not lit.
 */
export function unitPrismGeometry(profile: PrismProfile): {
  positions: Float32Array;
  indices: Uint32Array;
} {
  const prism = prismLocal(profile, 1);
  const indices: number[] = [];
  for (const f of prism.faces) for (const t of f.triangles) indices.push(t[0], t[1], t[2]);
  return { positions: Float32Array.from(prism.positions), indices: Uint32Array.from(indices) };
}
