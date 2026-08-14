/*
 * `Slots.ShellOverlay` occupant for the editor-mode `LibraryModal` — mirrors
 * `SaveAsComponentHost`'s shape (a thin store-subscribing wrapper around a
 * shared dialog). `onClose` reports through `platform.toolHost`, not a direct
 * store close: the "Library" tool's `deactivate` already closes the store,
 * and going through `report(null)` in one call also clears the toolbar's
 * active-tool highlight, so every close path (Escape, backdrop, Place, X)
 * stays in sync with the toolbar.
 */
import { usePlatform } from "@/platform";
import { useLibraryModalOpen } from "@/modules/library/libraryModalStore";
import { LibraryModal } from "./LibraryModal";

export function LibraryModalHost() {
  const open = useLibraryModalOpen();
  const platform = usePlatform();

  return (
    <LibraryModal
      open={open}
      onClose={() => platform.toolHost.report(null)}
      canPlace
    />
  );
}
