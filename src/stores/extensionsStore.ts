/*
 * Extensions store — the UI state behind the manager, the missing-extension
 * banner and its details dialog.
 *
 * WHAT IS REAL HERE AND WHAT IS NOT.
 *
 * `missing` is REAL: it is the diff between the modules a document carries
 * state for (`CadClient.listDocumentModules`, which the Rust layer answers) and
 * the modules this build registered. That is the ADR-0005 promise made visible
 * — a document opens, its unknown state is preserved, and the user is TOLD.
 *
 * Everything a marketplace would supply — a browsable catalog, available
 * updates, install/uninstall — has no backend at all: there is no addon loader
 * yet (see the closing note in `platform/extension.ts`). Those surfaces render
 * as explicit empty states rather than as fabricated listings, so nobody
 * mistakes a mock for a registry. `dismissed` and `detailsFor` are pure session
 * state.
 */
import { createStore, useStore } from "zustand";
import { createClient } from "@/ipc/client";
import { missingModules, type MissingModule } from "@/platform";
import { logError } from "@/debug/log";

export type ExtensionsTab = "installed" | "browse" | "updates";

export interface ExtensionsState {
  /** Manager modal. */
  managerOpen: boolean;
  tab: ExtensionsTab;
  /** Modules this document needs that this build does not have. */
  missing: readonly MissingModule[];
  /** The banner is per-document: reopening a file offers the notice again. */
  dismissed: boolean;
  /** Module id whose details dialog is open, or null. */
  detailsFor: string | null;

  openManager(tab?: ExtensionsTab): void;
  closeManager(): void;
  setTab(tab: ExtensionsTab): void;
  dismissBanner(): void;
  showDetails(moduleId: string): void;
  closeDetails(): void;
  /** Re-run the diff. `registered` is `platform.moduleIds()`. */
  refresh(registered: readonly string[]): Promise<void>;
  /** Drop per-document state (new document opened). */
  reset(): void;
}

export const extensionsStore = createStore<ExtensionsState>((set) => ({
  managerOpen: false,
  tab: "installed",
  missing: [],
  dismissed: false,
  detailsFor: null,

  openManager: (tab) => set((s) => ({ managerOpen: true, tab: tab ?? s.tab })),
  closeManager: () => set({ managerOpen: false }),
  setTab: (tab) => set({ tab }),
  dismissBanner: () => set({ dismissed: true }),
  showDetails: (moduleId) => set({ detailsFor: moduleId }),
  closeDetails: () => set({ detailsFor: null }),

  async refresh(registered) {
    try {
      const documentModules = await createClient().listDocumentModules();
      set({ missing: missingModules(documentModules, registered) });
    } catch (cause) {
      // A failed probe must not block the editor: the banner simply stays
      // silent, which is the same thing it does when nothing is missing.
      logError("extensions", "listDocumentModules failed", {
        error: cause instanceof Error ? cause.message : String(cause),
      });
    }
  },

  reset: () => set({ missing: [], dismissed: false, detailsFor: null }),
}));

export function useExtensionsStore<T>(selector: (s: ExtensionsState) => T): T {
  return useStore(extensionsStore, selector);
}
