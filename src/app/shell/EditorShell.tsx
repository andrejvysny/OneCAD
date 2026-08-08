/*
 * EditorShell — permanent application structure, and nothing else.
 *
 * It owns the window layout and the viewport host. It does NOT know Extrude,
 * Fillet, the model tree or the sketch chrome: everything else on screen is a
 * contribution the modules registered, rendered through slot hosts
 * (docs/ARCHITECTURE.md §6).
 *
 * REGION ORDER IS LOAD-BEARING. The regions are absolutely-positioned siblings
 * over the canvas, so DOM order decides stacking inside a z-index band; the
 * sequence below reproduces the shipped editor exactly and
 * `src/test/contracts/shellContract.ts` is the gate.
 *
 * Contributions register on MOUNT rather than at bootstrap, because the editor
 * tree is a deliberate code-split chunk (see `App.tsx`) and the start screen must
 * not pay for it. They are owned by a scope that dies with this screen.
 */
import { useEffect, useLayoutEffect } from "react";
import { useShortcuts } from "@/shortcuts/useShortcuts";
import { createClient } from "@/ipc/client";
import { workerStore } from "@/stores/workerStore";
import { repairStore } from "@/stores/repairStore";
import { ViewportRoot } from "@/viewport/ViewportRoot";
import { SlotHost, Slots, usePlatform, type Platform, type SlotId } from "@/platform";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { contributeModelingUi } from "@/modules/modeling/ui";
import { SHELL_MODULE_ID, contributeShellChrome } from "@/modules/shell/register";

/**
 * The regions inside the work area, in render order. `"viewport"` is the host's
 * own canvas, which is platform-owned and never a contribution (spec §25).
 */
export const EDITOR_REGIONS: readonly (SlotId | "viewport")[] = [
  "viewport",
  Slots.ViewportOverlay,
  Slots.ToolbarPrimary,
  Slots.ToolbarContextual,
  Slots.ShellLeft,
  Slots.ShellRight,
  Slots.ShellNotification,
  Slots.ViewportChrome,
  Slots.ShellBottom,
];

/** Registers the editor's contributions for as long as the editor is mounted. */
function useEditorContributions(platform: Platform): void {
  // Layout effect, not effect: registering after paint would show one frame of
  // an empty editor.
  useLayoutEffect(() => {
    const modeling = platform.createScope(MODELING_MODULE_ID);
    const shell = platform.createScope(SHELL_MODULE_ID);
    contributeShellChrome(shell);
    contributeModelingUi(modeling);
    return () => {
      modeling.dispose();
      shell.dispose();
    };
  }, [platform]);
}

export function EditorShell() {
  const platform = usePlatform();
  useEditorContributions(platform);
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
    <div className="flex h-full w-full select-none flex-col overflow-hidden bg-surface font-ui">
      <SlotHost slot={Slots.ShellTop} />
      <div className="relative min-h-0 flex-1">
        {EDITOR_REGIONS.map((region) =>
          region === "viewport" ? (
            <ViewportRoot key={region} className="absolute inset-x-0 bottom-[34px] top-0" />
          ) : (
            <SlotHost key={region} slot={region} />
          ),
        )}
      </div>
    </div>
  );
}
