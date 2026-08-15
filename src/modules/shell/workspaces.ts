/*
 * The shipped workspaces.
 *
 * A workspace is a PRESENTATION configuration — which capabilities are
 * prominent for a kind of work — not a set of capabilities (ADR-0001).
 * Switching is a lens over one document, never an app change: the title bar,
 * explorer, inspector, viewport and status bar are the same objects in every
 * one of them.
 *
 * `Design` reproduces today's editor exactly, so the default look is unchanged;
 * what changed is that the arrangement is data the shell reads rather than JSX
 * it hard-codes.
 *
 * HONESTY ABOUT THE OTHER THREE. Simulation, Drawing and Visualization are real
 * arrangements, but the modules whose tools would fill them do not exist yet.
 * Each therefore switches the modeling tool surfaces OFF and shows a
 * `WorkspacePlaceholder` notice naming what is missing, rather than leaving
 * modeling's toolbar on screen under someone else's label — a workspace that
 * silently offers the wrong tools is worse than one that says it is empty.
 *
 * NOTE on the ids: the spec sketches `onecad.workspace.design`, but every
 * contribution id must sit under its owner's namespace (ADR-0006), and these
 * are owned by `onecad.shell` because they compose several modules.
 */
import { Slots, type PanelId, type PanelPlacement, type WorkspaceDefinition } from "@/platform";
import { ModelingPanels } from "@/modules/modeling/panelIds";
import { ShellPanels } from "./panelIds";
import { ShellWorkspaces } from "./workspaceIds";

export { ShellWorkspaces, DEFAULT_WORKSPACE_ID } from "./workspaceIds";

/**
 * Placements every workspace shares. Absent `visible` means visible, so this
 * list documents the layout without becoming a second source of truth for it.
 *
 * Overlays (tool chips, badges, measure UI) are deliberately NOT listed: they
 * are owned by whichever tool is running, and a workspace that enumerated them
 * would have to be edited every time a tool grew a chip.
 */
const COMMON: readonly PanelPlacement[] = [
  { panelId: ShellPanels.TitleBar, slot: Slots.ShellTop },
  { panelId: ModelingPanels.ModelTree, slot: Slots.ShellLeft },
  { panelId: ModelingPanels.Inspector, slot: Slots.ShellRight },
  // Flow strips under the title bar, not floating notices — see the banner's
  // own placement note.
  { panelId: ShellPanels.MissingExtensionBanner, slot: Slots.ShellTop },
  { panelId: ShellPanels.CornerCluster, slot: Slots.ViewportChrome },
  { panelId: ShellPanels.NavPill, slot: Slots.ViewportChrome },
  { panelId: ShellPanels.StatusBar, slot: Slots.ShellBottom },
  { panelId: ShellPanels.CommandPalette, slot: Slots.ShellOverlay },
  { panelId: ShellPanels.ExtensionsManager, slot: Slots.ShellOverlay },
  { panelId: ShellPanels.MissingExtensionDialog, slot: Slots.ShellOverlay },
  { panelId: ShellPanels.CustomizeWorkspaceSheet, slot: Slots.ShellOverlay },
];

/** Modeling's authoring surfaces — on in Design, off wherever it is not the job. */
const MODELING_TOOL_SLOTS: readonly (readonly [PanelId, string])[] = [
  [ModelingPanels.FloatingToolbar, Slots.ToolbarPrimary],
  [ModelingPanels.SketchChromeBar, Slots.ToolbarContextual],
];

const modelingTools = (visible: boolean): PanelPlacement[] =>
  MODELING_TOOL_SLOTS.map(([panelId, slot]) => ({
    panelId,
    slot: slot as PanelPlacement["slot"],
    visible,
  }));

export const DESIGN_WORKSPACE_ID = ShellWorkspaces.Design;

export const DESIGN_WORKSPACE: WorkspaceDefinition = {
  id: ShellWorkspaces.Design,
  title: "Design",
  icon: "cube",
  panels: [...COMMON, ...modelingTools(true)],
};

/** A lens with no module behind it yet: modeling's tools off, a notice on. */
function placeholderWorkspace(
  id: (typeof ShellWorkspaces)[keyof typeof ShellWorkspaces],
  title: string,
  icon: string,
): WorkspaceDefinition {
  return {
    id,
    title,
    icon,
    panels: [
      ...COMMON,
      ...modelingTools(false),
      { panelId: ShellPanels.WorkspacePlaceholder, slot: Slots.ShellTop },
    ],
  };
}

export const SIMULATION_WORKSPACE = placeholderWorkspace(
  ShellWorkspaces.Simulation,
  "Simulation",
  "study",
);
export const DRAWING_WORKSPACE = placeholderWorkspace(
  ShellWorkspaces.Drawing,
  "Drawing",
  "paperPencil",
);
export const VISUALIZATION_WORKSPACE = placeholderWorkspace(
  ShellWorkspaces.Visualization,
  "Visualization",
  "material",
);

export const SHELL_WORKSPACES: readonly WorkspaceDefinition[] = [
  DESIGN_WORKSPACE,
  SIMULATION_WORKSPACE,
  DRAWING_WORKSPACE,
  VISUALIZATION_WORKSPACE,
];
