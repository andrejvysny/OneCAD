import { renderStore } from "@/modules/render/store/renderStore";
import { documentStore } from "@/stores/documentStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { repairStore } from "@/stores/repairStore";
import { selectionStore } from "@/stores/selectionStore";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";

/**
 * Invalidate every UI identity scoped to the document being replaced. Changing
 * the tool first lets mounted controllers cancel preview sessions before stale
 * selection refs are removed.
 */
export function resetDocumentScopedUi(): void {
  toolStore.getState().setMode("model");
  viewportStore.getState().setPendingExtrude(null);
  // Isolation names bodies of the OUTGOING document — carrying it over would
  // hide every body of the incoming one (its ids never match).
  viewportStore.getState().exitIsolate();
  selectionStore.getState().clear();
  selectionStore.getState().setHover(null);
  // Overlay/local UI stores also carry state scoped to the outgoing document — an
  // un-reset repair banner, in-flight sketch undo stack, or armed tool chip would
  // all silently leak into the freshly-opened document. (Suppressed rows need no
  // reset: they ride the projection, which the incoming document replaces.)
  repairStore.getState().reset();
  sketchStore.getState().reset();
  toolChipStore.getState().clear();
  // The missing-extension notice and its dismissal are per-DOCUMENT: a new file
  // gets the notice offered again, and the outgoing file's missing modules must
  // not be reported against it. `EditorShell` re-runs the diff on mount, but a
  // document swapped WITHOUT remounting the editor never triggers that.
  extensionsStore.getState().reset();
  // The rename override names ONE document. `applySnapshot` already drops it on
  // a documentId change; this covers the mock lane, which carries no id.
  documentStore.getState().setDisplayTitle(null);
  // Module-owned document state is per-DOCUMENT (ADR-0004), so a module's live
  // copy of it is document-scoped UI state like everything above. `onecad.render`
  // is the first module with any, and its reset is IN-MEMORY ONLY — this runs
  // BEFORE the swap, and persisting here would clear the OUTGOING document's
  // materials. The incoming document's slice arrives on the next re-hydrate
  // (`modules/render/module.ts`).
  renderStore.getState().reset();
}
