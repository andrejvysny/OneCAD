import * as THREE from "three";
import type { SketchEntity, SketchPlane } from "@/ipc/types";
import type { DraftEntity } from "@/tools/sketch/toolMachine";
import { entityAabb } from "@/tools/sketch/snapCandidates";
import { planePointToWorld } from "./sketchBasis";

export function sketchFrameBounds(
  plane: SketchPlane,
  entities: readonly SketchEntity[],
  drafts: readonly DraftEntity[] = [],
): THREE.Box3 | null {
  const bounds2d = { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity };
  for (const entity of entities) {
    if (entity.construction) continue;
    const box = entity.type === "Point" && entity.p0
      ? { minX: entity.p0[0], minY: entity.p0[1], maxX: entity.p0[0], maxY: entity.p0[1] }
      : entityAabb(entity);
    if (box) unionFinite(bounds2d, box);
  }
  for (const draft of drafts) {
    if (draft.construction) continue;
    const box = draftBounds(draft);
    if (box) unionFinite(bounds2d, box);
  }
  if (!Number.isFinite(bounds2d.minX) || (bounds2d.minX === bounds2d.maxX && bounds2d.minY === bounds2d.maxY)) {
    return null;
  }
  const world = new THREE.Box3();
  for (const x of [bounds2d.minX, bounds2d.maxX]) {
    for (const y of [bounds2d.minY, bounds2d.maxY]) {
      world.expandByPoint(planePointToWorld(plane, { x, y }));
    }
  }
  return world.isEmpty() ? null : world;
}

interface Bounds2d { minX: number; minY: number; maxX: number; maxY: number }

function unionFinite(target: Bounds2d, box: Bounds2d): void {
  if (![box.minX, box.minY, box.maxX, box.maxY].every(Number.isFinite)) return;
  target.minX = Math.min(target.minX, box.minX);
  target.minY = Math.min(target.minY, box.minY);
  target.maxX = Math.max(target.maxX, box.maxX);
  target.maxY = Math.max(target.maxY, box.maxY);
}

function draftBounds(draft: DraftEntity): Bounds2d | null {
  if (draft.type === "Point" && draft.p0) return pointBounds(draft.p0.x, draft.p0.y);
  if (draft.type === "Line" && draft.p0 && draft.p1) {
    return {
      minX: Math.min(draft.p0.x, draft.p1.x), minY: Math.min(draft.p0.y, draft.p1.y),
      maxX: Math.max(draft.p0.x, draft.p1.x), maxY: Math.max(draft.p0.y, draft.p1.y),
    };
  }
  if ((draft.type === "Circle" || draft.type === "Arc") && draft.center && draft.radius !== undefined) {
    return {
      minX: draft.center.x - draft.radius, minY: draft.center.y - draft.radius,
      maxX: draft.center.x + draft.radius, maxY: draft.center.y + draft.radius,
    };
  }
  if (draft.type === "Ellipse" && draft.center && draft.majorR !== undefined && draft.minorR !== undefined) {
    const rotation = draft.rotation ?? 0;
    const ex = Math.hypot(draft.majorR * Math.cos(rotation), draft.minorR * Math.sin(rotation));
    const ey = Math.hypot(draft.majorR * Math.sin(rotation), draft.minorR * Math.cos(rotation));
    return {
      minX: draft.center.x - ex, minY: draft.center.y - ey,
      maxX: draft.center.x + ex, maxY: draft.center.y + ey,
    };
  }
  return null;
}

function pointBounds(x: number, y: number): Bounds2d {
  return { minX: x, minY: y, maxX: x, maxY: y };
}
