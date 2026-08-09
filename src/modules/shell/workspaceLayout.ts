/*
 * Resolving "which panels does the active workspace show?".
 *
 * This lives in `onecad.shell`, which owns the workspaces, and not in the
 * platform: `SlotHost` takes a predicate and never learns what a workspace is
 * (`src/platform/react/SlotHost.tsx`), and `src/platform/**` may not import an
 * application store at all.
 *
 * THE RULE, and why it is the conservative one:
 *
 *   a panel is hidden only if something SAYS SO — an explicit
 *   `visible: false` placement in the workspace, or a user override from the
 *   customize sheet. A panel the workspace does not mention is shown.
 *
 * The alternative (an unlisted panel is hidden) reads cleaner and is wrong in
 * practice: overlays are owned by whichever tool is running, and a workspace
 * that had to enumerate them would silently swallow every chip and badge a new
 * tool grows until someone remembered to add it to four workspace files. A
 * workspace declares INTENT about the surfaces it cares about; it is not an
 * allow-list of everything that may appear on screen.
 */
import type { PanelContribution, PanelId, WorkspaceDefinition, WorkspaceId } from "@/platform";

export interface PanelVisibility {
  /** The workspace that answered, for callers that want to label the layout. */
  readonly workspace: WorkspaceDefinition | undefined;
  isVisible(panel: PanelContribution): boolean;
}

export function resolvePanelVisibility(
  workspace: WorkspaceDefinition | undefined,
  hiddenPanelIds: readonly string[],
): PanelVisibility {
  const declaredHidden = new Set<string>(
    (workspace?.panels ?? []).filter((p) => p.visible === false).map((p) => p.panelId),
  );
  const userHidden = new Set<string>(hiddenPanelIds);

  return {
    workspace,
    isVisible: (panel) => !declaredHidden.has(panel.id) && !userHidden.has(panel.id),
  };
}

/**
 * The panels a workspace mentions, in placement order, for the customize sheet.
 *
 * Only panels the workspace actually placed are offerable: a sheet listing
 * every registered panel would offer to hide overlays that have no meaning
 * outside the tool that owns them.
 */
export function customizablePanels(
  workspace: WorkspaceDefinition | undefined,
  registered: readonly PanelContribution[],
): readonly PanelContribution[] {
  const byId = new Map<string, PanelContribution>(registered.map((p) => [p.id, p]));
  const seen = new Set<PanelId>();
  const out: PanelContribution[] = [];
  for (const placement of workspace?.panels ?? []) {
    if (seen.has(placement.panelId)) continue;
    seen.add(placement.panelId);
    const panel = byId.get(placement.panelId);
    // A panel with no `title` has no name to show in a settings list, and an
    // overlay/dialog is not a thing a user meaningfully "turns off" — those are
    // driven by their own open-state, not by layout.
    if (panel?.title) out.push(panel);
  }
  return out;
}

export function activeWorkspace(
  workspaces: readonly WorkspaceDefinition[],
  activeId: WorkspaceId,
  fallbackId: WorkspaceId,
): WorkspaceDefinition | undefined {
  return (
    workspaces.find((w) => w.id === activeId) ?? workspaces.find((w) => w.id === fallbackId)
  );
}
