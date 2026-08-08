import { useMemo } from "react";
import { cn } from "@/ui/cn";
import { useToolStore, activeTool, type Tool } from "@/stores/toolStore";
import { activateTool } from "@/tools/activateTool";
import { useSelectionStore } from "@/stores/selectionStore";
import { useDocumentStore } from "@/stores/documentStore";
import { getToolApplicability, type ToolApplicability } from "@/tools/modelTools/toolApplicability";
import { usePlatform, useRegistryEntries } from "@/platform";
import { toolbarFromRegistry } from "@/modules/modeling/registryToolbar";
import { ToolButton } from "./ToolButton";
import { isSeparator } from "./toolbarConfig";

const ALWAYS_ENABLED: ToolApplicability = { enabled: true };

/**
 * Centered floating tool pill (prototype 1c). Swaps its tool set with the mode
 * and tints its background in sketch mode (toolbar-sketch token). Every pick
 * routes through `activateTool` — AUTO-MODE: picking a tool can itself cross
 * the mode boundary (a sketch tool from model mode starts a sketch; a model
 * tool from sketch mode finishes it), there is no manual mode toggle.
 *
 * Model-tool buttons gray out when the current selection doesn't satisfy that
 * tool's precondition — same rule `ModelToolController`'s arm functions gate
 * on (`toolApplicability.ts`), so a button never shows enabled and then fails
 * on click. Sketch-tool buttons are untouched (pure pointer gestures, no
 * selection precondition). The ACTIVE tool is exempt from its own check —
 * without this, a tool like Boolean would gray itself out mid-gesture the
 * moment its own arm handshake changes the selection shape it started from.
 *
 * The arrangement is read from the platform TOOL REGISTRY on every render, not
 * from a module-load-time table: a tool registered (or disposed) after mount has
 * to reach the toolbar, which is the whole point of the registry. `useRegistryEntries`
 * is the subscription — it returns the registry's cached, reference-stable
 * snapshot — and the projection is memoized off it, because `toolbarFromRegistry`
 * allocates a fresh array per call and would loop `useSyncExternalStore` if it
 * were the snapshot itself.
 */
export function FloatingToolbar() {
  const mode = useToolStore((s) => s.mode);
  const current = useToolStore(activeTool);
  const selected = useSelectionStore((s) => s.selected);
  const sketches = useDocumentStore((s) => s.sketches);

  const platform = usePlatform();
  const tools = useRegistryEntries(platform.tools);
  const entries = useMemo(
    () => toolbarFromRegistry(platform, mode),
    // `tools` is the subscription: a registry change gives a new snapshot
    // reference, which is what re-runs the projection.
    [platform, tools, mode],
  );

  const pick = (id: Tool) => {
    void activateTool(id);
  };

  return (
    <div
      role="toolbar"
      aria-label="Tools"
      className={cn(
        "absolute left-1/2 top-3 z-30 flex -translate-x-1/2 items-center gap-0.5",
        "rounded-lg border border-border p-1 shadow-card",
        mode === "sketch" ? "bg-toolbar-sketch" : "bg-surface",
      )}
    >
      {entries.map((e, i) => {
        if (isSeparator(e)) {
          return (
            <span
              key={`sep-${i}`}
              aria-hidden="true"
              className="mx-1 h-5 w-px bg-border"
            />
          );
        }
        const isCurrent = current === e.id;
        const verdict =
          mode === "model" && !isCurrent
            ? getToolApplicability(e.id, selected, { sketches })
            : ALWAYS_ENABLED;
        return (
          <ToolButton
            key={e.id}
            icon={e.icon}
            label={e.label}
            shortcut={e.shortcut}
            active={isCurrent}
            onClick={() => pick(e.id)}
            disabled={!verdict.enabled}
            disabledReason={verdict.reason}
          />
        );
      })}
    </div>
  );
}
