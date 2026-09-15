---
name: repo-tauri-agent-bridge
description: Non-obvious facts for tools/tauri-agent — webdriverio promise awaiting, the wdio plugin global, missing jsdom types, importing app code from the agent package, jsdom layout gaps, aria-disabled toolbar buttons
metadata:
  type: project
---

Facts that bit while building the `tools/tauri-agent` semantic layer (T3).

- **`browser.execute` only awaits a returned promise on a BiDi session.** `node_modules/webdriverio/build/index.js` (`async function execute`, ~line 3505) sets `awaitPromise: true` only on the BiDi branch; the classic W3C branch does `return (${script}).apply(null, arguments)` and ships whatever that returns. `@wdio/tauri-service` itself passes async functions to plain `browser.execute` (index.js ~3293, ~3931), so the embedded tauri webdriver evidently does await — but treat it as a boundary and validate the returned shape rather than trusting it. A silently unawaited promise turns a window-geometry read into NaN coordinates.
- **The wdio plugin's page global is `window.wdioTauri`** with `execute` / `waitForInit` / `cleanup*` (`node_modules/@wdio/tauri-plugin/dist-js/index.d.ts`). Not `__wdio*`.
- **No `@types/jsdom` in the repo** although `jsdom` is installed (vitest pulls it). A local ambient `declare module "jsdom"` shim under `tests/` is the way; do not add the dependency.
- **A tauri-agent test CAN import app code directly** — `tests/wheelClass.test.ts` imports `src/viewport/engine/navInput.ts` through a relative `../../../` path and both bun and `tsc -p tools/tauri-agent` accept it (navInput is pure and imports nothing). Prefer a differential test against the real module over mirroring its private constants (`BIG_TICK_PX`, `SMALL_DELTA_PX`, `FAST_GAP_MS`, `SLOW_GAP_MS`, `PRIOR_FLIP_MARGIN` are module-private) and re-reading the file as text — a text/regex comparison proves nothing about behaviour.
- **jsdom has no layout AND no `document.elementFromPoint`.** Any test of the in-page scripts must stub `Element.prototype.getBoundingClientRect` *and* assign `document.elementFromPoint`; otherwise every rect is 0x0 (so the visibility filter drops the whole fixture) and the hit test throws.
- **Toolbar buttons use `aria-disabled`, not `disabled`** (`src/features/toolbar/ToolButton.tsx`), and their classes are Tailwind state utilities that churn on every toggle — so an element fingerprint built from class names reports false staleness. Use `tag#id:nth-of-type` paths.

**Why:** each of these is invisible from the type signatures and only shows up as a wrong coordinate, a wrong element, or a silent skip.
**How to apply:** when touching `tools/tauri-agent/src/semantic/*` or anything that drives the app over the embedded WebDriver.
