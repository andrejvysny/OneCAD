/*
 * ConstraintMenu — the "discover all" constraint command surface (Main
 * toolbar role in the 3-role model: main toolbar = discover every kind,
 * canvas context = fast selection-based apply via `ConstraintContextChips`,
 * Inspector = manage existing). Same button grid the old, persistent
 * `SketchConstraintToolbar` row rendered — one per kind, enabled iff that
 * kind is in the applicable set for the current selection (shared
 * `useApplicableConstraints`) — now behind a trigger + popover living inside
 * `SketchChromeBar` instead of its own floating pill, retiring a third
 * stacked toolbar row (Sketcher UX cleanup, Track A3).
 */
import { useMemo, useRef, useState } from "react";
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { Button } from "@/ui/Button";
import { Popover } from "@/ui/Popover";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { createClient } from "@/ipc/client";
import { applyConstraint } from "@/tools/sketch/sketchService";
import type { SketchConstraintType } from "@/ipc/types";
import type { ApplicableConstraint } from "@/tools/sketch/constraintApplicability";
import { useApplicableConstraints } from "./useApplicableConstraints";
import { CONSTRAINT_PRESENTATION, DIMENSIONAL_TYPES, GEOMETRIC_TYPES } from "./constraintCatalog";

export function ConstraintMenu() {
  const mode = useToolStore((s) => s.mode);
  const session = useSketchStore((s) => s.session);
  const { applicables } = useApplicableConstraints();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const clientRef = useRef<ReturnType<typeof createClient> | null>(null);
  if (!clientRef.current) {
    try {
      clientRef.current = createClient();
    } catch {
      clientRef.current = null;
    }
  }

  // First applicable per type (one button per kind).
  const byType = useMemo(() => {
    const m = new Map<SketchConstraintType, ApplicableConstraint>();
    for (const a of applicables) if (!m.has(a.type)) m.set(a.type, a);
    return m;
  }, [applicables]);

  if (mode !== "sketch" || !session) return null;

  const button = (type: SketchConstraintType) => {
    const applicable = byType.get(type);
    const enabled = applicable !== undefined;
    const { icon, label } = CONSTRAINT_PRESENTATION[type];
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
          "flex h-7 w-7 items-center justify-center rounded-sm leading-none transition-colors",
          enabled
            ? "cursor-pointer text-ink-4 hover:bg-hover-3 focus-visible:shadow-focus-ring focus-visible:outline-none"
            : "cursor-default text-ink-5 opacity-40",
        )}
      >
        <Icon name={icon} size={17} strokeWidth={1.7} />
      </button>
    );
  };

  return (
    <div className="flex items-center">
      <Button
        ref={trigger}
        size="sm"
        variant="secondary"
        aria-label="Constraints"
        aria-haspopup="dialog"
        aria-expanded={open}
        className={cn(
          "shrink-0 whitespace-nowrap text-ink-3",
          open && "bg-hover-3 text-ink-2",
        )}
        onClick={() => setOpen((v) => !v)}
      >
        {/* Tier 2: icon-bearing label collapses to icon+chevron only — one
            step past Construction's own collapse (`SketchChromeBar`'s
            priority order), narrower than the trigger's own row. */}
        <span className="@max-[760px]/canvas:hidden">Constraints</span>
        <Icon name="chevronDown" size={11} strokeWidth={2} />
      </Button>

      {/* `Popover` renders its own `role="dialog"` wrapper unconditionally —
          matching `aria-haspopup="dialog"` above without this component
          needing to assert a role of its own. The inner toolbar keeps its
          own name; an AT reading the dialog falls through to it. */}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={trigger}
        placement="bottom-start"
        width={196}
        className="p-1"
      >
        <div role="toolbar" aria-label="Constraints">
          <div className="grid grid-cols-4 gap-0.5">{GEOMETRIC_TYPES.map(button)}</div>
          <div aria-hidden="true" className="my-1 h-px bg-border" />
          <div className="grid grid-cols-4 gap-0.5">{DIMENSIONAL_TYPES.map(button)}</div>
        </div>
      </Popover>
    </div>
  );
}
