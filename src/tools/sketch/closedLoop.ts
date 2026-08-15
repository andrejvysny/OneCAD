/*
 * Closed-loop detection (PURE) — finds simple polygon chains of Line/Arc
 * entities in a sketch (endpoint-to-endpoint, degree-2 at every shared vertex)
 * so the viewport can mark each loop's centroid, mirroring how
 * `prismPreview.ringCentroid` already centers an extrude handle on a solved
 * region's triangulated ring — this instead works straight off entity
 * endpoints, before any region/triangulation exists, so it also lights up
 * while a profile is still being drawn.
 */
import type { SketchEntity } from "@/ipc/types";
import { entityPolyline } from "@/viewport/engine/SketchObject";
import { ringCentroid } from "@/tools/preview/prismPreview";

const EPS = 1e-6;

function endpoints(e: SketchEntity): [[number, number], [number, number]] | null {
  if (e.type === "Line" && e.p0 && e.p1) return [e.p0, e.p1];
  if (e.type === "Arc" && e.start && e.end) return [e.start, e.end];
  return null;
}

function key(p: [number, number]): string {
  return `${Math.round(p[0] / EPS)},${Math.round(p[1] / EPS)}`;
}

/** One closed chain of entity ids, in walk order. */
export interface ClosedLoop {
  entityIds: string[];
}

/**
 * Every simple closed chain of Line/Arc entities in `entities`. An entity
 * belongs to at most one loop; entities on a branching/T-junction vertex
 * (more than 2 chain-eligible entities meeting at one point) are excluded
 * from every loop rather than guessed into one, since there is no unique
 * closed chain through a degree-&gt;2 vertex.
 */
export function findClosedLoops(entities: SketchEntity[]): ClosedLoop[] {
  const ends = new Map(entities.map((e) => [e.id, endpoints(e)]).filter((p): p is [string, [[number, number], [number, number]]] => p[1] !== null));

  // Adjacency: point key -> entities touching it there, with their OTHER end.
  const adjacency = new Map<string, { entityId: string; otherKey: string }[]>();
  for (const [id, [a, b]] of ends) {
    const ka = key(a);
    const kb = key(b);
    (adjacency.get(ka) ?? adjacency.set(ka, []).get(ka)!).push({ entityId: id, otherKey: kb });
    (adjacency.get(kb) ?? adjacency.set(kb, []).get(kb)!).push({ entityId: id, otherKey: ka });
  }
  // A vertex with more than 2 chain-eligible touches is a branch/T-junction —
  // no entity through it has a UNIQUE next hop, so exclude them all up front.
  const branchIds = new Set<string>();
  for (const touches of adjacency.values()) {
    if (touches.length > 2) for (const t of touches) branchIds.add(t.entityId);
  }

  const loops: ClosedLoop[] = [];
  const visited = new Set<string>();
  for (const [startId, [startA]] of ends) {
    if (visited.has(startId) || branchIds.has(startId)) continue;
    const startKey = key(startA);
    const chain: string[] = [];
    let currentId = startId;
    let cameFromKey = startKey;
    let closed = false;
    let stuck = false;
    for (let guard = 0; guard < entities.length + 1 && !closed && !stuck; guard++) {
      if (branchIds.has(currentId) || visited.has(currentId)) {
        stuck = true;
        break;
      }
      chain.push(currentId);
      visited.add(currentId);
      const pair = ends.get(currentId)!;
      const otherKey = key(pair[0]) === cameFromKey ? key(pair[1]) : key(pair[0]);
      if (otherKey === startKey && chain.length >= 3) {
        closed = true;
        break;
      }
      const touches = adjacency.get(otherKey) ?? [];
      const next = touches.find((t) => t.entityId !== currentId && !visited.has(t.entityId));
      if (!next) {
        stuck = true;
        break;
      }
      cameFromKey = otherKey;
      currentId = next.entityId;
    }
    if (closed) {
      loops.push({ entityIds: chain });
    } else {
      // Open chain (or the guard ran out) — not a simple loop; release its
      // entities so they can still be tried as part of another loop that
      // shares one of them from a different starting point.
      for (const id of chain) visited.delete(id);
    }
  }
  return loops;
}

/** Plane (u,v) centroid of a closed loop, tessellating Arc entities so the
 *  centroid reflects the true curved boundary, not its chord. */
export function loopCentroid(loop: ClosedLoop, byId: Map<string, SketchEntity>): [number, number] | null {
  const ring: [number, number][] = [];
  for (const id of loop.entityIds) {
    const e = byId.get(id);
    if (!e) return null;
    const pts = entityPolyline(e);
    for (let i = 0; i < pts.length; i += 3) ring.push([pts[i], pts[i + 1]]);
  }
  if (ring.length < 3) return null;
  return ringCentroid(ring);
}
