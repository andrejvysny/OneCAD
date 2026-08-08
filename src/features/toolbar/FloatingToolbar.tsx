import { cn } from "@/ui/cn";
import { useToolStore, activeTool, type Tool } from "@/stores/toolStore";
import { activateTool } from "@/tools/activateTool";
import { useSelectionStore } from "@/stores/selectionStore";
import { useDocumentStore } from "@/stores/documentStore";
import { getToolApplicability, type ToolApplicability } from "@/tools/modelTools/toolApplicability";
import { ToolButton } from "./ToolButton";
import { toolsForMode, isSeparator } from "./toolbarConfig";

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
 */
export function FloatingToolbar() {
  const mode = useToolStore((s) => s.mode);
  const current = useToolStore(activeTool);
  const selected = useSelectionStore((s) => s.selected);
  const sketches = useDocumentStore((s) => s.sketches);

  const entries = toolsForMode(mode);

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
