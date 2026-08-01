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

export interface MeasureState {
  /** Measured elements, oldest first (≤2 — see measureTool MAX_PICKS). */
  picks: MeasurePick[];
  /** Centre-to-centre relationship once two picks exist, else null. */
  summary: MeasureSummary | null;
  /** Replace the whole reading (the controller recomputes both together). */
  set(picks: MeasurePick[], summary: MeasureSummary | null): void;
  clear(): void;
}

export const measureStore = createStore<MeasureState>()((set) => ({
  picks: [],
  summary: null,
  set(picks, summary) {
    set({ picks, summary });
  },
  clear() {
    set({ picks: [], summary: null });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useMeasureStore<T>(selector: (s: MeasureState) => T): T {
  return useStore(measureStore, selector);
}
