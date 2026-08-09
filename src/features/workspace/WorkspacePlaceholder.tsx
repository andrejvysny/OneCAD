import { Icon } from "@/icons/Icon";
import { Button } from "@/ui/Button";
import { usePlatform, useRegistryEntries } from "@/platform";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { ShellWorkspaces } from "@/modules/shell/workspaceIds";

/**
 * The notice a workspace shows when the module that would fill it is not
 * installed (prototype 2a's missing-extension language, applied to a lens
 * rather than to document data).
 *
 * Simulation, Drawing and Visualization are registered arrangements with no
 * module behind them yet. Rather than leaving modeling's toolbar on screen
 * under someone else's label — which would offer Extrude inside "Drawing" —
 * the workspace switches those surfaces off and this strip says why. The
 * document, the explorer, the inspector and the viewport stay: switching lens
 * must never hide the model.
 *
 * It renders nothing in a workspace that is not one of the placeholders, so a
 * real module registering its own workspace never trips it.
 */
const PLACEHOLDER_IDS = new Set<string>([
  ShellWorkspaces.Simulation,
  ShellWorkspaces.Drawing,
  ShellWorkspaces.Visualization,
]);

export function WorkspacePlaceholder() {
  const platform = usePlatform();
  const workspaces = useRegistryEntries(platform.workspaces);
  const activeId = useWorkspaceStore((s) => s.activeId);
  const setActive = useWorkspaceStore((s) => s.setActive);

  if (!PLACEHOLDER_IDS.has(activeId)) return null;
  const title = workspaces.find((w) => w.id === activeId)?.title ?? "This workspace";

  return (
    <div
      role="status"
      data-testid="workspace-placeholder"
      className="flex h-[38px] flex-none items-center gap-2.5 border-b border-warn-border bg-warn-surface px-3.5"
    >
      <Icon name="alert" size={15} strokeWidth={1.8} className="flex-none text-warn" />
      <span className="text-[12.5px] text-warn-strong">
        {title} has no tools installed yet. Your model is still here — modeling tools are hidden so
        this workspace does not offer the wrong ones.
      </span>
      <span className="flex-1" />
      <Button
        size="sm"
        variant="secondary"
        data-testid="workspace-placeholder-browse"
        onClick={() => extensionsStore.getState().openManager("browse")}
        className="border-warn-border text-warn-strong"
      >
        Find extensions
      </Button>
      <Button
        size="sm"
        variant="ghost"
        data-testid="workspace-placeholder-back"
        onClick={() => setActive(ShellWorkspaces.Design)}
        className="text-warn"
      >
        Back to Design
      </Button>
    </div>
  );
}
