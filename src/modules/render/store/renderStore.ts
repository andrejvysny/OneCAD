/*
 * `onecad.render`'s live material state — the parsed, in-memory half of the
 * module-state slice `model/serialize.ts` defines (docs/RENDER_MODULE.md §4,
 * ADR-0004/0005).
 *
 * A zustand VANILLA store, exactly like `stores/documentStore.ts`: the store is
 * the truth, React is one optional reader of it, and a headless test (or the
 * viewport, which is not React at all) drives the identical API.
 *
 * Two rules shape everything below.
 *
 * OPTIMISTIC, THEN PERSIST. Every mutating action updates memory first and then
 * writes the whole slice through `platform/documentState.ts`, i.e. through the
 * backend's transaction path — so a programmatic material change is as undoable
 * as a user edit (docs/ARCHITECTURE.md §9) and there is no second mutation lane
 * that skips undo. A FAILED write re-hydrates from the backend rather than
 * guessing: the optimistic value is a prediction, and a rejected prediction is
 * corrected by asking, never by keeping.
 *
 * REFUSE RATHER THAN CLOBBER. A slice this build cannot parse (a future
 * `schemaVersion`, foreign bytes) puts the store in `readOnly` and every write
 * stands down. Rewriting it as v1 would silently destroy a user's data, which
 * ADR-0005 forbids; showing nothing and touching nothing is the honest failure.
 */
import { createStore } from "zustand";

import { createClient } from "@/ipc/client";
import { createDocumentStateService, type DocumentStateService } from "@/platform";

import type { MaterialDef, MaterialId } from "../model/material";
import {
  CURRENT_RENDER_SCHEMA_VERSION,
  createEmptyRenderState,
  isEmptyRenderState,
  type RenderModuleState,
} from "../model/state";
import { parseRenderModuleState, serializeRenderModuleState } from "../model/serialize";
import { RENDER_MODULE_ID } from "../manifest";

// ── Persistence seam ─────────────────────────────────────────────────────────

/**
 * Created LAZILY, and through `createClient()` for the same reason modeling's
 * services are (`modules/modeling/register.ts`): binding a client at module load
 * would capture whichever lane happened to exist when the file was first
 * imported, which under vitest is decided by import order rather than by intent.
 */
let service: DocumentStateService | null = null;

function documentState(): DocumentStateService {
  return (service ??= createDocumentStateService(createClient(), RENDER_MODULE_ID));
}

/** Test seam: force a specific service (or `null` to fall back to the client). */
export function setRenderDocumentStateService(next: DocumentStateService | null): void {
  service = next;
}

// ── Live-body seam ───────────────────────────────────────────────────────────

/**
 * How this store learns which bodies exist, INJECTED by `module.ts` from
 * modeling's published `GeometryQuery` service. Render must never import
 * `documentStore` — modeling owns it — and it must not hold a `Platform`
 * reference either, so the dependency arrives as one function.
 *
 * `null` (unset, or the service is gone) means "the body set is unknown", which
 * is deliberately NOT the same as "there are no bodies": classifying every
 * assignment as dangling because nobody told us the ids would be a false alarm.
 */
export type RenderBodyLister = () => readonly string[] | null;

let bodyLister: RenderBodyLister | null = null;

export function setRenderBodyLister(next: RenderBodyLister | null): void {
  bodyLister = next;
}

// ── Write/read ordering ──────────────────────────────────────────────────────

/**
 * Bumped by every local mutation (and by `reset`). A `hydrate()` that started
 * before a local edit landed must DISCARD its result: the backend read is older
 * than the optimistic state, and applying it would visibly revert an edit the
 * user already sees. One counter is enough — the store has a single writer.
 */
let generation = 0;

// ── State ────────────────────────────────────────────────────────────────────

export interface RenderStoreState {
  /** True once a read of the slice has completed (successfully or not). */
  hydrated: boolean;
  /**
   * The slice exists but this build cannot parse it. Every mutating action then
   * refuses, so the foreign bytes survive verbatim (ADR-0005).
   */
  readOnly: boolean;
  /** Tolerant-parse diagnostics from the last hydrate; `[]` when clean. */
  warnings: string[];
  /** ALWAYS a valid state — empty when there is nothing (or nothing readable). */
  state: RenderModuleState;
  /** Assigned body ids absent from the live document. Reported, never deleted. */
  danglingBodies: string[];
  /**
   * Face overrides the VIEWPORT could not bind to a face of the body it drew,
   * keyed by body id. Session feedback only — never persisted, and never merged
   * into `danglingBodies`: a face that is missing from one tessellation is not
   * evidence that the assignment is stale.
   */
  unboundFaceOverrides: Record<string, string[]>;

  /** Read the slice and replace in-memory state with it. Never throws. */
  hydrate(): Promise<void>;
  /** Add or replace one material (keyed by `def.id`). */
  upsertMaterial(def: MaterialDef): Promise<void>;
  /** Remove a material AND every assignment naming it, in one write. */
  removeMaterial(id: MaterialId): Promise<void>;
  /** Assign (or, with `null`, unassign) a body-level material. */
  assignBody(bodyId: string, materialId: MaterialId | null): Promise<void>;
  /** Assign (or, with `null`, unassign) a per-face override. */
  assignFace(faceElementId: string, materialId: MaterialId | null): Promise<void>;
  /** Drop several face overrides at once (the override-replace prompt, WP3). */
  clearFaceOverrides(faceElementIds: readonly string[]): Promise<void>;
  /** Viewport feedback: overrides that bound to no face of `bodyId`. */
  reportUnboundFaceOverrides(bodyId: string, faceElementIds: readonly string[]): void;
  /** Re-derive {@link RenderStoreState.danglingBodies} from the live body set. */
  recomputeDangling(): void;
  /** Forget everything, IN MEMORY ONLY — see the note on the implementation. */
  reset(): void;
}

function initialState() {
  return {
    hydrated: false,
    readOnly: false,
    warnings: [] as string[],
    state: createEmptyRenderState(),
    danglingBodies: [] as string[],
    unboundFaceOverrides: {} as Record<string, string[]>,
  };
}

/** Assignment body ids the live document does not have. */
function danglingFrom(state: RenderModuleState): string[] {
  const live = bodyLister?.() ?? null;
  if (live === null) return [];
  const known = new Set(live);
  return Object.keys(state.assignments.bodies).filter((id) => !known.has(id));
}

export const renderStore = createStore<RenderStoreState>()((set, get) => {
  /**
   * Commit `next` to memory and to the document, in that order.
   *
   * `clear()` rather than a write of an empty state is ADR-0004's skip-if-empty
   * rule: a document nobody assigned a material in must not grow a render slice
   * just because the module loaded.
   */
  const commit = async (next: RenderModuleState): Promise<void> => {
    generation += 1;
    set({ state: next, danglingBodies: danglingFrom(next) });
    try {
      if (isEmptyRenderState(next)) await documentState().clear();
      else {
        await documentState().write({
          schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
          payload: serializeRenderModuleState(next),
        });
      }
    } catch (cause) {
      // Reconcile, never guess: the write may have half-landed, and the backend
      // is the only thing that knows.
      console.warn("onecad.render: writing material state failed; re-reading", cause);
      await get().hydrate();
    }
  };

  /** `false` (with a warning) when the store must not write. */
  const writable = (action: string): boolean => {
    if (!get().readOnly) return true;
    console.warn(
      `onecad.render: ${action} refused — this document's render state was written by ` +
        "another version and is preserved unread",
    );
    return false;
  };

  return {
    ...initialState(),

    async hydrate() {
      const at = generation;
      let slice: Awaited<ReturnType<DocumentStateService["read"]>>;
      try {
        slice = await documentState().read();
      } catch (cause) {
        console.warn("onecad.render: reading material state failed", cause);
        set({ hydrated: true });
        return;
      }
      // A local edit landed while the read was in flight — it is NEWER than what
      // came back, so the read is stale and dropping it is the correct outcome.
      if (at !== generation) return;

      if (slice === null) {
        const state = createEmptyRenderState();
        set({ hydrated: true, readOnly: false, warnings: [], state, danglingBodies: danglingFrom(state) });
        return;
      }

      const { state, warnings } = parseRenderModuleState(slice.payload);
      if (state === null) {
        // The slice EXISTS and did not parse: hold it read-only. The empty state
        // below is what the UI renders, not what gets written back — nothing is
        // written back at all until the document is opened by a build that
        // understands it.
        const empty = createEmptyRenderState();
        set({ hydrated: true, readOnly: true, warnings, state: empty, danglingBodies: [] });
        return;
      }
      set({ hydrated: true, readOnly: false, warnings, state, danglingBodies: danglingFrom(state) });
    },

    async upsertMaterial(def) {
      if (!writable("upsertMaterial")) return;
      const current = get().state;
      await commit({ ...current, library: { ...current.library, [def.id]: def } });
    },

    async removeMaterial(id) {
      if (!writable("removeMaterial")) return;
      const current = get().state;
      if (current.library[id] === undefined && !usesMaterial(current, id)) return;
      const library = { ...current.library };
      delete library[id];
      // ONE write, not three. A material removal that persisted the library
      // first would leave a window in which the document referenced a material
      // that no longer exists — and if the second write never happened (crash,
      // failed IPC), that window would be permanent.
      await commit({
        ...current,
        library,
        assignments: {
          bodies: withoutMaterial(current.assignments.bodies, id),
          faces: withoutMaterial(current.assignments.faces, id),
        },
      });
    },

    async assignBody(bodyId, materialId) {
      if (!writable("assignBody")) return;
      const current = get().state;
      const bodies = { ...current.assignments.bodies };
      if (materialId === null) delete bodies[bodyId];
      else bodies[bodyId] = materialId;
      await commit({ ...current, assignments: { ...current.assignments, bodies } });
    },

    async assignFace(faceElementId, materialId) {
      if (!writable("assignFace")) return;
      const current = get().state;
      const faces = { ...current.assignments.faces };
      if (materialId === null) delete faces[faceElementId];
      else faces[faceElementId] = materialId;
      await commit({ ...current, assignments: { ...current.assignments, faces } });
    },

    async clearFaceOverrides(faceElementIds) {
      if (!writable("clearFaceOverrides")) return;
      const current = get().state;
      if (!faceElementIds.some((id) => current.assignments.faces[id] !== undefined)) return;
      const faces = { ...current.assignments.faces };
      for (const id of faceElementIds) delete faces[id];
      await commit({ ...current, assignments: { ...current.assignments, faces } });
    },

    reportUnboundFaceOverrides(bodyId, faceElementIds) {
      set((s) => {
        const next = { ...s.unboundFaceOverrides };
        if (faceElementIds.length === 0) delete next[bodyId];
        else next[bodyId] = [...faceElementIds];
        return { unboundFaceOverrides: next };
      });
    },

    recomputeDangling() {
      set((s) => ({ danglingBodies: danglingFrom(s.state) }));
    },

    /**
     * IN MEMORY ONLY — deliberately not a write.
     *
     * This is the document-SWAP hook (`ipc/documentLifecycle.ts`): the outgoing
     * document's materials must leave the screen, and persisting an empty state
     * here would `clear()` the slice of the document being closed, i.e. delete
     * the user's materials as a side effect of closing the file.
     */
    reset() {
      // Any hydrate still in flight belongs to the OUTGOING document.
      generation += 1;
      set(initialState());
    },
  };
});

/** Does any assignment name `id`? */
function usesMaterial(state: RenderModuleState, id: MaterialId): boolean {
  return (
    Object.values(state.assignments.bodies).includes(id) ||
    Object.values(state.assignments.faces).includes(id)
  );
}

function withoutMaterial(
  map: Record<string, MaterialId>,
  id: MaterialId,
): Record<string, MaterialId> {
  return Object.fromEntries(Object.entries(map).filter(([, materialId]) => materialId !== id));
}
