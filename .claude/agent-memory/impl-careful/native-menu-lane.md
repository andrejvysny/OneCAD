---
name: native-menu-lane
description: OneCAD native-menu (src-tauri/src/menu.rs) seams — muda's main-thread panic makes build() untestable, which OS chords the predefined items silently claim, and the duplicate-client trap when testing a tauriClient event
metadata:
  type: project
---

Durable facts for anyone touching `src-tauri/src/menu.rs`, `events.rs`, or a
`tauriClient` event listener. Learned building the native menu (WP-U1 / D-3).

**Why:** each cost a full cycle to find and none is derivable from CLAUDE.md.

**How to apply:** read before adding a menu item, a backend event, or a vitest
test that constructs a tauri client by hand.

- `menu::build()` is NOT unit-testable, with `tauri::test::mock_app()` or
  without: muda asserts a main-thread marker before creating any menu child
  (`muda-0.19.3/src/platform_impl/macos/mod.rs:328`) and libtest runs every test
  on a spawned thread, so such a test PANICS ("`muda::MenuChild` can only be
  created on the main thread") rather than failing. `--test-threads=1` does not
  help. Test the pure id → verb map (`menu::action_for`) instead and let the
  rendered menu be an owed user-run check.
- The mock runtime runs `run_on_main_thread` tasks INLINE while the event loop is
  not running (`tauri/src/test/mock_runtime.rs:76-91`), so the panic comes from
  muda, not from a deadlock — do not go looking for a channel hang.
- A `PredefinedMenuItem` silently claims its OS chord. `close_window` owns ⌘W,
  which in OneCAD means "close the PROJECT and return to the start screen"
  (`fileActions.closeProject`) and NOT "close the window" (the `CloseRequested`
  guard in `run()` turns that into a quit prompt). It is deliberately absent from
  the File menu for that reason. Check every predefined item's chord against
  `useShortcuts.ts` before adding one.
- macOS dispatches a key equivalent to exactly ONE handler: the webview's
  `performKeyEquivalent:` is asked first and, when it consumes the chord, the
  menu item never fires — and vice versa (see the `WryWebView` override and its
  comment in `wry-0.55.1/src/wkwebview/class/wry_web_view.rs:44-56`). A menu
  accelerator and a JS keydown are therefore never the same press twice, so no
  FE-side debounce is needed (and one would swallow a held ⌘Z's key repeat).
- `createClient()` MEMOIZES the tauri client, but only for itself
  (`client.ts:1057`). A vitest that calls `createTauriClient()` directly AND then
  loads a module doing `createClient()` at module scope (`appStore.ts:17`,
  `fileActions.ts:15`) while `mockIPC` has `__TAURI_INTERNALS__` installed mints
  a SECOND client — and every `emit` is then delivered TWICE. Statically import
  that module at the top of the test file so it loads before the internals flag
  exists.
- `tauriClient` must not statically import `@/features/**`: it would close an
  `ipc → features → ipc/client → ipc` cycle at module-eval time and pull the
  whole tool/shell graph into the module every lane loads. Use a dynamic
  `await import(...)` inside the event callback.
- `ApplyEditScope.noRegenWhen` is the ONLY way to stop the regen awaiter sitting
  out its full timeout for a command that conditionally enqueues nothing.
  `api::undo`/`api::redo` schedule a regen only when the runtime handed back a
  revert (`api/mod.rs:1597`, `:1621`), so an empty stack — and a revert refused
  while a drag gesture is open — needs the undo/redo depth-did-not-move
  predicate, or "Nothing to undo" arrives seconds late.
