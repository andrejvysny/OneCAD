/*
 * Measure store (W2-B) — the bridge from the imperative `ModelToolController`
 * to the React `MeasureOverlay`, mirroring `toolChipStore`'s role for the model
 * tool chips.
 *
 * It holds ONLY presentation state: the picks the user has made and where each
 * label anchors in world space. No document state, no history — measuring is a
 * read, so nothing here is ever persisted or undoable.
 */
import { createStore, useStore } from "zustand";
import type { MeasurePick, MeasureSummary } from "@/tools/modelTools/measureTool";
import type { MassProperties } from "@/ipc/types";

export interface MeasureState {
  /** Measured elements, oldest first (≤2 — see measureTool MAX_PICKS). */
  picks: MeasurePick[];
  /** Centre-to-centre relationship once two picks exist, else null. */
  summary: MeasureSummary | null;
  /**
   * Mass properties of the body the newest pick belongs to (WP-C1), or null
   * while none is loaded.
   *
   * Written by `MeasurePanel`, NOT by the tool controller. The picks above come
   * from an imperative pick handler; this is a derived, async READ keyed off
   * `picks`, and a React effect beside the thing that renders it is the shorter
   * path — nothing else in the app needs it, and the controller would only be
   * relaying. It lives in the store rather than in component state so the whole
   * reading is cleared as one unit (`clear`) and is inspectable from e2e's
   * `__stores` surface like the rest.
   */
  mass: MassProperties | null;
  /** Replace the whole reading (the controller recomputes both together). */
  set(picks: MeasurePick[], summary: MeasureSummary | null): void;
  /** Attach (or drop) the mass reading for the currently measured body. */
  setMass(mass: MassProperties | null): void;
  clear(): void;
}

export const measureStore = createStore<MeasureState>()((set) => ({
  picks: [],
  summary: null,
  mass: null,
  set(picks, summary) {
    // `mass` is deliberately LEFT ALONE. It carries its own `bodyId` and the
    // panel renders it only when that matches the measured body, so a stale
    // reading is filtered at the point of display rather than blanked here —
    // which keeps the card from flickering empty on every re-pick of the same body.
    set({ picks, summary });
  },
  setMass(mass) {
    set({ mass });
  },
  clear() {
    set({ picks: [], summary: null, mass: null });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useMeasureStore<T>(selector: (s: MeasureState) => T): T {
  return useStore(measureStore, selector);
}
