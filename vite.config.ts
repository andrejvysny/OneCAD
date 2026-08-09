/// <reference types="vitest/config" />
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import splashPlugin from "./scripts/vite-splash-plugin";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

const STUB_PATH = resolvePath(dirname(fileURLToPath(import.meta.url)), "src/ipc/mockClientStub.ts");

/**
 * Compile-time eviction of the mock kernel (mockClient.ts + its three.js
 * ShapeUtils/Vector2 path via mockRegions.ts) from PRODUCTION builds only.
 * client.ts's `createClient()` must stay sync (no dynamic import), so the mock
 * is always statically imported; this redirects that one specifier to a
 * throwing stub instead, at bundle time. dev/test/e2e are untouched — they
 * still need the real mockClient to drive the UI with no backend.
 */
function evictMockClientInProd(): Plugin {
  return {
    name: "onecad-evict-mock-client",
    enforce: "pre",
    resolveId(source, importer) {
      if (!importer || importer === STUB_PATH) return null; // no self-loop from the stub
      if (!/(^|\/)mockClient$/.test(source)) return null;
      return STUB_PATH;
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => ({
  plugins: [react(), tailwindcss(), splashPlugin(), ...(mode === "production" ? [evictMockClientInProd()] : [])],

  resolve: {
    alias: {
      // The SDK is aliased rather than published: a real package buys nothing
      // until something outside this repo builds against it (P2.5 plan).
      "@onecad/sdk": "/src/sdk/index.ts",
      "@": "/src",
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
    // Pre-transform the boot-critical chain on server start. The editor tree is
    // lazy now (App.tsx code split), so without this the first navigation into
    // the editor pays the whole ~130-module cold transform. Warmup runs in the
    // background and never blocks a request, so listing the editor files costs
    // first paint nothing while making the first editor click instant even if
    // StartScreen's idle-prefetch has not fired yet.
    warmup: {
      clientFiles: [
        "./src/main.tsx",
        "./src/App.tsx",
        "./src/features/start/StartScreen.tsx",
        "./src/features/shell/EditorScreen.tsx",
        "./src/viewport/ViewportRoot.tsx",
      ],
    },
  },

  // Pre-bundle the heavy deps at server start instead of on first discovery.
  // The deep three/examples paths matter most: they are only discovered when
  // the crawler reaches the (now lazily-imported) viewport engine, and a
  // mid-session re-optimize triggers a full page reload — which would also
  // flake e2e. three/webgpu stays excluded: it is a single prebuilt file
  // behind the experimental WebGPU flag; optimizing it cold costs more than
  // serving it raw ever does.
  optimizeDeps: {
    include: [
      "three",
      "three/examples/jsm/lines/Line2.js",
      "three/examples/jsm/lines/LineGeometry.js",
      "three/examples/jsm/lines/LineMaterial.js",
      "three/examples/jsm/lines/LineSegments2.js",
      "three/examples/jsm/lines/LineSegmentsGeometry.js",
      "three/examples/jsm/environments/RoomEnvironment.js",
      "react",
      "react-dom/client",
      "zustand",
      "@tauri-apps/api/core",
      "@tauri-apps/api/event",
    ],
    exclude: ["three/webgpu"],
  },

  // Vitest configuration
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
}));
