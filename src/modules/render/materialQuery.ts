/*
 * `RenderServices.MaterialQuery`'s implementation — a read-only view over
 * `store/renderStore.ts` (contract + reasoning: `manifest.ts`).
 *
 * Every method is a synchronous read of already-parsed state. That is the point:
 * the viewport asks "what material does this face wear?" once per body per
 * repaint, and an async or re-parsing answer there would put IPC latency on a
 * render path.
 *
 * `resolveAssignment` / `resolveMaterial` / `resolvedMaterialHash` are reused
 * verbatim from `model/`, never reimplemented here — the override-wins rule and
 * the hash have exactly one definition (`model/assignment.ts`, `model/material.ts`).
 */
import {
  resolveMaterial,
  resolvedMaterialHash,
  type MaterialDef,
  type MaterialId,
} from "./model/material";
import { resolveAssignment } from "./model/assignment";
import type { MaterialQueryService } from "./manifest";
import { renderStore } from "./store/renderStore";

/** The material `id` names, or `null` — a dangling id is never invented into one. */
function lookup(id: MaterialId | null): MaterialDef | null {
  if (id === null) return null;
  return renderStore.getState().state.library[id] ?? null;
}

export function createMaterialQueryService(): MaterialQueryService {
  return {
    listMaterials: () => Object.values(renderStore.getState().state.library),

    getMaterial: (id) => renderStore.getState().state.library[id] ?? null,

    materialIdForBody: (bodyId) =>
      renderStore.getState().state.assignments.bodies[bodyId] ?? null,

    materialForFace: (bodyId, faceElementId) =>
      lookup(resolveAssignment(renderStore.getState().state.assignments, bodyId, faceElementId)),

    resolvedForBody(bodyId) {
      const def = lookup(resolveAssignment(renderStore.getState().state.assignments, bodyId));
      return def === null ? null : resolveMaterial(def);
    },

    poolKeyForBody(bodyId) {
      const materialId = renderStore.getState().state.assignments.bodies[bodyId] ?? null;
      const def = lookup(materialId);
      return def === null ? null : `${def.id}:${resolvedMaterialHash(def)}`;
    },

    faceOverridesForBody(_bodyId, faceElementIds) {
      // `bodyId` is the CALLER's context — it decides which face ids belong to
      // the body it is drawing. Face ElementIds are globally unique and Rust-
      // minted (CLAUDE.md), so the map itself needs no body qualification, and
      // re-deriving the body from a face id here would be a second, weaker copy
      // of a fact the caller already holds.
      const { faces } = renderStore.getState().state.assignments;
      const out: Record<string, MaterialId> = {};
      for (const id of faceElementIds) {
        const materialId = faces[id];
        if (materialId !== undefined) out[id] = materialId;
      }
      return out;
    },

    usageCounts() {
      const { assignments } = renderStore.getState().state;
      const counts: Record<MaterialId, number> = {};
      for (const id of [
        ...Object.values(assignments.bodies),
        ...Object.values(assignments.faces),
      ]) {
        counts[id] = (counts[id] ?? 0) + 1;
      }
      return counts;
    },

    // Gated on the persisted `state` object's identity: every mutating action
    // replaces it, and nothing else in the store does. A raw store subscription
    // would also fire for `hydrated`, `unboundFaceOverrides` and dangling-body
    // recomputes — none of which change what a material looks like.
    subscribe(cb) {
      let previous = renderStore.getState().state;
      return renderStore.subscribe((s) => {
        if (s.state === previous) return;
        previous = s.state;
        cb();
      });
    },
  };
}
