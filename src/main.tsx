import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./app/ErrorBoundary";
import { logError } from "./debug/log";
import { createClient } from "./ipc/client";
import { installLogForwarder } from "./debug/logSink";
import { startThemeController } from "./theme/themeController";
import "./styles/globals.css";

// The WDIO bridge is bundled only into the feature-built Tauri E2E artifact.
// Ordinary dev/release builds replace this condition with false and drop it.
if (import.meta.env.VITE_TAURI_E2E === "1") {
  await import("@wdio/tauri-plugin");
}

// Before the first render: reconcile the attribute index.html guessed with the
// hydrated preference, and start following the OS. Idempotent.
startThemeController();

// ── Global error capture (DEV-OBSERVABILITY Wave F) ──────────────────────────
// The ErrorBoundary below covers render/lifecycle throws; these two cover what
// it structurally cannot see — a throw from an event handler, a rAF callback,
// or a floating promise. Both are logger-gated, so a shipped build is silent
// unless the user passes `?trace`.
window.addEventListener("error", (e) => {
  logError("err", `uncaught: ${e.message}`, {
    source: e.filename,
    line: e.lineno,
    col: e.colno,
    stack: e.error instanceof Error ? e.error.stack : undefined,
  });
});

window.addEventListener("unhandledrejection", (e) => {
  const reason: unknown = e.reason;
  logError("err", `unhandled rejection: ${reason instanceof Error ? reason.message : String(reason)}`, {
    reason,
  });
});

// Dev-only: expose stores for Playwright/manual debugging (stripped in prod),
// and start forwarding log events to the Rust `log_event` command (a no-op
// outside a Tauri webview, so the mock/e2e lane keeps console + ring only).
if (import.meta.env.DEV) {
  installLogForwarder();
  void Promise.all([
    import("./stores/documentStore"),
    import("./stores/sketchStore"),
    import("./stores/viewportStore"),
    import("./stores/toolStore"),
    import("./stores/settingsStore"),
    import("./stores/selectionStore"),
    import("./stores/measureStore"),
    import("./stores/appStore"),
    import("./stores/toolChipStore"),
    import("./stores/workerStore"),
    import("./stores/repairStore"),
    import("./stores/sketchSelectionStore"),
    import("./stores/liveDimStore"),
  ]).then(
    ([
      doc,
      sketch,
      viewport,
      tool,
      settings,
      selection,
      measure,
      app,
      toolChip,
      worker,
      repair,
      sketchSelection,
      liveDim,
    ]) => {
      (window as unknown as Record<string, unknown>).__stores = {
        document: doc.documentStore,
        sketch: sketch.sketchStore,
        viewport: viewport.viewportStore,
        tool: tool.toolStore,
        settings: settings.settingsStore,
        selection: selection.selectionStore,
        measure: measure.measureStore,
        app: app.appStore,
        toolChip: toolChip.toolChipStore,
        worker: worker.workerStore,
        repair: repair.repairStore,
        sketchSelection: sketchSelection.sketchSelectionStore,
        liveDim: liveDim.liveDimStore,
      };
    },
  );
  // Dev-only CadClient handle (mirrors __stores) — e2e reads WIRE params
  // (`getOperationParams`) that never reach a store, e.g. an Extrude/Revolve's
  // `profile.regionAnchor` (SCHEMA §7.3 WP-B).
  (window as unknown as Record<string, unknown>).__client = createClient();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
