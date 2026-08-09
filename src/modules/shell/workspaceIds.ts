/*
 * Workspace IDENTITY, split from `workspaces.ts` for the same reason
 * `panelIds.ts` is split from `register.ts`: the session store that holds the
 * active workspace needs the id and nothing else, and importing the definitions
 * would drag every panel id (and, through them, the editor's module graph) into
 * whatever imports the store.
 *
 * The ids sit under `onecad.shell`, not `onecad.simulation`/`onecad.drawing`:
 * a workspace is a PRESENTATION arrangement over whatever modules happen to be
 * installed, and these four are arrangements the application ships. When a real
 * simulation module lands it registers its own workspace under its own
 * namespace and this one goes away — ADR-0006 forbids the shell from squatting
 * another owner's namespace in the meantime.
 */
import { contributionId, type WorkspaceId } from "@/platform";
import { SHELL_MODULE_ID } from "./panelIds";

const workspaceId = (name: string) =>
  contributionId<WorkspaceId>(SHELL_MODULE_ID, `onecad.shell.workspace.${name}`);

export const ShellWorkspaces = {
  Design: workspaceId("design"),
  Simulation: workspaceId("simulation"),
  Drawing: workspaceId("drawing"),
  Visualization: workspaceId("visualization"),
} as const;

/** What the editor opens in, and what an unresolvable active id falls back to. */
export const DEFAULT_WORKSPACE_ID: WorkspaceId = ShellWorkspaces.Design;
