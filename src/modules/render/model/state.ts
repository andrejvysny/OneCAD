/*
 * `onecad.render`'s future module-state slice shape (docs/RENDER_MODULE.md §4,
 * ADR-0004/0005). Not wired to `platform/documentState.ts` here — that seam is
 * a later work package's job; this file only fixes the shape it will read and
 * write.
 */
import type { MaterialDef, MaterialId } from "./material";
import type { RenderAssignments } from "./assignment";

export const CURRENT_RENDER_SCHEMA_VERSION = 1;
export const PINNED_OPENPBR_REVISION = "1.1.1";

export interface RenderModuleState {
  schemaVersion: 1;
  openPbrRevision: "1.1.1";
  library: Record<MaterialId, MaterialDef>;
  assignments: RenderAssignments;
}

export function createEmptyRenderState(): RenderModuleState {
  return {
    schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
    openPbrRevision: PINNED_OPENPBR_REVISION,
    library: {},
    assignments: { bodies: {}, faces: {} },
  };
}

/** True when there is nothing worth writing — the ADR-0004 skip-if-empty case
 *  for a module that has never been touched in this document. */
export function isEmptyRenderState(state: RenderModuleState): boolean {
  return (
    Object.keys(state.library).length === 0 &&
    Object.keys(state.assignments.bodies).length === 0 &&
    Object.keys(state.assignments.faces).length === 0
  );
}
