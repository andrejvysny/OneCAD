/*
 * The default workspace.
 *
 * A workspace is a PRESENTATION configuration — which capabilities are prominent
 * for a kind of work — not a set of capabilities (ADR-0001). `Design` reproduces
 * today's editor exactly, so nothing about the app looks different; what changes
 * is that the arrangement is now data the shell reads rather than JSX it hard-codes.
 *
 * There is deliberately no workspace switcher in the UI: one workspace does not
 * need chrome to choose between (spec §54).
 *
 * NOTE on the id: the spec sketches `onecad.workspace.design`, but every
 * contribution id must sit under its owner's namespace (ADR-0006), and this
 * workspace is owned by `onecad.shell` because it composes several modules. The
 * enforced rule wins over the sketch.
 */
import { contributionId, Slots, type WorkspaceDefinition, type WorkspaceId } from "@/platform";
import { ModelingPanels } from "@/modules/modeling/panelIds";
import { SHELL_MODULE_ID, ShellPanels } from "./panelIds";

export const DESIGN_WORKSPACE_ID: WorkspaceId = contributionId<WorkspaceId>(
  SHELL_MODULE_ID,
  "onecad.shell.workspace.design",
);

export const DESIGN_WORKSPACE: WorkspaceDefinition = {
  id: DESIGN_WORKSPACE_ID,
  title: "Design",
  icon: "cube",
  // Placements are declarative and match what the modules register. Absent
  // `visible` means visible, so this list documents the layout without being a
  // second source of truth for it.
  panels: [
    { panelId: ShellPanels.TitleBar, slot: Slots.ShellTop },
    { panelId: ModelingPanels.FloatingToolbar, slot: Slots.ToolbarPrimary },
    { panelId: ModelingPanels.SketchChromeBar, slot: Slots.ToolbarContextual },
    { panelId: ModelingPanels.SketchConstraintToolbar, slot: Slots.ToolbarContextual },
    { panelId: ModelingPanels.ModelTree, slot: Slots.ShellLeft },
    { panelId: ModelingPanels.Inspector, slot: Slots.ShellRight },
    { panelId: ShellPanels.CornerCluster, slot: Slots.ViewportChrome },
    { panelId: ShellPanels.NavPill, slot: Slots.ViewportChrome },
    { panelId: ShellPanels.StatusBar, slot: Slots.ShellBottom },
  ],
};
