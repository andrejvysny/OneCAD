import { useDocumentStore } from "@/stores/documentStore";
import { FileMenu } from "./FileMenu";
import { closeProject } from "./fileActions";
import { Icon } from "@/icons/Icon";

/**
 * 44px overlay title bar (prototype 1c). Reserves the left inset for the native
 * macOS traffic lights (Tauri titleBarStyle Overlay draws them over the webview,
 * so the app does not paint its own) and is a drag region. Home button (return
 * to start screen) + file menu on the left, doc title + dirty dot centered.
 * There is deliberately NO mode toggle: the editor mode follows the picked
 * tool + context (AUTO-MODE, see tools/activateTool.ts); the StatusBar shows
 * the current mode as an indicator.
 *
 * Tauri V2 drag: `data-tauri-drag-region` attribute on every element in the bar.
 * Interactive elements (home button, FileMenu button) have their own click
 * handlers so they remain clickable within the drag surface.
 */
export function TitleBar() {
  const title = useDocumentStore((s) => s.title);
  const dirty = useDocumentStore((s) => s.dirty);

  return (
    <div
      data-tauri-drag-region
      className="flex h-[44px] flex-none select-none items-center gap-2 border-b border-border bg-titlebar px-4"
    >
      {/* Native traffic-light reservation (OS-drawn in overlay mode). */}
      <span data-tauri-drag-region aria-hidden="true" className="w-[54px] flex-none" />
      {/* No data-tauri-drag-region on interactive controls — it would swallow
          the click into a window drag. Routes through closeProject(), which
          prompts save/discard/cancel when the document is dirty and returns
          straight to the start screen when it isn't. */}
      <button
        type="button"
        onClick={() => void closeProject()}
        aria-label="Close project and return to start screen"
        className="flex h-[26px] w-[26px] cursor-pointer items-center justify-center rounded-sm text-ink-4 hover:bg-hover hover:text-ink-2"
      >
        <Icon name="home" size={15} strokeWidth={1.7} />
      </button>
      <FileMenu />
      <span
        data-tauri-drag-region
        className="flex items-center gap-2 text-[13px] font-semibold text-titlebar-text"
      >
        OneCAD — {title}
        {dirty && (
          <span
            aria-label="Unsaved changes"
            className="h-[7px] w-[7px] rounded-full bg-ink-5"
          />
        )}
      </span>
      <span data-tauri-drag-region className="flex-1" />
    </div>
  );
}
