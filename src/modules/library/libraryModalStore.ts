/*
 * Whether the editor-mode `LibraryModal` is open. A plain vanilla store, not
 * React state: the "Library" toolbar tool's `activate`/`deactivate` (a
 * platform `ToolDefinition`, not a component) has to flip this from outside
 * React — same shape `sidebarTabStore`/`tasksStore` already use for the same
 * reason.
 */
import { createStore, useStore } from "zustand";

interface LibraryModalState {
  isOpen: boolean;
  open(): void;
  close(): void;
}

export const libraryModalStore = createStore<LibraryModalState>()((set) => ({
  isOpen: false,
  open() {
    set({ isOpen: true });
  },
  close() {
    set({ isOpen: false });
  },
}));

export function useLibraryModalOpen(): boolean {
  return useStore(libraryModalStore, (s) => s.isOpen);
}
