/*
 * rebindPick — resolving a selection ref against a mesh, and re-resolving one
 * whose TopoKey did not survive a regen.
 *
 * A TopoKey (`"f:22"`) is a SNAPSHOT-SCOPED shape-map ordinal (SCHEMA §10): a
 * rebuild may renumber every one of them. So a face selected before a parametric
 * edit routinely names NOTHING after it — `TopoIndex.ordinalForId` returns -1,
 * `HighlightLayer` drops the overlay, and the highlight simply vanishes with no
 * way for the user to tell a deselect from a defect.
 *
 * `rebindRef` re-binds such a ref to the element it still visually refers to, and
 * is built to REFUSE rather than guess (the house rule: a deterministic no-bind
 * beats a silent wrong bind):
 *
 *  - POSITION comes from the pick's own anchor — the point the user clicked,
 *    which lies ON the element — falling back to the old element's centroid.
 *  - DIRECTION comes from the PREVIOUS registry entry, the one the ref was picked
 *    against. Position alone cannot tell "this face is still here, renumbered"
 *    from "this face moved and a different one slid under the anchor": grow an
 *    extrude with its top face selected and the old anchor lands exactly ON a
 *    side WALL. The normal test rejects that; distance alone would bind it.
 *    With no previous entry naming the old element there is no evidence at all,
 *    and nothing is rebound.
 *  - A match must also be within {@link REBIND_TOL_FRAC} of the body's bbox
 *    diagonal. The tolerance exists because a regen RE-TESSELLATES — the same
 *    nominal surface returns as different triangles — and is deliberately far
 *    tighter than any parametric delta a user would type.
 *
 * `id` is IDENTITY (`${bodyId}#${topoKey}` — what `sameRef` and the selection
 * toggle compare on) and is NEVER rewritten; only the `topoKey` field the
 * viewport resolves through. Same policy as `promotePick`'s elementId write-back.
 */
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import type { TopoIndex } from "./faceRangeIndex";
import type { MeshEntry } from "./meshRegistry";
import type { BodyMeshView } from "./parseMeshPayload";

type V3 = [number, number, number];

/** A rebind candidate must sit within this fraction of the bbox diagonal. */
const REBIND_TOL_FRAC = 1e-3;
/** …and agree in direction with the element the ref used to name (≈25°). */
const REBIND_DIR_MIN = 0.9;
/** Degenerate bbox (a single point body): still allow an exact positional match. */
const REBIND_TOL_FLOOR = 1e-9;

/**
 * Ordinal of the element a ref names, or -1.
 *
 * The mesh's id tables hold TopoKeys today, but MESH1 `IDS_HAVE_ELEMENTIDS`
 * lets a producer emit MINTED ElementIds instead (mesh_format.md §2). A ref that
 * has been promoted carries both, so try the persistent id when the transient
 * one misses — inert until the worker sets the flag, and the only lookup that
 * survives a renumber for free once it does.
 */
export function ordinalForRef(index: TopoIndex, view: BodyMeshView, ref: EntityRef): number {
  if (ref.topoKey) {
    const ord = index.ordinalForId(ref.topoKey);
    if (ord >= 0) return ord;
  }
  if (ref.elementId && view.idsHaveElementIds) return index.ordinalForId(ref.elementId);
  return -1;
}

// ── vector scratch ──────────────────────────────────────────────────────────

const dot3 = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

const distSq = (q: V3, x: number, y: number, z: number): number => {
  const dx = q[0] - x;
  const dy = q[1] - y;
  const dz = q[2] - z;
  return dx * dx + dy * dy + dz * dz;
};

function bboxDiag(view: BodyMeshView): number {
  return Math.hypot(
    view.bboxMax[0] - view.bboxMin[0],
    view.bboxMax[1] - view.bboxMin[1],
    view.bboxMax[2] - view.bboxMin[2],
  );
}

/**
 * Squared distance from `q` to triangle (ai,bi,ci) — indices are FLOAT offsets
 * into `P`. Ericson, *Real-Time Collision Detection* §5.1.5 (barycentric region
 * test); returns the exact closest-point distance, edges and vertices included.
 */
function pointTriangleDistSq(q: V3, P: Float32Array, ai: number, bi: number, ci: number): number {
  const ax = P[ai], ay = P[ai + 1], az = P[ai + 2];
  const bx = P[bi], by = P[bi + 1], bz = P[bi + 2];
  const cx = P[ci], cy = P[ci + 1], cz = P[ci + 2];
  const abx = bx - ax, aby = by - ay, abz = bz - az;
  const acx = cx - ax, acy = cy - ay, acz = cz - az;
  const apx = q[0] - ax, apy = q[1] - ay, apz = q[2] - az;
  const d1 = abx * apx + aby * apy + abz * apz;
  const d2 = acx * apx + acy * apy + acz * apz;
  if (d1 <= 0 && d2 <= 0) return apx * apx + apy * apy + apz * apz; // vertex a
  const bpx = q[0] - bx, bpy = q[1] - by, bpz = q[2] - bz;
  const d3 = abx * bpx + aby * bpy + abz * bpz;
  const d4 = acx * bpx + acy * bpy + acz * bpz;
  if (d3 >= 0 && d4 <= d3) return bpx * bpx + bpy * bpy + bpz * bpz; // vertex b
  const vc = d1 * d4 - d3 * d2;
  if (vc <= 0 && d1 >= 0 && d3 <= 0) {
    const v = d1 / (d1 - d3); // edge ab
    return distSq(q, ax + abx * v, ay + aby * v, az + abz * v);
  }
  const cpx = q[0] - cx, cpy = q[1] - cy, cpz = q[2] - cz;
  const d5 = abx * cpx + aby * cpy + abz * cpz;
  const d6 = acx * cpx + acy * cpy + acz * cpz;
  if (d6 >= 0 && d5 <= d6) return cpx * cpx + cpy * cpy + cpz * cpz; // vertex c
  const vb = d5 * d2 - d1 * d6;
  if (vb <= 0 && d2 >= 0 && d6 <= 0) {
    const w = d2 / (d2 - d6); // edge ac
    return distSq(q, ax + acx * w, ay + acy * w, az + acz * w);
  }
  const va = d3 * d6 - d5 * d4;
  if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
    const w = (d4 - d3) / (d4 - d3 + (d5 - d6)); // edge bc
    return distSq(q, bx + (cx - bx) * w, by + (cy - by) * w, bz + (cz - bz) * w);
  }
  const denom = 1 / (va + vb + vc); // interior
  const v = vb * denom;
  const w = vc * denom;
  return distSq(q, ax + abx * v + acx * w, ay + aby * v + acy * w, az + abz * v + acz * w);
}

// ── faces ───────────────────────────────────────────────────────────────────

/** Area-weighted centroid + unit normal of a face, or null when degenerate. */
export function faceGeom(view: BodyMeshView, ord: number): { point: V3; dir: V3 } | null {
  if (ord < 0 || ord >= view.faceCount) return null;
  const firstTri = view.faceRanges[ord * 2];
  const triCount = view.faceRanges[ord * 2 + 1];
  const P = view.positions;
  const IX = view.indices;
  let nx = 0, ny = 0, nz = 0, cx = 0, cy = 0, cz = 0, area2 = 0;
  for (let t = firstTri; t < firstTri + triCount; t++) {
    const a = IX[t * 3] * 3, b = IX[t * 3 + 1] * 3, c = IX[t * 3 + 2] * 3;
    const ux = P[b] - P[a], uy = P[b + 1] - P[a + 1], uz = P[b + 2] - P[a + 2];
    const vx = P[c] - P[a], vy = P[c + 1] - P[a + 1], vz = P[c + 2] - P[a + 2];
    const kx = uy * vz - uz * vy, ky = uz * vx - ux * vz, kz = ux * vy - uy * vx;
    const w = Math.hypot(kx, ky, kz); // 2 × triangle area
    nx += kx; ny += ky; nz += kz;
    cx += (w * (P[a] + P[b] + P[c])) / 3;
    cy += (w * (P[a + 1] + P[b + 1] + P[c + 1])) / 3;
    cz += (w * (P[a + 2] + P[b + 2] + P[c + 2])) / 3;
    area2 += w;
  }
  const nl = Math.hypot(nx, ny, nz);
  if (nl === 0 || area2 === 0) return null;
  return { point: [cx / area2, cy / area2, cz / area2], dir: [nx / nl, ny / nl, nz / nl] };
}

/** Squared distance from `q` to a face's nearest triangle. */
function faceDistSq(view: BodyMeshView, ord: number, q: V3): number {
  const firstTri = view.faceRanges[ord * 2];
  const triCount = view.faceRanges[ord * 2 + 1];
  const IX = view.indices;
  let best = Infinity;
  for (let t = firstTri; t < firstTri + triCount; t++) {
    const d = pointTriangleDistSq(q, view.positions, IX[t * 3] * 3, IX[t * 3 + 1] * 3, IX[t * 3 + 2] * 3);
    if (d < best) best = d;
  }
  return best;
}

/** Cheap prefilter. No FACE_BBOXES section ⇒ every face is a candidate. */
function inFaceBbox(view: BodyMeshView, ord: number, q: V3, eps: number): boolean {
  const bb = view.faceBboxes;
  if (!bb) return true;
  const o = ord * 6;
  for (let a = 0; a < 3; a++) {
    if (q[a] < bb[o + a] - eps || q[a] > bb[o + 3 + a] + eps) return false;
  }
  return true;
}

/** Nearest face to `q` that also points the same way as `dir`, or -1. */
function nearestFace(view: BodyMeshView, q: V3, dir: V3, tol: number): number {
  let best = -1;
  let bestSq = tol * tol;
  for (let f = 0; f < view.faceCount; f++) {
    if (!inFaceBbox(view, f, q, tol)) continue;
    const dsq = faceDistSq(view, f, q);
    if (dsq >= bestSq) continue;
    // Only for a candidate that would WIN — the normal costs a pass over its
    // triangles, and most faces never get close enough to need one.
    const g = faceGeom(view, f);
    if (!g || dot3(g.dir, dir) < REBIND_DIR_MIN) continue;
    best = f;
    bestSq = dsq;
  }
  return best;
}

// ── edges ───────────────────────────────────────────────────────────────────

/** Squared distance from `q` to segment `s` of an expanded segment buffer. */
function pointSegmentDistSq(q: V3, pos: Float32Array, s: number): number {
  const o = s * 6;
  const ax = pos[o], ay = pos[o + 1], az = pos[o + 2];
  const dx = pos[o + 3] - ax, dy = pos[o + 4] - ay, dz = pos[o + 5] - az;
  const len = dx * dx + dy * dy + dz * dz;
  if (len === 0) return distSq(q, ax, ay, az);
  let t = ((q[0] - ax) * dx + (q[1] - ay) * dy + (q[2] - az) * dz) / len;
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  return distSq(q, ax + dx * t, ay + dy * t, az + dz * t);
}

/** Nearest segment of one edge to `q`: its squared distance and unit direction. */
function nearestSegment(
  pos: Float32Array,
  first: number,
  count: number,
  q: V3,
): { distSq: number; dir: V3 } | null {
  let best = Infinity;
  let at = -1;
  for (let s = first; s < first + count; s++) {
    const d = pointSegmentDistSq(q, pos, s);
    if (d < best) {
      best = d;
      at = s;
    }
  }
  if (at < 0) return null;
  const o = at * 6;
  const dx = pos[o + 3] - pos[o], dy = pos[o + 4] - pos[o + 1], dz = pos[o + 5] - pos[o + 2];
  const len = Math.hypot(dx, dy, dz);
  if (len === 0) return null;
  return { distSq: best, dir: [dx / len, dy / len, dz / len] };
}

/** Probe point + local direction of an edge as the OLD mesh had it. */
function edgeRefFrame(entry: MeshEntry, ord: number, anchor: V3 | null): { point: V3; dir: V3 } | null {
  const ranges = entry.edgeSegmentRanges;
  const pos = entry.edgeSegmentPositions;
  if (!ranges || !pos) return null;
  const first = ranges[ord * 2];
  const count = ranges[ord * 2 + 1];
  if (count === 0) return null;
  // No anchor (a ref built off the tree rather than a pick): the midpoint of the
  // edge's middle segment stands in for it — always ON the edge, unlike a chord.
  const mid = first + (count >> 1);
  const o = mid * 6;
  const point: V3 = anchor ?? [
    (pos[o] + pos[o + 3]) / 2,
    (pos[o + 1] + pos[o + 4]) / 2,
    (pos[o + 2] + pos[o + 5]) / 2,
  ];
  const near = nearestSegment(pos, first, count, point);
  return near ? { point, dir: near.dir } : null;
}

/** Nearest edge to `q` running (anti)parallel to `dir`, or -1. */
function nearestEdge(entry: MeshEntry, q: V3, dir: V3, tol: number): number {
  const ranges = entry.edgeSegmentRanges;
  const pos = entry.edgeSegmentPositions;
  const index = entry.edgeIndex;
  if (!ranges || !pos || !index) return -1;
  let best = -1;
  let bestSq = tol * tol;
  for (let e = 0; e < index.count; e++) {
    const near = nearestSegment(pos, ranges[e * 2], ranges[e * 2 + 1], q);
    if (!near || near.distSq >= bestSq) continue;
    // ABSOLUTE: an edge has no outward side, so a reversed parametrisation is
    // the same edge (a face's normal, by contrast, is signed and stays signed).
    if (Math.abs(dot3(near.dir, dir)) < REBIND_DIR_MIN) continue;
    best = e;
    bestSq = near.distSq;
  }
  return best;
}

// ── public API ──────────────────────────────────────────────────────────────

/**
 * The TopoKey `ref` should carry in `next`, or null when nothing there is a
 * defensible match. `prev` is the entry the ref was picked against.
 */
export function rebindRef(ref: EntityRef, prev: MeshEntry, next: MeshEntry): string | null {
  if (ref.kind !== "face" && ref.kind !== "edge") return null;
  const tol = Math.max(REBIND_TOL_FRAC * bboxDiag(next.view), REBIND_TOL_FLOOR);
  const wp = ref.anchor?.worldPoint;
  const anchor: V3 | null = wp ? [wp[0], wp[1], wp[2]] : null;

  if (ref.kind === "face") {
    const prevOrd = ordinalForRef(prev.faceIndex, prev.view, ref);
    if (prevOrd < 0) return null;
    const from = faceGeom(prev.view, prevOrd);
    if (!from) return null;
    const ord = nearestFace(next.view, anchor ?? from.point, from.dir, tol);
    return ord < 0 ? null : next.faceIndex.idOf(ord);
  }

  if (!prev.edgeIndex || !next.edgeIndex) return null;
  const prevOrd = ordinalForRef(prev.edgeIndex, prev.view, ref);
  if (prevOrd < 0) return null;
  const from = edgeRefFrame(prev, prevOrd, anchor);
  if (!from) return null;
  const ord = nearestEdge(next, from.point, from.dir, tol);
  return ord < 0 ? null : next.edgeIndex.idOf(ord);
}

/** Does this ref belong to `bodyId` AND name nothing in `next` any more? */
function isStale(ref: EntityRef, bodyId: string, next: MeshEntry): boolean {
  if (ref.kind !== "face" && ref.kind !== "edge") return false;
  if (ref.bodyId !== bodyId || !ref.topoKey) return false;
  const index = ref.kind === "face" ? next.faceIndex : next.edgeIndex;
  return index ? ordinalForRef(index, next.view, ref) < 0 : false;
}

/**
 * Rebind every stale face/edge ref of `bodyId` in `refs`. Returns a NEW array,
 * or null when nothing moved — so a caller can skip the store write (and the
 * repaint) entirely in the overwhelmingly common no-op case.
 */
export function rebindRefs(
  bodyId: string,
  prev: MeshEntry,
  next: MeshEntry,
  refs: readonly EntityRef[],
): EntityRef[] | null {
  let changed = false;
  const out = refs.map((ref) => {
    if (!isStale(ref, bodyId, next)) return ref;
    const topoKey = rebindRef(ref, prev, next);
    if (!topoKey || topoKey === ref.topoKey) return ref;
    changed = true;
    // `id` is identity and stays put; only the transient TopoKey is refreshed.
    return { ...ref, topoKey };
  });
  return changed ? out : null;
}

/**
 * Store glue: re-point the selection + hover at the geometry they still mean,
 * after `bodyId`'s mesh was replaced. Call between the registry `swap` and the
 * highlight refresh, so the rebuild reads the rebound keys.
 */
export function rebindSelectionForBody(
  bodyId: string,
  prev: MeshEntry | undefined,
  next: MeshEntry,
): void {
  if (!prev || prev === next) return;
  const sel = selectionStore.getState();
  const selected = rebindRefs(bodyId, prev, next, sel.selected);
  if (selected) sel.set(selected);
  if (sel.hover) {
    const hover = rebindRefs(bodyId, prev, next, [sel.hover]);
    if (hover) sel.setHover(hover[0]);
  }
}
