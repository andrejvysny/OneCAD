import { useEffect, useId, useState } from "react";
import { useViewportEngine } from "@/viewport/engineBridge";
import type { ActiveToolPresentation, ActiveToolPreviewState } from "@/tools/modelTools/activeToolPresentation";
import type { ToolValueValidation } from "@/stores/toolChipStore";

const FIT_PREVIEW_TOOLS: ReadonlySet<ActiveToolPresentation["tool"]> = new Set([
  "extrudeDepth",
  "revolveAngle",
  "filletRadius",
  "shellThickness",
  "offsetFace",
  "hole",
  "linearPattern",
  "circularPattern",
  "transform",
  "mirror",
  "booleanOp",
]);

function disabledReason(
  engine: ReturnType<typeof useViewportEngine>,
  phase: ActiveToolPresentation["phase"],
  preview: ActiveToolPreviewState,
  validationStatus: ToolValueValidation["status"],
): string | null {
  if (!engine) return "Viewport unavailable";
  if (phase === "applying") return "Applying operation";
  if (validationStatus === "pending") return "Preview is still updating";
  if (validationStatus === "invalid") return "Preview is invalid";
  if (preview === "pending") return "Preview is still updating";
  if (preview === "invalid") return "Preview is invalid";
  if (preview !== "valid") return "No current preview geometry to frame";
  return null;
}

export function FitPreviewButton({
  tool,
  phase,
  preview,
  validationStatus,
  terminal = false,
}: {
  tool: ActiveToolPresentation["tool"];
  phase: ActiveToolPresentation["phase"];
  preview: ActiveToolPreviewState;
  validationStatus: ToolValueValidation["status"];
  terminal?: boolean;
}) {
  const engine = useViewportEngine();
  const [message, setMessage] = useState<string | null>(null);
  const reasonId = useId();
  useEffect(() => {
    setMessage(null);
  }, [tool, phase, preview, validationStatus, engine]);
  if (terminal || !FIT_PREVIEW_TOOLS.has(tool)) return null;
  const reason = disabledReason(engine, phase, preview, validationStatus);
  const disabled = reason !== null;
  const onFit = () => {
    if (!engine || disabled) return;
    setMessage(engine.fitPreview() ? null : "No current preview geometry to frame");
  };

  return (
    <div className="mb-3 min-w-0">
      <button
        type="button"
        data-testid="fit-preview"
        aria-label="Fit preview"
        aria-describedby={reason ? reasonId : undefined}
        title={reason ?? "Fit the current preview geometry"}
        disabled={disabled}
        onClick={onFit}
        className="rounded-md border border-border bg-surface px-2.5 py-1.5 text-[12px] font-medium text-ink-2 shadow-popover disabled:cursor-not-allowed disabled:opacity-50"
      >
        Fit preview
      </button>
      {message && <div role="status" className="mt-1 text-[12px] text-ink-5">{message}</div>}
      {disabled && <span id={reasonId} className="sr-only">{reason}</span>}
    </div>
  );
}
