/*
 * Shell key bindings.
 *
 * Only the two view-navigation chords live here; `Escape` (cancel) and `Enter`
 * (finish sketch) are global in reach but modeling in meaning, so they stay in
 * modeling's table.
 *
 * `shortcuts/keymap.ts` concatenates this after modeling's global table, which
 * reproduces the frozen GLOBAL_KEYS order exactly.
 */
import type { KeyBinding } from "@/shortcuts/keymap";

export const SHELL_GLOBAL_BINDINGS: readonly KeyBinding[] = [
  { key: "h", action: { type: "home" } },
  // ⇧F, not F: the model Fillet tool owns plain `f`, and mode tables win over
  // global ones.
  { key: "f", shift: true, action: { type: "zoomFit" } },
];
