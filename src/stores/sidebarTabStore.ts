/*
 * Which occupant of the LEFT sidebar region is showing. `Slots.ShellLeft`
 * hosts one full-bleed panel worth of space; `ModelTreePanel` and
 * `VariablesPanel` (both modeling) register into it and each render `null`
 * when they aren't the active tab, reading this shared store — a VS
 * Code-style sidebar-tabs pattern that needs no platform/slot changes
 * (`SlotHost` still just mounts every registered panel; only their OWN
 * render output is conditional).
 *
 * UI-only, transient, no persistence — same category as `viewportStore`'s
 * chrome state, not document/document-module state.
 */
import { createStore, useStore } from "zustand";

/** `"materials"` is `onecad.render`'s `MaterialLibraryPanel` — a third occupant
 *  of the same region, registered by that module (`modules/render/ui/register.ts`). */
export type SidebarTab = "model" | "variables" | "materials";

interface SidebarTabState {
  activeTab: SidebarTab;
  setActiveTab(tab: SidebarTab): void;
}

export const sidebarTabStore = createStore<SidebarTabState>()((set) => ({
  activeTab: "model",
  setActiveTab(tab) {
    set({ activeTab: tab });
  },
}));

export function useSidebarTabStore<T>(selector: (s: SidebarTabState) => T): T {
  return useStore(sidebarTabStore, selector);
}
