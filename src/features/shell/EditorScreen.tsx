import { useEffect } from "react";
import { useShortcuts } from "@/shortcuts/useShortcuts";
import { createClient } from "@/ipc/client";
import { workerStore } from "@/stores/workerStore";
import { repairStore } from "@/stores/repairStore";
import { TitleBar } from "./TitleBar";
import { StatusBar } from "./StatusBar";
import { NavPill } from "./NavPill";
import { CornerCluster } from "./CornerCluster";
import { FloatingToolbar } from "@/features/toolbar/FloatingToolbar";
import { ModelToolChips } from "@/features/toolbar/ModelToolChips";
import { ModelTreePanel } from "@/features/tree/ModelTreePanel";
import { InspectorPanel } from "@/features/inspector/InspectorPanel";
import { RepairBanner } from "@/features/repair/RepairBanner";
import { SketchChromeBar } from "@/features/sketch/SketchChromeBar";
import { ConstraintBadgeLayer } from "@/features/sketch/ConstraintBadgeLayer";
import { ConstraintContextChips } from "@/features/sketch/ConstraintContextChips";
import { SketchConstraintToolbar } from "@/features/sketch/SketchConstraintToolbar";
import { ViewportRoot } from "@/viewport/ViewportRoot";

/**
 * Editor shell — docked sidebars (left model tree, right inspector) with
 * floating chrome (centered toolbar, status bar, nav pill, corner cluster)
 * over the live Three.js viewport (ViewportRoot). ViewportRoot renders the
 * real canvas and falls back to a hatched placeholder while the engine loads.
 * The status-bar strip occupies the bottom 34px, so the viewport sits above it.
 */
export function EditorScreen() {
  useShortcuts();

  // Relay the C++ sidecar's worker-status events into the store the status bar
  // reads (the real client listens to the backend; the mock never emits).
  useEffect(() => {
    return createClient().onWorkerStatus((s) => workerStore.getState().set(s));
  }, []);

  // Relay `needs-repair` events into the repair store (drives the banner + panel).
  // Emitted after every published regen — empty items means repairs cleared.
  useEffect(() => {
    return createClient().onNeedsRepair((e) => repairStore.getState().applyEvent(e));
  }, []);

  // NOTE: the `close-requested` subscription + <UnsavedChangesDialog/> live in
  // App.tsx, NOT here — the start screen must hear the event too, else the Rust
  // ExitGuard latches on the first attempt and the app becomes unclosable.

  return (
    <div className="flex h-full w-full select-none flex-col overflow-hidden bg-white font-ui">
      <TitleBar />
      <div className="relative min-h-0 flex-1">
        <ViewportRoot className="absolute inset-x-0 bottom-[34px] top-0" />
        <ConstraintBadgeLayer />
        <ConstraintContextChips />
        <ModelToolChips />

        <FloatingToolbar />
        <SketchChromeBar />
        <SketchConstraintToolbar />
        <ModelTreePanel />
        <InspectorPanel />
        <RepairBanner />
        <CornerCluster />
        <NavPill />
        <StatusBar />
      </div>
    </div>
  );
}
