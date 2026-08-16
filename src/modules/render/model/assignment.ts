/*
 * Body/face material assignments (docs/RENDER_MODULE.md §4). Two separate maps
 * rather than one union-keyed map: a body id and a persistent face `ElementId`
 * are drawn from disjoint id spaces (this repo has no branded id types for
 * either — both are plain strings on the wire, see `ipc/types.ts`), so nothing
 * is gained by merging them and a lot of "which kind of key is this" ambiguity
 * would be introduced by trying.
 */
import type { MaterialId } from "./material";

export interface RenderAssignments {
  /** Keyed by BodyId. */
  bodies: Record<string, MaterialId>;
  /** Keyed by persistent face ElementId. */
  faces: Record<string, MaterialId>;
}

/** Face override wins over the body's assignment; `null` when neither is set. */
export function resolveAssignment(
  assignments: RenderAssignments,
  bodyId: string,
  faceElementId?: string,
): MaterialId | null {
  if (faceElementId !== undefined) {
    const faceMaterial = assignments.faces[faceElementId];
    if (faceMaterial !== undefined) return faceMaterial;
  }
  return assignments.bodies[bodyId] ?? null;
}

/** How many of a body's faces carry their own override — used later to decide
 *  whether an inspector shows "mixed" for a body-level material control. */
export function faceOverrideCount(
  assignments: RenderAssignments,
  faceIdsOfBody: readonly string[],
): number {
  let count = 0;
  for (const faceId of faceIdsOfBody) {
    if (assignments.faces[faceId] !== undefined) count++;
  }
  return count;
}

/**
 * Pure classification against the current document's live ids — never a
 * deletion. Per the repo's identity law (CLAUDE.md, H5-B), a dangling
 * assignment is reported so a caller can surface it as a repair concern,
 * never silently dropped.
 */
export function partitionDanglingAssignments(
  assignments: RenderAssignments,
  knownBodyIds: ReadonlySet<string>,
  knownFaceElementIds: ReadonlySet<string>,
): { live: RenderAssignments; dangling: { bodies: string[]; faces: string[] } } {
  const liveBodies: Record<string, MaterialId> = {};
  const danglingBodies: string[] = [];
  for (const [bodyId, materialId] of Object.entries(assignments.bodies)) {
    if (knownBodyIds.has(bodyId)) liveBodies[bodyId] = materialId;
    else danglingBodies.push(bodyId);
  }

  const liveFaces: Record<string, MaterialId> = {};
  const danglingFaces: string[] = [];
  for (const [faceId, materialId] of Object.entries(assignments.faces)) {
    if (knownFaceElementIds.has(faceId)) liveFaces[faceId] = materialId;
    else danglingFaces.push(faceId);
  }

  return {
    live: { bodies: liveBodies, faces: liveFaces },
    dangling: { bodies: danglingBodies, faces: danglingFaces },
  };
}
