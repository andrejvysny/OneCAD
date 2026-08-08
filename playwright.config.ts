import { defineConfig } from "@playwright/test";

/*
 * Playwright e2e config for OneCAD (mock-backend lane).
 *
 * The specs run the real Vite app in a plain Chromium (no __TAURI_INTERNALS__),
 * so createClient() falls back to the deterministic MOCK client + localSolver —
 * no Tauri, no C++ worker. Vite is pinned to port 1420 (strictPort) by
 * vite.config.ts, so baseURL is fixed and webServer reuses an already-running
 * `bun run dev` when present.
 *
 * WebGL: the viewport is a real Three.js canvas. Headless Chromium needs the
 * SwiftShader flags below to expose WebGL2 (Chromium gates SW WebGL behind
 * --enable-unsafe-swiftshader). Without a live engine the SketchController is
 * never created and none of the drawing flows exist, so this is load-bearing.
 */
// Default to a dedicated e2e port, NOT vite.config.ts's 1420: 1420 is Tauri's
// dev-server default, so `npm run tauri dev` (or any other Tauri/vite app) is
// routinely listening there — and reuseExistingServer would silently test the
// WRONG app (Playwright only checks the port is open, not whose server it is).
// The webServer command forwards the port to Vite, so the app is identical.
const PORT = Number(process.env.E2E_PORT ?? 4177);
const BASE_URL = `http://localhost:${PORT}`;

export default defineConfig({
  testDir: "./e2e",
  // One Vite dev server + WebGL contention → keep it serial and predictable.
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  // NO retries, in CI or locally. A retried pass reads as green, and that is
  // exactly how the debounced auto-fit regression (ViewportRoot → ViewportEngine
  // `requestAutoFit`) survived: every failure it caused was intermittent, so CI's
  // single retry hid a real product defect while the local lane went hard red.
  // The flakes that motivated the retry are root-caused now — camera state read
  // before it settled, and a spec helper folding the fat-line edge layer into a
  // bbox — so a red run means a defect, which is the only signal worth gating on.
  // Re-introduce a retry only with a named, understood source of nondeterminism.
  retries: 0,
  reporter: process.env.CI ? "line" : "list",
  timeout: 45_000,
  expect: { timeout: 8_000 },
  use: {
    baseURL: BASE_URL,
    // Fixed viewport → the canvas-relative click math is deterministic and the
    // central drawing zone never overlaps the floating DOM panels.
    viewport: { width: 1280, height: 800 },
    trace: "on-first-retry",
    launchOptions: {
      args: [
        "--use-gl=angle",
        "--use-angle=swiftshader",
        "--enable-unsafe-swiftshader",
        "--ignore-gpu-blocklist",
      ],
    },
  },
  projects: [
    { name: "chromium", use: { browserName: "chromium" } },
    { name: "webkit", use: { browserName: "webkit", launchOptions: { args: [] } } },
  ],
  webServer: {
    command: `bun run dev -- --port ${PORT} --strictPort`,
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
