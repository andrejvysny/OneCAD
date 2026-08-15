import { useDocumentStore } from "@/stores/documentStore";
import { FileMenu } from "./FileMenu";
import { GeneratorsMenu } from "./GeneratorsMenu";
import { TitleBarButton } from "./TitleBarButton";
import { closeProject } from "./fileActions";
import { WorkspaceSwitcher } from "@/features/workspace/WorkspaceSwitcher";
import { PaletteButton } from "@/features/palette/PaletteButton";

/**
 * 44px overlay title bar (prototype 1c/2a). Reserves the left inset for the
 * native macOS traffic lights (Tauri titleBarStyle Overlay draws them over the
 * webview, so the app does not paint its own) and is a drag region.
 *
 * Left to right: document name, home, workspace, File, Generators. ⌘K on the
 * right. Every button is a `TitleBarButton` — one geometry, one hover, one
 * radius. There is deliberately NO mode toggle: the editor mode follows the
 * picked tool + context (AUTO-MODE, see tools/activateTool.ts) and the
 * StatusBar shows it.
 *
 * No app wordmark: the window itself (macOS traffic lights, Dock icon) already
 * says which app this is, so a second "OneCAD" label here would only repeat
 * that. The document name leads instead — the thing that changes, and the
 * thing most worth finding first as the bar fills up.
 *
 * The title is a LABEL, not a control: renaming lives in File ▸ Rename…, because
 * a click here already means "drag the window" and a rename you can start by
 * mis-clicking the chrome is one you will start by accident.
 *
 * Tauri V2 drag: `data-tauri-drag-region` on every NON-interactive element in
 * the bar. Interactive controls must not carry it — the OS swallows the press
 * into a window drag and the click never fires.
 */
export function TitleBar() {
  const title = useDocumentStore((s) => s.title);
  // Set by File ▸ Rename…; display-only, see `RenameDocumentDialog`.
  const displayTitle = useDocumentStore((s) => s.displayTitle);
  const dirty = useDocumentStore((s) => s.dirty);

  return (
    <div
      data-tauri-drag-region
      className="flex h-[44px] flex-none select-none items-center gap-1 border-b border-border bg-titlebar px-4"
    >
      {/* Native traffic-light reservation (OS-drawn in overlay mode). */}
      <span data-tauri-drag-region aria-hidden="true" className="w-[54px] flex-none" />
      <span
        data-tauri-drag-region
        data-testid="document-title"
        className="mr-1 flex items-center gap-2 text-[13px] font-semibold text-titlebar-text"
      >
        {displayTitle ?? title}
        {dirty && (
          <span
            aria-label="Unsaved changes"
            className="h-[7px] w-[7px] rounded-full bg-ink-5"
          />
        )}
      </span>
      {/* Routes through closeProject(), which prompts save/discard/cancel when
          the document is dirty and returns straight to the start screen when
          it isn't. */}
      <TitleBarButton
        icon="home"
        aria-label="Close project and return to start screen"
        onClick={() => void closeProject()}
      />
      {/* Workspace is GLOBAL context, so it sits with the window's identity and
          never becomes another toolbar layer (prototype 2a). */}
      <WorkspaceSwitcher />
      <FileMenu />
      {/* Parametric shape generators (gears today, more later) — a body-mint
          tool family that doesn't belong in the general modeling toolbar. */}
      <GeneratorsMenu />
      <span data-tauri-drag-region className="flex-1" />
      <PaletteButton />
    </div>
  );
}
