import { createStore, useStore } from "zustand";

export type ToolChipPlacement =
  | { mode: "anchored" }
  | { mode: "floating"; x: number; y: number }
  | { mode: "docked" };

interface ToolChipPlacementState {
  placement: ToolChipPlacement;
  anchor(): void;
  floatAt(x: number, y: number): void;
  dock(): void;
  reset(): void;
}

const ANCHORED = { mode: "anchored" } as const;

export const toolChipPlacementStore = createStore<ToolChipPlacementState>((set) => ({
  placement: ANCHORED,
  anchor: () => set({ placement: ANCHORED }),
  floatAt: (x, y) => set({ placement: { mode: "floating", x, y } }),
  dock: () => set({ placement: { mode: "docked" } }),
  reset: () => set({ placement: ANCHORED }),
}));

export function useToolChipPlacement<T>(selector: (state: ToolChipPlacementState) => T): T {
  return useStore(toolChipPlacementStore, selector);
}
