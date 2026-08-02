/*
 * SketchConstraintToolbar (UI a) — the persistent constraint section of the sketch
 * chrome: a centered pill (below the editing chrome) with a button per user-
 * applicable kind — the nine geometric kinds, a separator, then the four
 * dimensional kinds. Each button is ENABLED iff that kind is in the applicable set
 * for the current selection (shared `useApplicableConstraints`); a click routes
 * through `applyConstraint` (geometric → author + reject-on-conflict; dimensional →
 * open the seeded Dimension chip). Shown only while editing a sketch.
 */
import { useMemo, useRef } from "react";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { createClient } from "@/ipc/client";
import { applyConstraint } from "@/tools/sketch/sketchService";
import type { SketchConstraintType } from "@/ipc/types";
import type { ApplicableConstraint } from "@/tools/sketch/constraintApplicability";
import { useApplicableConstraints } from "./useApplicableConstraints";
import { CONSTRAINT_PRESENTATION, DIMENSIONAL_TYPES, GEOMETRIC_TYPES } from "./constraintCatalog";

export function SketchConstraintToolbar() {
  const mode = useToolStore((s) => s.mode);
  const session = useSketchStore((s) => s.session);
  const { applicables } = useApplicableConstraints();
  const clientRef = useRef<ReturnType<typeof createClient> | null>(null);
  if (!clientRef.current) {
    try {
      clientRef.current = createClient();
    } catch {
      clientRef.current = null;
    }
  }

  // First applicable per type (the toolbar offers one button per kind).
  const byType = useMemo(() => {
    const m = new Map<SketchConstraintType, ApplicableConstraint>();
    for (const a of applicables) if (!m.has(a.type)) m.set(a.type, a);
    return m;
  }, [applicables]);

  if (mode !== "sketch" || !session) return null;

  const button = (type: SketchConstraintType) => {
    const applicable = byType.get(type);
    const enabled = applicable !== undefined;
    const { glyph, label } = CONSTRAINT_PRESENTATION[type];
    return (
      <button
        key={type}
        type="button"
        aria-label={label}
        title={label}
        disabled={!enabled}
        onClick={() => {
          if (applicable && clientRef.current) void applyConstraint(clientRef.current, applicable);
        }}
        className={cn(
          "flex h-7 min-w-7 items-center justify-center rounded-sm px-1 text-[13px] font-semibold leading-none transition-colors",
          enabled
            ? "cursor-pointer text-ink-4 hover:bg-hover-3 focus-visible:shadow-focus-ring focus-visible:outline-none"
            : "cursor-default text-ink-5 opacity-40",
        )}
      >
        {glyph}
      </button>
    );
  };

  return (
    <div
      role="toolbar"
      aria-label="Constraints"
      data-testid="sketch-constraint-toolbar"
      className="absolute left-1/2 top-[108px] z-30 flex -translate-x-1/2 items-center gap-0.5 rounded-lg border border-border bg-surface p-1 shadow-card"
    >
      {GEOMETRIC_TYPES.map(button)}
      <span aria-hidden="true" className="mx-1 h-5 w-px bg-border" />
      {DIMENSIONAL_TYPES.map(button)}
    </div>
  );
}
