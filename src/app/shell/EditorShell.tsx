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
import { useEffect, useLayoutEffect, useMemo } from "react";
import { useShortcuts } from "@/shortcuts/useShortcuts";
import { createClient } from "@/ipc/client";
import { workerStore } from "@/stores/workerStore";
import { repairStore } from "@/stores/repairStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { ViewportRoot } from "@/viewport/ViewportRoot";
import {
  SlotHost,
  Slots,
  usePlatform,
  useRegistryEntries,
  type PanelContribution,
  type Platform,
  type SlotId,
} from "@/platform";
import { activeWorkspace, resolvePanelVisibility } from "@/modules/shell/workspaceLayout";
import { DEFAULT_WORKSPACE_ID } from "@/modules/shell/workspaceIds";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { contributeModelingUi } from "@/modules/modeling/ui";
import { LIBRARY_MODULE_ID } from "@/modules/library/manifest";
import { contributeLibraryUi } from "@/modules/library/register";
import { SHELL_MODULE_ID, contributeShellChrome } from "@/modules/shell/register";

/**
 * The regions inside the work area, in render order. `"viewport"` is the host's
 * own canvas, which is platform-owned and never a contribution (spec §25).
 */
/** Top offset of the fused toolbar stack (was `FloatingToolbar`'s own
 *  `top-3`). Owned here, not by a features/toolbar constant: the stack's
 *  position is a shell layout decision, not either row's own. */
const TOOLBAR_STACK_TOP = 12;

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
  // LAST, and deliberately: a modal has to cover every region above it, and DOM
  // order is what decides that inside a z-index band.
  Slots.ShellOverlay,
];

/** Registers the editor's contributions for as long as the editor is mounted. */
function useEditorContributions(platform: Platform): void {
  // Layout effect, not effect: registering after paint would show one frame of
  // an empty editor.
  useLayoutEffect(() => {
    const modeling = platform.createScope(MODELING_MODULE_ID);
    const shell = platform.createScope(SHELL_MODULE_ID);
    const library = platform.createScope(LIBRARY_MODULE_ID);
    contributeShellChrome(shell);
    contributeModelingUi(modeling);
    contributeLibraryUi(library);
    return () => {
      modeling.dispose();
      shell.dispose();
      library.dispose();
    };
  }, [platform]);
}

/**
 * The active workspace's answer to "is this panel on screen right now?".
 *
 * Resolved HERE rather than inside `SlotHost` because `src/platform/**` may not
 * import an application store; the platform takes a predicate and never learns
 * what a workspace is. See `modules/shell/workspaceLayout.ts` for the rule.
 */
function usePanelFilter(platform: Platform): (panel: PanelContribution) => boolean {
  const workspaces = useRegistryEntries(platform.workspaces);
  const activeId = useWorkspaceStore((s) => s.activeId);
  const hidden = useWorkspaceStore((s) => s.hiddenPanels[s.activeId]);
  return useMemo(() => {
    const workspace = activeWorkspace(workspaces, activeId, DEFAULT_WORKSPACE_ID);
    return resolvePanelVisibility(workspace, hidden ?? []).isVisible;
  }, [workspaces, activeId, hidden]);
}

export function EditorShell() {
  const platform = usePlatform();
  useEditorContributions(platform);
  useShortcuts();
  const isVisible = usePanelFilter(platform);

  // Which of this document's modules this build does not have. Real data: the
  // Rust layer answers `listDocumentModules`, and the diff is what the
  // missing-extension banner reports (ADR-0005 — preserved, and SAID SO).
  useEffect(() => {
    const store = extensionsStore.getState();
    store.reset();
    void store.refresh(platform.moduleIds());
  }, [platform]);

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
      <SlotHost slot={Slots.ShellTop} filter={isVisible} />
      {/* `@container/canvas` gives the toolbar stack below a definite-width
          ancestor to query against. It must NOT go on the stack itself: that
          div is shrink-to-fit (absolute + left-1/2 + -translate-x-1/2, no
          declared width), and `container-type: inline-size` forces size
          containment on the queried element's own inline axis — the exact
          bug the sketch chrome's header comment documents (collapsed the
          pill to ~0px, so every `@min-[…]` read as never-true). This div,
          the flex-1 canvas region, has a real width from layout, so it's
          the query context every collapse tier below reads. */}
      <div className="@container/canvas relative min-h-0 flex-1">
        {EDITOR_REGIONS.map((region) => {
          if (region === "viewport") {
            return <ViewportRoot key={region} className="absolute inset-x-0 bottom-[34px] top-0" />;
          }
          if (region === Slots.ToolbarContextual) {
            // Rendered inside the ToolbarPrimary branch below: the two stack
            // as one fused shell with a single owned width/border/shadow,
            // not two independently positioned regions that only look fused
            // when their content happens to be the same width.
            return null;
          }
          if (region === Slots.ToolbarPrimary) {
            return (
              <div
                key={region}
                style={{ top: TOOLBAR_STACK_TOP }}
                className="absolute left-1/2 z-30 flex -translate-x-1/2 flex-col items-stretch"
              >
                <SlotHost slot={Slots.ToolbarPrimary} filter={isVisible} />
                <SlotHost slot={Slots.ToolbarContextual} filter={isVisible} />
              </div>
            );
          }
          return <SlotHost key={region} slot={region} filter={isVisible} />;
        })}
      </div>
    </div>
  );
}
