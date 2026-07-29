import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

// Dev-only: expose stores for Playwright/manual debugging (stripped in prod).
if (import.meta.env.DEV) {
  void Promise.all([
    import("./stores/documentStore"),
    import("./stores/sketchStore"),
    import("./stores/viewportStore"),
    import("./stores/toolStore"),
    import("./stores/settingsStore"),
    import("./stores/selectionStore"),
  ]).then(([doc, sketch, viewport, tool, settings, selection]) => {
    (window as unknown as Record<string, unknown>).__stores = {
      document: doc.documentStore,
      sketch: sketch.sketchStore,
      viewport: viewport.viewportStore,
      tool: tool.toolStore,
      settings: settings.settingsStore,
      selection: selection.selectionStore,
    };
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
