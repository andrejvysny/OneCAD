import { useDocumentStore } from "@/stores/documentStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { THEMES, THEME_ORDER, type ThemePref } from "@/theme/themes";
import { FileMenu } from "./FileMenu";
import { closeProject } from "./fileActions";
import { Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";

/**
 * Glyph per preference — chrome-only, so it lives here rather than in the
 * registry (which the engine also reads and has no use for icons).
 */
const THEME_ICON: Record<ThemePref, IconName> = {
  light: "themeLight",
  dark: "themeDark",
  system: "themeSystem",
};

/**
 * Title-bar appearance toggle: shows the CURRENT preference and cycles to the
 * next on click, in the registry's own order (Light → Dark → System → Light).
 *
 * A cycling button is the wrong control for picking one of N — which is exactly
 * why the display-mode button stopped being one — but this is a shortcut, not
 * the picker: the full three-way radio lives in the display popover, and a
 * cycle is what makes a one-click "flip to dark" possible from the title bar.
 * The accessible name always states the current value, so the control is never
 * ambiguous even though its icon changes.
 */
function ThemeToggle() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

  const next = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length];

  return (
    <button
      type="button"
      onClick={() => setTheme(next)}
      aria-label={`Appearance: ${THEMES[theme].label}`}
      title={`Appearance: ${THEMES[theme].label} — switch to ${THEMES[next].label}`}
      className="flex h-[26px] w-[26px] cursor-pointer items-center justify-center rounded-sm text-ink-4 hover:bg-hover hover:text-ink-2"
    >
      <Icon name={THEME_ICON[theme]} size={15} strokeWidth={1.7} />
    </button>
  );
}

/**
 * 44px overlay title bar (prototype 1c). Reserves the left inset for the native
 * macOS traffic lights (Tauri titleBarStyle Overlay draws them over the webview,
 * so the app does not paint its own) and is a drag region. Centered-left doc
 * title + dirty dot, appearance toggle + close on the right. There is
 * deliberately NO mode toggle: the editor mode follows the picked tool + context
 * (AUTO-MODE, see tools/activateTool.ts); the StatusBar shows the current mode
 * as an indicator.
 *
 * Tauri V2 drag: `data-tauri-drag-region` attribute on every element in the bar.
 * Interactive elements (FileMenu button) have their own click handlers so they
 * remain clickable within the drag surface.
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
      {/* No data-tauri-drag-region on interactive controls — it would swallow
          the click into a window drag. */}
      <ThemeToggle />
      <button
        type="button"
        onClick={() => void closeProject()}
        aria-label="Close project"
        className="flex h-[26px] w-[26px] cursor-pointer items-center justify-center rounded-sm text-ink-4 hover:bg-hover hover:text-ink-2"
      >
        <Icon name="x" size={14} strokeWidth={2} />
      </button>
    </div>
  );
}
