import { selectionStore } from "@/stores/selectionStore";
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
  selectionStore.getState().clear();
  selectionStore.getState().setHover(null);
}
