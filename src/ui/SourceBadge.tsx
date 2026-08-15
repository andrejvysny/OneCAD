/*
 * The component-library source badge (LGU-1 canvas D1-A): a 9px uppercase pill
 * reading Parametric / Static / Mine, worn on a card thumbnail's top-right
 * corner and beside a detail panel's name.
 *
 * Lives in `src/ui` rather than `features/library` because it is a shape, not a
 * library concept — the KIND is decided by `modules/library/componentMeta.ts`,
 * which owns that judgement and its tests.
 *
 * Colors come from the `--badge-*` token triples. They are deliberately not
 * `--color-*` names (see `tokens.css`), so they arrive as arbitrary values.
 */
import { cn } from "./cn";
import type { SourceBadgeKind } from "@/modules/library/componentMeta";
import { badgeLabelOf } from "@/modules/library/componentMeta";

const TONE: Record<SourceBadgeKind, string> = {
  parametric:
    "text-[var(--badge-parametric-fg)] bg-[var(--badge-parametric-bg)] border-[var(--badge-parametric-line)]",
  static:
    "text-[var(--badge-static-fg)] bg-[var(--badge-static-bg)] border-[var(--badge-static-line)]",
  mine: "text-[var(--badge-mine-fg)] bg-[var(--badge-mine-bg)] border-[var(--badge-mine-line)]",
};

export interface SourceBadgeProps {
  kind: SourceBadgeKind;
  className?: string;
}

export function SourceBadge({ kind, className }: SourceBadgeProps) {
  return (
    <span
      data-testid="source-badge"
      data-kind={kind}
      className={cn(
        "select-none rounded-[4px] border px-1.5 py-[1.5px] text-[9px] font-[750] uppercase leading-none tracking-[0.07em]",
        TONE[kind],
        className,
      )}
    >
      {badgeLabelOf(kind)}
    </span>
  );
}
