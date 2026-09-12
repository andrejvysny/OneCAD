import { createStore, useStore } from "zustand";

export const INSPECTOR_MIN_WIDTH = 280;
export const INSPECTOR_DEFAULT_WIDTH = 320;
export const INSPECTOR_MAX_WIDTH = 420;
export const INSPECTOR_COLLAPSED_WIDTH = 32;
export const INSPECTOR_CHROME_GUTTER = 12;

export function clampInspectorWidth(
  width: number,
  fallback = INSPECTOR_DEFAULT_WIDTH,
): number {
  if (Number.isNaN(width)) {
    return fallback;
  }
  return Math.max(INSPECTOR_MIN_WIDTH, Math.min(INSPECTOR_MAX_WIDTH, width));
}

export interface InspectorLayoutState {
  width: number;
  open: boolean;

  setWidth(width: number): void;
  setOpen(open: boolean): void;
  toggleOpen(): void;
  reset(): void;
}

export const inspectorLayoutStore = createStore<InspectorLayoutState>((set) => ({
  width: INSPECTOR_DEFAULT_WIDTH,
  open: true,

  setWidth: (width) => set((state) => ({
    width: clampInspectorWidth(width, state.width),
  })),
  setOpen: (open) => set({ open }),
  toggleOpen: () => set((state) => ({ open: !state.open })),
  reset: () => set({ width: INSPECTOR_DEFAULT_WIDTH, open: true }),
}));

export function inspectorInset(state: Pick<InspectorLayoutState, "open" | "width">): number {
  return state.open ? state.width : INSPECTOR_COLLAPSED_WIDTH;
}

export function inspectorChromeInset(state: Pick<InspectorLayoutState, "open" | "width">): number {
  return inspectorInset(state) + INSPECTOR_CHROME_GUTTER;
}

export function useInspectorLayoutStore<T>(selector: (state: InspectorLayoutState) => T): T {
  return useStore(inspectorLayoutStore, selector);
}

export function useInspectorInset(): number {
  return useInspectorLayoutStore(inspectorInset);
}

export function useInspectorChromeInset(): number {
  return useInspectorLayoutStore(inspectorChromeInset);
}
