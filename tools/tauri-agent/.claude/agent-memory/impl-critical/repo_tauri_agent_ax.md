---
name: repo-tauri-agent-ax
description: macOS accessibility layer in the tauri-agent Swift helper — the CF-cast trap, verified AX coordinate space, the private window-id symbol, why bounds+title correlation is ambiguous, and walk costs
metadata:
  type: project
---

Facts measured while building the AX read layer in
`tools/tauri-agent/src/platform/macos/helper/main.swift` (verbs `ax_windows`, `ax_snapshot`,
`ax_find`, `ax_point`, `ax_focused_window`, `ax_modal`, `ax_menu`). AX READS, CGEvent ACTS — no verb
there performs an AXAction, and that split is the architecture, not a stage.

- **`as?` to a CoreFoundation class type ALWAYS succeeds in Swift**, and swiftc refuses to compile
  it: *"conditional downcast to CoreFoundation type 'AXUIElement' will always succeed"*. The only
  real check is `CFGetTypeID(v) == AXUIElementGetTypeID()` (or `AXValueGetTypeID()`) followed by a
  force cast. Applies to `AXUIElement`, `AXValue`, `CFArray`. Bridged casts to `String`, `NSNumber`
  and `[AnyObject]` are genuinely checked and are safe.
- **AX screen coordinates are ALREADY global display points, top-left origin, y down** — the same
  space `click`/`move` take. Verified, not assumed: a Chrome `AXWindow`'s `AXPosition`/`AXSize` read
  (0, 33) 1512x949, identical to the same window's `kCGWindowBounds`. Never insert a conversion.
  `CGDisplayBounds` is in that space too; `NSScreen.frame` (bottom-left, AppKit) is NOT.
- **`_AXUIElementGetWindow` is private but present** (resolves via `dlsym` with `RTLD_DEFAULT` =
  `UnsafeMutableRawPointer(bitPattern: -2)`, which Swift does not re-export). It returns exactly the
  CGWindowID `windows` and `screencapture -l` speak. Resolve it at runtime, never link it.
- **The bounds+title fallback for a window id is weak on purpose.** Two windows of one app routinely
  share a rect to the pixel (two Chrome windows: both 0,33 1512x949), and `kCGWindowName` is a
  DECORATED, elided form of the AX title ("Meet – ftf-ywkw-zmq 🔊" vs the full tab title), so exact
  title equality usually fails where it matters. Ambiguity must report a null id, never a guess.
- **`AXWindows` does not only contain `AXWindow`s** — Finder reports its desktop there as an
  `AXScrollArea`, for which `_AXUIElementGetWindow` also fails. Report the role; never assume it.
- **An AX walk costs ~0.26 ms per node** at ~10 attribute reads per node (measured against Chrome and
  Slack). A full Chrome window is ~300 nodes; its whole menu bar is 1,436 nodes / 307 KB of reply /
  362 ms. AX reads are synchronous IPC into the target and the helper answers one verb at a time
  (the TS client serialises through a FIFO), so every walk needs a node cap, a visit cap, a depth cap
  AND a wall-clock deadline, plus `AXUIElementSetMessagingTimeout` on the application element.
- **Use `kill(pid, 0)` for liveness, not `NSRunningApplication(processIdentifier:)`** — the latter
  answers about applications, and an AX target's pid may be a faceless CLI (the test process is).
- **`AXMenuItemCmdModifiers` is the Carbon mask**: bit 0 Shift, bit 1 Option, bit 2 Control, bit 3
  means NO Command (Command is implied when that bit is clear). Verified: ⇧⌘N reports 1, and an item
  with no key equivalent at all reports 8.
- **Testing gotcha**: a GUI target found by sweeping `ps` is not necessarily snapshot-able.
  NotificationCenter has two on-screen AX windows with neither focused nor main, so a
  "focused → main → only" ladder correctly REFUSES it. A test sweep must prefer a rootable target and
  address windows by CGWindowID to stay deterministic.

**Why:** every one of these cost a compile error, a wrong coordinate, or a red test, and none is
visible from the type signatures.
**How to apply:** when touching the AX section of `main.swift` or any consumer of its verbs. The
read-only-to-the-machine rule from [[repo-tauri-agent-native]] still binds: AX reads are fine in
`tests/helperProtocol.test.ts`, AX actions are not.
