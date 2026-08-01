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
}
