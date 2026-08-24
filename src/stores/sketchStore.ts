/*
 * Live sketch-edit store (F-WP6).
 *
 * Holds the authoritative sketch SESSION currently being edited (plane +
 * entities + constraints + dof/status) plus the entity/constraint id counters.
 * Written by the SketchController on enter / commit / solve; read by the
 * ConstraintBadgeLayer (badges at entity midpoints) and the engine's
 * SketchObject (via the controller). High-frequency interaction state (preview,
 * snap indicator, ghost badge) is NOT here — it is driven imperatively through
 * the engine so pointer-move does not churn React.
 *
 * DOF/status ALSO flow to documentStore.sketches[id] + viewportStore.dofBadge so
 * the chrome bar / inspector / status bar (bound since F-WP3) stay live.
 *
 * ── Session generation (fencing) ──────────────────────────────────────────────
 * `sessionGeneration` bumps on EVERY setSession. A queued sketch mutation captures
 * the generation at its turn start and re-checks it after each await before writing
 * any store/engine state; a mismatch means a newer session superseded it, so the
 * write is silently dropped. This is the frontend analogue of the backend's
 * workerEpoch fence — last-write-wins by generation, no torn state.
 *
 * ── Sketch-scoped undo/redo (C2) ──────────────────────────────────────────────
 * A per-session snapshot stack (entities + constraints), independent of the model
 * history. Mutations push the PRE-mutation snapshot on confirmed success; undo pops
 * it, re-solves the prior arrays, and moves the current state onto the redo stack.
 * Cleared on enter / exit / dispose so a stale session's history never leaks.
 */
import { createStore, useStore } from "zustand";
import type {
  SketchConstraint,
  SketchEntity,
  SketchEntityStates,
  SketchSession,
} from "@/ipc/types";

/** A restorable snapshot of the authoritative sketch arrays. */
export interface SketchSnapshot {
  entities: SketchEntity[];
  constraints: SketchConstraint[];
}

/** Who pushed the top undo snapshot — used to COALESCE consecutive edits to the
 *  same dimensional constraint into a single undo entry. */
export interface UndoProvenance {
  kind: string;
  constraintId?: string;
}

/** Undo/redo history depth cap (oldest entries are sliced off). */
const UNDO_CAP = 50;

export interface SketchState {
  session: SketchSession | null;
  /** Bumped on EVERY setSession — the fencing token queued mutations check. */
  sessionGeneration: number;
  /** FRONTEND constraint ids the solver reports in conflict (SCHEMA §7.4). ONE
   *  owner: every successful solve write-back REPLACES it (`setConflicting`); session
   *  enter SEEDS it from `session.conflicting`; exit/dispose/session-swap CLEAR it
   *  (setSession(null)). The inspector + badge layer tint these ids red. */
  conflictingIds: string[];
  /** PER-ENTITY constrained state (SCHEMA §7.4 `entityStates`), keyed by FRONTEND
   *  entity id. Same single-owner rule as `conflictingIds`: every successful solve
   *  write-back REPLACES it (`setEntityStates`), session enter SEEDS it from
   *  `session.entityStates`, exit/dispose/session-swap CLEAR it. An entity ABSENT
   *  from the map is UNKNOWN — the viewport falls back to the whole-sketch tint for
   *  it, never to under-constrained. */
  entityStates: SketchEntityStates;
  /** STICKY draw modifier (W1-B): while on, every entity the draw tools commit is
   *  authored as CONSTRUCTION geometry (dashed, excluded from regions, still
   *  solved). Toggled by X with an empty sketch selection / the chrome-bar button.
   *  Session-independent — it survives entering and leaving sketches, and is not
   *  persisted (a document close resets the store). */
  constructionMode: boolean;
  entitySeq: number;
  constraintSeq: number;
  undoStack: SketchSnapshot[];
  redoStack: SketchSnapshot[];
  /** Provenance of the top undo snapshot (for edit coalescing). Null after a pop. */
  lastUndoPush: UndoProvenance | null;
  setSession(session: SketchSession | null): void;
  /** REPLACE the conflicting-id set (every solve write-back / session-enter seed).
   *  Short-circuits when unchanged so live drag solves don't churn subscribers. */
  setConflicting(ids: string[]): void;
  /** REPLACE the per-entity state map (every solve write-back / session-enter seed).
   *  Short-circuits when unchanged so a gesture echoing its fixed map — and every
   *  identity solve — keeps the SAME object reference and churns no subscriber. */
  setEntityStates(states: SketchEntityStates): void;
  /** Flip the sticky construction draw modifier. */
  toggleConstructionMode(): void;
  /** Mint the next entity id (`e1`, `e2`, …) and advance the counter. */
  nextEntityId(): string;
  /** Mint the next constraint id (`c1`, `c2`, …) and advance the counter. */
  nextConstraintId(): string;
  /** Push a PRE-mutation snapshot onto the undo stack (clears redo, caps at 50). */
  pushUndoSnapshot(snapshot: SketchSnapshot, provenance?: UndoProvenance | null): void;
  /** Pop the newest undo snapshot, moving `current` onto the redo stack. Null when empty. */
  popUndo(current: SketchSnapshot): SketchSnapshot | null;
  /** Pop the newest redo snapshot, moving `current` onto the undo stack. Null when empty. */
  popRedo(current: SketchSnapshot): SketchSnapshot | null;
  /** Drop the entire undo/redo history (enter / exit / dispose). */
  clearSketchUndo(): void;
  reset(): void;
}

export const sketchStore = createStore<SketchState>()((set, get) => ({
  session: null,
  sessionGeneration: 0,
  conflictingIds: [],
  entityStates: {},
  constructionMode: false,
  entitySeq: 0,
  constraintSeq: 0,
  undoStack: [],
  redoStack: [],
  lastUndoPush: null,

  setSession(session) {
    // Tearing down (session === null: exit / dispose / session-swap) clears the
    // conflicting-id set and the per-entity states; a live session leaves both to
    // the write-back's setConflicting / setEntityStates.
    set((s) => ({
      session,
      sessionGeneration: s.sessionGeneration + 1,
      ...(session === null ? { conflictingIds: [], entityStates: {} } : {}),
    }));
  },

  setConflicting(ids) {
    set((s) => {
      const cur = s.conflictingIds;
      // Skip the write (keep the array ref) when unchanged — subscribers don't churn
      // on repeated live drag solves that report the same (usually empty) set.
      if (cur.length === ids.length && cur.every((v, i) => v === ids[i])) return {};
      return { conflictingIds: ids };
    });
  },

  setEntityStates(states) {
    set((s) => {
      const cur = s.entityStates;
      const keys = Object.keys(states);
      if (Object.keys(cur).length === keys.length && keys.every((k) => cur[k] === states[k])) {
        return {}; // unchanged ⇒ keep the object ref (no subscriber churn)
      }
      return { entityStates: states };
    });
  },

  toggleConstructionMode() {
    set((s) => ({ constructionMode: !s.constructionMode }));
  },

  nextEntityId() {
    const n = get().entitySeq + 1;
    set({ entitySeq: n });
    return `e${n}`;
  },

  nextConstraintId() {
    const n = get().constraintSeq + 1;
    set({ constraintSeq: n });
    return `c${n}`;
  },

  pushUndoSnapshot(snapshot, provenance = null) {
    set((s) => ({
      undoStack: [...s.undoStack, snapshot].slice(-UNDO_CAP),
      redoStack: [],
      lastUndoPush: provenance,
    }));
  },

  popUndo(current) {
    const stack = get().undoStack;
    if (stack.length === 0) return null;
    const prev = stack[stack.length - 1];
    set((s) => ({
      undoStack: s.undoStack.slice(0, -1),
      redoStack: [...s.redoStack, current].slice(-UNDO_CAP),
      lastUndoPush: null,
    }));
    return prev;
  },

  popRedo(current) {
    const stack = get().redoStack;
    if (stack.length === 0) return null;
    const next = stack[stack.length - 1];
    set((s) => ({
      redoStack: s.redoStack.slice(0, -1),
      undoStack: [...s.undoStack, current].slice(-UNDO_CAP),
      lastUndoPush: null,
    }));
    return next;
  },

  clearSketchUndo() {
    set({ undoStack: [], redoStack: [], lastUndoPush: null });
  },

  reset() {
    set({
      session: null,
      sessionGeneration: 0,
      conflictingIds: [],
      entityStates: {},
      constructionMode: false,
      entitySeq: 0,
      constraintSeq: 0,
      undoStack: [],
      redoStack: [],
      lastUndoPush: null,
    });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useSketchStore<T>(selector: (s: SketchState) => T): T {
  return useStore(sketchStore, selector);
}
