import { createStore, useStore } from "zustand";

export type ToolChipPlacement =
  | { mode: "anchored" }
  | { mode: "floating"; x: number; y: number }
  | { mode: "docked" };

interface ToolChipPlacementState {
  /**
   * The user's own choice. It outlives the operation that set it, so it is the
   * thing spec §4.4 forbids an automatic fallback from rewriting: a label that
   * once failed to fit must not make every later operation open docked.
   */
  placement: ToolChipPlacement;
  /**
   * TRANSIENT: this arm's label has no floating footprint, so it renders in the
   * stable fallback slot for now (§4.4, §10.3). Cleared the moment a placement
   * fits again, and never written to {@link ToolChipPlacementState.placement}.
   */
  autoFallback: boolean;
  /** The explicit "Place label" gesture is running: the grip is visible (§4.4). */
  placing: boolean;
  anchor(): void;
  floatAt(x: number, y: number): void;
  dock(): void;
  setAutoFallback(active: boolean): void;
  setPlacing(placing: boolean): void;
  reset(): void;
}

const ANCHORED = { mode: "anchored" } as const;

export const toolChipPlacementStore = createStore<ToolChipPlacementState>((set) => ({
  placement: ANCHORED,
  autoFallback: false,
  placing: false,
  // Every EXPLICIT choice also cancels the transient fallback: the user has just
  // said where the label goes, and leaving the fallback armed would override it.
  anchor: () => set({ placement: ANCHORED, autoFallback: false, placing: false }),
  floatAt: (x, y) => set({ placement: { mode: "floating", x, y }, autoFallback: false, placing: false }),
  dock: () => set({ placement: { mode: "docked" }, autoFallback: false, placing: false }),
  setAutoFallback: (autoFallback) => set({ autoFallback }),
  setPlacing: (placing) => set({ placing }),
  reset: () => set({ placement: ANCHORED, autoFallback: false, placing: false }),
}));

export function useToolChipPlacement<T>(selector: (state: ToolChipPlacementState) => T): T {
  return useStore(toolChipPlacementStore, selector);
}
