/*
 * Sketch-mode selection store (sketch entity selection + hover foundation).
 *
 * Transient, edit-session-scoped selection of SKETCH entities — deliberately
 * separate from the global selectionStore (persistent document body/sketch/
 * feature/face/edge refs). A pick is a `SketchSel`: an entityId plus an optional
 * `point`. The point being present ⇒ a vertex/handle pick (endpoint / center);
 * absent ⇒ an edge/body pick. Identity for toggling is entityId + point, so the
 * same entity's body and one of its vertices are distinct selections.
 *
 * The store is cleared on sketch enter/exit; a later WP wires pointer events to
 * populate it. ViewportRoot subscribes it into the engine (selected → recolor,
 * hover → hover material).
 *
 * It also carries the CONSTRAINT pick (`selectedConstraintId`, set by clicking a
 * canvas badge) — same edit-session lifetime, and mutually exclusive with the
 * entity selection so `Delete` always has exactly one referent.
 */
import { createStore, useStore } from "zustand";
import type { ConstraintPosition } from "@/ipc/types";

/** A sketch pick: an entity, optionally narrowed to one of its named points. */
export interface SketchSel {
  entityId: string;
  /** Present ⇒ a vertex/handle pick; absent ⇒ an edge/body pick. */
  point?: ConstraintPosition;
}

/** Two picks are the same selection iff entityId AND point match. */
export function sameSketchSel(a: SketchSel, b: SketchSel): boolean {
  return a.entityId === b.entityId && a.point === b.point;
}

export interface SketchSelectionState {
  selected: SketchSel[];
  hover: SketchSel | null;
  constraintHover: string | null;
  /**
   * The constraint picked on canvas (its badge), or null. MUTUALLY EXCLUSIVE
   * with `selected`: a constraint and an entity are two different things to
   * act on, and `Delete` has to know which one the user means without guessing
   * — so writing either side clears the other, in the SAME store write.
   */
  selectedConstraintId: string | null;
  set(sel: SketchSel[]): void;
  toggle(sel: SketchSel): void;
  clear(): void;
  setHover(sel: SketchSel | null): void;
  setConstraintHover(id: string | null): void;
  setSelectedConstraint(id: string | null): void;
}

export const sketchSelectionStore = createStore<SketchSelectionState>()((set) => ({
  selected: [],
  hover: null,
  constraintHover: null,
  selectedConstraintId: null,

  set(sel) {
    set({ selected: sel, selectedConstraintId: null });
  },

  toggle(sel) {
    set((s) => {
      const has = s.selected.some((r) => sameSketchSel(r, sel));
      return {
        selected: has
          ? s.selected.filter((r) => !sameSketchSel(r, sel))
          : [...s.selected, sel],
        selectedConstraintId: null,
      };
    });
  },

  // Full reset — used on sketch enter/exit to drop stale edit-session state.
  clear() {
    set({ selected: [], hover: null, constraintHover: null, selectedConstraintId: null });
  },

  setHover(sel) {
    set({ hover: sel });
  },

  setConstraintHover(id) {
    set({ constraintHover: id });
  },

  setSelectedConstraint(id) {
    // One write, so a badge click cannot be observed mid-way with both an
    // entity selection and a constraint selection live.
    set(id === null ? { selectedConstraintId: null } : { selectedConstraintId: id, selected: [] });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useSketchSelectionStore<T>(selector: (s: SketchSelectionState) => T): T {
  return useStore(sketchSelectionStore, selector);
}
