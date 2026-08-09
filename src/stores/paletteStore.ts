/*
 * Command-palette store: open/closed and the query. Nothing else.
 *
 * The RESULTS are deliberately not here. They are a projection of
 * `platform.commands` + `platform.tools` + `platform.workspaces` read at render
 * time, so a module registered a second ago is searchable without anything
 * having to push its commands into a store first — the whole point of a
 * registry-derived palette (docs/ARCHITECTURE.md §6).
 */
import { createStore, useStore } from "zustand";

export interface PaletteState {
  open: boolean;
  query: string;
  /** Keyboard cursor into the filtered result list. */
  index: number;

  show(): void;
  hide(): void;
  toggle(): void;
  setQuery(query: string): void;
  setIndex(index: number): void;
}

export const paletteStore = createStore<PaletteState>((set, get) => ({
  open: false,
  query: "",
  index: 0,

  // Opening always starts from an empty query: a palette that remembers the
  // last search makes ⌘K unpredictable — the same keystroke lands somewhere
  // different depending on what you typed ten minutes ago.
  show: () => set({ open: true, query: "", index: 0 }),
  hide: () => set({ open: false, query: "", index: 0 }),
  toggle: () => (get().open ? get().hide() : get().show()),
  setQuery: (query) => set({ query, index: 0 }),
  setIndex: (index) => set({ index }),
}));

export function usePaletteStore<T>(selector: (s: PaletteState) => T): T {
  return useStore(paletteStore, selector);
}
