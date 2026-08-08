import { lazy, Suspense, useEffect, useLayoutEffect, useState } from "react";
import { StartScreen } from "@/features/start/StartScreen";
import { UnsavedChangesDialog } from "@/features/shell/UnsavedChangesDialog";
import { createClient } from "@/ipc/client";
import { appStore, useAppStore } from "@/stores/appStore";
import { PlatformProvider } from "@/platform";
import { bootstrapOneCAD } from "@/app/bootstrap";

// Code-split: the editor tree (three.js, viewport engine, model/sketch tools)
// and the dev gallery are both large and not needed for the start screen's
// first paint. StartScreen idle-prefetches the editor chunk with the EXACT
// same specifier (see its effect) so the New-project transition rarely blocks
// on a cold fetch.
const EditorScreen = lazy(() =>
  import("@/features/shell/EditorScreen").then((m) => ({ default: m.EditorScreen })),
);
const DevGallery = lazy(() => import("@/app/DevGallery"));

/** Full-screen boot fallback shown while a lazy screen chunk loads. */
function BootPanel() {
  return (
    <div className="flex h-full w-full items-center justify-center bg-canvas text-ink">
      <span className="text-[16px] font-bold tracking-[-0.01em]">OneCAD</span>
    </div>
  );
}

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
  // The Platform is built once, here, and passed down — never imported as a
  // global. Building it in a state initializer keeps it available on the FIRST
  // render (a screen may mount contributions immediately) and keeps every test
  // that renders <App/> working without extra setup.
  const [platform] = useState(bootstrapOneCAD);
  const screen = useAppStore((s) => s.screen);
  const params = new URLSearchParams(window.location.search);
  const showGallery = params.has("gallery");
  // Viewport/sketch demos live in the editor shell — boot straight into it so
  // Playwright can exercise them without the start-screen click-through.
  const forceEditor = params.has("vpdemo") || params.has("sketchdemo") || params.has("toolsdemo");

  // Index.html paints a static splash before any JS runs; drop it once React
  // has something to show. useLayoutEffect (not useEffect) so removal happens
  // before the browser paints the first real frame, avoiding a flash of both.
  // Optional chaining: absent under jsdom (index.html isn't loaded in tests)
  // and after StrictMode's double-invoke already removed it once.
  useLayoutEffect(() => {
    document.getElementById("splash")?.remove();
  }, []);

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
    <PlatformProvider platform={platform}>
      {/* StartScreen is eager (never suspends); DevGallery/EditorScreen are the
          two lazy chunks this boundary covers. UnsavedChangesDialog stays
          OUTSIDE it — it must stay mountable even while a screen is loading. */}
      <Suspense fallback={<BootPanel />}>{body}</Suspense>
      <UnsavedChangesDialog />
    </PlatformProvider>
  );
}

export default App;
