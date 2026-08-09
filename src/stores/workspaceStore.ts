/*
 * Workspace store — which workspace is in front, plus the two menus that hang
 * off the title-bar selector.
 *
 * A workspace is a PRESENTATION configuration (ADR-0001): the same document,
 * the same modules, a different arrangement. So the active id lives in SESSION
 * state, not in the document — reopening a file must not silently drag another
 * machine's chosen lens along with it.
 *
 * The id is a plain branded `WorkspaceId` and is deliberately NOT validated
 * against the registry here: a workspace can be registered later than this
 * store is created (the editor's contributions mount with the screen), and a
 * store that rejected an id it had not seen yet would make ordering
 * load-bearing. The shell resolves it and falls back when it does not resolve.
 *
 * `customize` holds the per-workspace override sheet (prototype 2c). It is
 * PREFERENCE state with no backend today: the sheet writes here, the shell
 * reads panel visibility from it, and nothing is persisted yet.
 */
import { createStore, useStore } from "zustand";
import type { PanelId, WorkspaceId } from "@/platform";
import { DEFAULT_WORKSPACE_ID } from "@/modules/shell/workspaceIds";

/** One toggle row in the customize sheet. */
export interface CustomizeToggle {
  readonly id: string;
  readonly label: string;
  readonly on: boolean;
}

export interface WorkspaceState {
  activeId: WorkspaceId;
  /** Title-bar workspace popover. */
  menuOpen: boolean;
  /** "Customize workspace…" sheet. */
  customizeOpen: boolean;
  /**
   * Panels the user switched OFF for the active workspace, keyed by workspace.
   * Absent ⇒ the workspace's own placement decides.
   */
  hiddenPanels: Record<string, readonly string[]>;
  /**
   * Toolbar groups the user switched OFF for the active workspace. A group is
   * `ToolDefinition.group` — the same string the toolbar already reads to place
   * its separators, so hiding one hides a coherent cluster rather than an
   * arbitrary slice.
   */
  hiddenToolGroups: Record<string, readonly string[]>;

  setActive(id: WorkspaceId): void;
  setMenuOpen(open: boolean): void;
  setCustomizeOpen(open: boolean): void;
  isPanelHidden(workspaceId: WorkspaceId, panelId: PanelId): boolean;
  togglePanel(workspaceId: WorkspaceId, panelId: PanelId, on: boolean): void;
  isToolGroupHidden(workspaceId: WorkspaceId, group: string): boolean;
  toggleToolGroup(workspaceId: WorkspaceId, group: string, on: boolean): void;
  resetWorkspace(workspaceId: WorkspaceId): void;
}

export const workspaceStore = createStore<WorkspaceState>((set, get) => ({
  activeId: DEFAULT_WORKSPACE_ID,
  menuOpen: false,
  customizeOpen: false,
  hiddenPanels: {},
  hiddenToolGroups: {},

  setActive: (id) => set({ activeId: id, menuOpen: false }),
  setMenuOpen: (menuOpen) => set({ menuOpen }),
  setCustomizeOpen: (customizeOpen) => set({ customizeOpen, menuOpen: false }),

  isPanelHidden: (workspaceId, panelId) =>
    (get().hiddenPanels[workspaceId] ?? []).includes(panelId),

  togglePanel: (workspaceId, panelId, on) =>
    set((s) => {
      const current = s.hiddenPanels[workspaceId] ?? [];
      const next = on ? current.filter((p) => p !== panelId) : [...new Set([...current, panelId])];
      return { hiddenPanels: { ...s.hiddenPanels, [workspaceId]: next } };
    }),

  isToolGroupHidden: (workspaceId, group) =>
    (get().hiddenToolGroups[workspaceId] ?? []).includes(group),

  toggleToolGroup: (workspaceId, group, on) =>
    set((s) => {
      const current = s.hiddenToolGroups[workspaceId] ?? [];
      const next = on ? current.filter((g) => g !== group) : [...new Set([...current, group])];
      return { hiddenToolGroups: { ...s.hiddenToolGroups, [workspaceId]: next } };
    }),

  resetWorkspace: (workspaceId) =>
    set((s) => {
      const { [workspaceId as string]: _panels, ...hiddenPanels } = s.hiddenPanels;
      const { [workspaceId as string]: _groups, ...hiddenToolGroups } = s.hiddenToolGroups;
      return { hiddenPanels, hiddenToolGroups };
    }),
}));

export function useWorkspaceStore<T>(selector: (s: WorkspaceState) => T): T {
  return useStore(workspaceStore, selector);
}
