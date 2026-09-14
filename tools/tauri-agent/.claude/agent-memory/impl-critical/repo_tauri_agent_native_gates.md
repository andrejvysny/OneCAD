---
name: repo-tauri-agent-native-gates
description: tauri-agent TypeScript native layer — the two point gates and the coordinate space that separates them, the PlatformAdapter fake sites, and the AxTarget narrowing trap
metadata:
  type: project
---

Facts from building the TypeScript half of the accessibility layer in `tools/tauri-agent`
(`src/platform/macos/ax.ts`, `src/native/resolve.ts`, `checkGlobalPoint` in
`src/geometry/mapping.ts`). Companion to [[repo-tauri-agent-ax]], which covers the Swift side.

- **There are TWO point gates and they are not interchangeable.** `checkPoint(css, geom, rects)`
  maps a css point through the calibrated MAIN window and refuses anything outside it — right for
  a webview target, wrong for a native one, because an `NSSavePanel`, a sheet or a menu
  legitimately extends past the main window. `checkGlobalPoint(global, allowedWindows, occlusion)`
  admits a point in ANY window of this app. Never "fix" a native refusal by widening `checkPoint`.
- **`nativeOcclusion.rects` are css px relative to the MAIN window's CONTENT BOX**, not global
  points and not per-window. Testing them against a raw global point silently reinterprets
  `{x:0,y:0,w:80,h:28}` as the primary display's top-left corner. That is why `GlobalOcclusion`
  carries the `WindowGeom` that anchors them, and why the rects are applied only when the point
  landed in that same `windowId`.
- **The AX layer deliberately does NOT apply those rects by default.** The traffic lights, the
  menu bar and panel chrome are precisely what accessibility exists to reach; refusing them as
  "occluders" would defeat the layer. `ResolveAxOpts.occlusion` is opt-in, for a point that came
  from the webview's own idea of where something is.
- **"Belongs to this application" means `NativeWindows.list(pid)`** — CGWindowList asked by pid, so
  ownership is the OS's answer. Its front-to-back order is legitimately used as ORDER there (the
  first containing window is the one that would receive the click), never as identity. A SANDBOXED
  app's open/save panel is hosted by another process and is therefore in neither the AX tree nor
  that list; OneCAD's panels are in-process.
- **Adding a required member to `PlatformAdapter` costs the non-mac adapters nothing** —
  `createWindowsAdapter`/`createLinuxAdapter` THROW rather than returning a literal. It does break
  exactly two test fakes: `fakePlatform` in `tests/fixtures/fakeEnv.ts` and `platformFor` in
  `tests/orchestrator.test.ts`. `tests/fixtures/fakeAx.ts` exports `unusedAx()` for both.
- **`tests/fixtures/fake-helper.ts` takes `--canned=<json path>`** (`{"<verb>": <result>}`,
  consulted before the built-in table), which is how a shaped reply such as `ax_point`'s is driven
  without the real binary. `--fail=<verb>:<code>` still covers the refusal paths.
- **TS trap: `AxFindOpts` is all-optional, so `{ref}` satisfies it too** and `"ref" in target` does
  not narrow. The query arm has to spell `ref?: undefined` — without it a held ref can slide into
  the re-find path, which is the one thing the resolver must never do.

**Why:** each of these is a silently-wrong-coordinate hazard rather than a compile error.
**How to apply:** when touching either gate, the AX resolver, or the `PlatformAdapter` shape.
