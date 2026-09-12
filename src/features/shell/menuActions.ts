/*
 * The native-menu router (WP-U1 / decision D-3).
 *
 * Rust's `menu-action` event carries a verb and nothing else (`src-tauri/src/
 * menu.rs`): the backend deliberately does not know what "undo" means, so every
 * menu item lands here and is answered by the SAME code the ⌘-chords already
 * call. Edit → Undo/Redo go through the one undo router (`undoActions`), which
 * re-checks the focus and keeps native text undo inside a field; File → New /
 * Open / Save / Save As reuse the `fileActions` + `appStore` bridges.
 *
 * Nothing is debounced against the keydown lane. On macOS a key equivalent is
 * resolved by exactly one handler: the webview's `performKeyEquivalent:` is
 * asked first and, when it consumes the chord, the menu item never fires (and
 * vice versa) — see the `WryWebView` override in `wry`'s
 * `wkwebview/class/wry_web_view.rs`. A menu click and a keystroke are therefore
 * never the same event twice, and a throttle here would instead swallow a held
 * ⌘Z's key repeat.
 */
import { openDocumentDialog, saveDocument, saveDocumentAs } from "@/features/shell/fileActions";
import { runRedo, runUndo } from "@/features/shell/undoActions";
import { appStore } from "@/stores/appStore";

/** The verbs `src-tauri/src/menu.rs` emits (`action_for`). Lockstep. */
export type MenuAction = "new" | "open" | "save" | "saveAs" | "undo" | "redo";

const MENU_ACTIONS: readonly MenuAction[] = ["new", "open", "save", "saveAs", "undo", "redo"];

/** Whether a `menu-action` payload names a verb this build understands. */
export function isMenuAction(value: unknown): value is MenuAction {
  return typeof value === "string" && (MENU_ACTIONS as readonly string[]).includes(value);
}

/** Run one native-menu verb. Unknown verbs are ignored (forward compatibility). */
export async function runMenuAction(action: MenuAction): Promise<void> {
  switch (action) {
    case "undo":
      await runUndo();
      return;
    case "redo":
      await runRedo();
      return;
    case "new":
      // The same guarded replacement the start screen's "New project" uses —
      // a dirty document still prompts before it is swapped.
      await appStore.getState().newProject();
      return;
    case "open":
      await openDocumentDialog();
      return;
    case "save":
      await saveDocument();
      return;
    case "saveAs":
      await saveDocumentAs();
      return;
  }
}
