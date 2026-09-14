---
name: tauri-agent-macos-helper
description: Gotchas for the tools/tauri-agent macOS native-input lane — single-file swiftc build, source-hash cache, posted-move readback lag, TCC state on this machine, and the @types/bun root leak
metadata:
  type: project
---

The `tools/tauri-agent` macOS adapter drives a Swift CGEvent helper
(`src/platform/macos/helper/main.swift`) over JSON lines. Facts that cost time to rediscover:

- **The helper must stay one file.** `build.ts` compiles it with a single `swiftc -O -framework
  Cocoa -framework Carbon <src> -o <out>`, so it exceeds the repo's 500-line file guideline by
  design; split it with `// MARK:` extensions inside the file, never into a second file.
- **Cursor readback lags a posted move.** `CGEvent(source:nil)?.location` right after posting a
  mouse-move event still returns the OLD point (WindowServer applies it a few ms later). Verbs
  therefore report the COMMANDED point; only `--selftest` reads back, and only after an 80 ms sleep.
- **The binary is cached by source hash** at `~/.cache/tauri-agent/macos-helper-<sha12>`, so editing
  `main.swift` silently produces a new binary and old ones accumulate. `ensureHelper()` memoises per
  process and clears the memo on failure so a retry recompiles.
- `NSRunningApplication.activate(options: [.activateIgnoringOtherApps])` warns as deprecated since
  macOS 14 ("will have no effect"); it is kept because the plan specifies it, and `MacWindows.focus`
  falls back to `osascript` System Events when the helper reports not-frontmost.
- **TCC on this machine (2026-09-13): Accessibility AND Screen Recording are both granted** —
  `--selftest` exits 0 with both true. Do not assume a preflight will fail here.
- A 1x1 `screencapture` PNG is ~4 KB because macOS embeds an ICC profile, so the "<1 KB means the
  Screen Recording grant is missing" heuristic never trips on a small-but-valid capture.
- **Root `tsconfig.json` has no `types` field**, so `@types/bun` in the ROOT `package.json` leaks
  `bun-types/globals.d.ts` into the app program and narrows `navigator.platform` to
  `"MacIntel"|"Win32"|"Linux x86_64"`, breaking `src/viewport/engine/CadOrbitControls.input.test.ts`.
  Keep bun types scoped to `tools/tauri-agent`.

See [[frontend-gotchas]] for the viewport side of the app this harness drives.
