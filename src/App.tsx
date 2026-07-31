import { useEffect } from "react";
import DevGallery from "@/app/DevGallery";
import { StartScreen } from "@/features/start/StartScreen";
import { EditorScreen } from "@/features/shell/EditorScreen";
import { UnsavedChangesDialog } from "@/features/shell/UnsavedChangesDialog";
import { createClient } from "@/ipc/client";
import { appStore, useAppStore } from "@/stores/appStore";

/**
 * App shell: switches between the start screen and the (placeholder) editor.
 * `?gallery` still mounts the F-WP1 primitive/icon showcase (DevGallery).
 *
 * The close/quit guard (`close-requested` subscription + UnsavedChangesDialog)
 * lives HERE, not in EditorScreen: Rust prevents the native close/⌘Q and latches
 * `ExitGuard` before emitting, so a screen that never subscribes leaves the event
 * unheard and the guard latched — the second attempt's `begin()` returns false and
 * nothing is emitted at all, making the app unclosable. Mounting at the app root
 * covers the start screen and the gallery route too. A clean/absent document
 * fast-paths straight to `confirmExit` inside `requestClose`, so no dialog shows
 * unless there really are unsaved changes.
 */
function App() {
  const screen = useAppStore((s) => s.screen);
  const params = new URLSearchParams(window.location.search);
  const showGallery = params.has("gallery");
  // Viewport/sketch demos live in the editor shell — boot straight into it so
  // Playwright can exercise them without the start-screen click-through.
  const forceEditor = params.has("vpdemo") || params.has("sketchdemo") || params.has("toolsdemo");

  useEffect(() => {
    return createClient().onCloseRequested(() => {
      void appStore.getState().requestClose("quit");
    });
  }, []);

  const body = showGallery ? (
    <DevGallery />
  ) : screen === "start" && !forceEditor ? (
    <StartScreen />
  ) : (
    <EditorScreen />
  );

  return (
    <>
      {body}
      <UnsavedChangesDialog />
    </>
  );
}

export default App;
