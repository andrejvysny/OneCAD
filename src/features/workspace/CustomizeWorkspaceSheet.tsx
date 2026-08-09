import { useMemo } from "react";
import { Button } from "@/ui/Button";
import { SectionLabel } from "@/ui/SectionLabel";
import { Switch } from "@/ui/Switch";
import { usePlatform, useRegistryEntries, type PanelId } from "@/platform";
import { activeWorkspace, customizablePanels } from "@/modules/shell/workspaceLayout";
import { DEFAULT_WORKSPACE_ID } from "@/modules/shell/workspaceIds";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { useViewportStore } from "@/stores/viewportStore";
import { LAYER_GROUPS, useLayersStore, type SceneLayer } from "@/stores/layersStore";

/**
 * "Customize workspace…" (prototype 2c) — workspace defaults plus this user's
 * overrides, in one small sheet.
 *
 * Every row here is DERIVED, never enumerated: panels come from what the active
 * workspace placed, tool groups from `ToolDefinition.group`, layers from
 * `LAYER_GROUPS`. A hard-coded list would go stale the first time a module
 * registered something, which is the failure this whole refactor is about.
 *
 * Overrides are session state (`workspaceStore`) with no persistence yet.
 * "Reset" clears them, which is why it is worth having even before they are
 * saved anywhere: it is the way back from a layout you did not mean to make.
 */
export function CustomizeWorkspaceSheet() {
  const open = useWorkspaceStore((s) => s.customizeOpen);
  const setCustomizeOpen = useWorkspaceStore((s) => s.setCustomizeOpen);
  const activeId = useWorkspaceStore((s) => s.activeId);
  const hiddenPanels = useWorkspaceStore((s) => s.hiddenPanels[s.activeId]);
  const hiddenGroups = useWorkspaceStore((s) => s.hiddenToolGroups[s.activeId]);
  const togglePanel = useWorkspaceStore((s) => s.togglePanel);
  const toggleToolGroup = useWorkspaceStore((s) => s.toggleToolGroup);
  const resetWorkspace = useWorkspaceStore((s) => s.resetWorkspace);

  const platform = usePlatform();
  const workspaces = useRegistryEntries(platform.workspaces);
  const panels = useRegistryEntries(platform.panels);
  const tools = useRegistryEntries(platform.tools);

  const layerVisible = useLayersStore((s) => s.visible);
  const setLayerVisible = useLayersStore((s) => s.setVisible);
  const gridVisible = useViewportStore((s) => s.gridVisible);
  const toggleGrid = useViewportStore((s) => s.toggleGrid);

  const workspace = activeWorkspace(workspaces, activeId, DEFAULT_WORKSPACE_ID);
  const offerable = useMemo(
    () => customizablePanels(workspace, panels),
    [workspace, panels],
  );
  const groups = useMemo(
    () => [...new Set(tools.map((t) => t.group).filter((g): g is string => Boolean(g)))],
    [tools],
  );

  if (!open) return null;

  const hiddenPanelSet = new Set(hiddenPanels ?? []);
  const hiddenGroupSet = new Set(hiddenGroups ?? []);
  const title = workspace?.title ?? "workspace";

  return (
    <div
      className="absolute inset-0 z-[85] flex items-center justify-center bg-scrim"
      onClick={() => setCustomizeOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Customize ${title} workspace`}
        data-testid="customize-workspace"
        onClick={(e) => e.stopPropagation()}
        className="max-h-[calc(100%-96px)] w-[380px] overflow-auto rounded-lg border border-border bg-surface p-[20px_22px] shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Customize {title} workspace</div>
        <div className="mt-[3px] text-[12px] text-ink-5">Workspace defaults plus your overrides.</div>

        <SectionLabel className="pb-1 pt-4">Panels</SectionLabel>
        {offerable.length === 0 && <Note>This workspace places no named panels.</Note>}
        {offerable.map((panel) => (
          <Row
            key={panel.id}
            label={panel.title ?? panel.id}
            testid={`customize-panel-${panel.id}`}
            on={!hiddenPanelSet.has(panel.id)}
            onChange={(on) => togglePanel(activeId, panel.id as PanelId, on)}
          />
        ))}

        <SectionLabel className="pb-1 pt-3.5">Tool groups</SectionLabel>
        {groups.length === 0 && <Note>No tools are registered in this workspace.</Note>}
        {groups.map((group) => (
          <Row
            key={group}
            label={group}
            testid={`customize-group-${group}`}
            on={!hiddenGroupSet.has(group)}
            onChange={(on) => toggleToolGroup(activeId, group, on)}
          />
        ))}

        <SectionLabel className="pb-1 pt-3.5">Viewport layers</SectionLabel>
        {LAYER_GROUPS.flatMap((g) => g.layers).map((layer) => (
          <Row
            key={layer.key}
            label={layer.label}
            testid={`customize-layer-${layer.key}`}
            on={layer.key === "grid" ? gridVisible : layerVisible[layer.key as SceneLayer]}
            onChange={(on) => {
              if (layer.key === "grid") toggleGrid();
              else setLayerVisible(layer.key as SceneLayer, on);
            }}
          />
        ))}

        <div className="mt-[18px] flex items-center border-t border-border-subtle pt-3.5">
          <Button
            variant="ghost"
            size="md"
            data-testid="customize-reset"
            onClick={() => resetWorkspace(activeId)}
          >
            Reset {title} workspace
          </Button>
          <span className="flex-1" />
          <Button data-testid="customize-done" onClick={() => setCustomizeOpen(false)}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}

function Row({
  label,
  on,
  onChange,
  testid,
}: {
  label: string;
  on: boolean;
  onChange: (on: boolean) => void;
  testid: string;
}) {
  return (
    <div className="flex h-8 items-center gap-2" data-testid={testid}>
      <span className="flex-1 truncate text-[13px] text-ink-2">{label}</span>
      <Switch checked={on} onChange={onChange} ariaLabel={label} />
    </div>
  );
}

function Note({ children }: { children: string }) {
  return <div className="py-1 text-[12px] text-ink-6">{children}</div>;
}
