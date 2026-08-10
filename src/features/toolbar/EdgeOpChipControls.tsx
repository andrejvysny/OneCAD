/*
 * EdgeOpChipControls — the fillet/chamfer cluster's own controls, plus the
 * `⋯` overflow that now holds the [Fillet|Chamfer] type toggle and the
 * chamfer second leg (mirrors ExtrudeChipControls; UNIFY-UX Phase 1).
 *
 * WHY THE TOGGLE MOVED HERE. The type flip is still reachable in one click —
 * unlike extrude's end-condition/draft settings it stays a PRIMARY decision,
 * just no longer competing with the dimension for the collapsed chip's space,
 * which the arrow (now a real handle, not a whole-viewport claim) needs clear
 * of the chip.
 */
import { useEffect, useState } from "react";
import { cn } from "@/ui/cn";
import { ChipOverflow } from "@/features/toolbar/ChipOverflow";
import { useSettingsStore } from "@/stores/settingsStore";
import { formatLength, parseLength } from "@/units/format";
import type { EdgeOpKind } from "@/tools/modelTools/modelToolMachine";

/** The armed-edge-op segments (FILLET-CHAMFER-UNIFY): the explicit override of
 *  the drag direction's automatic choice. */
const EDGE_OPS: { edgeOp: EdgeOpKind; label: string; testid: string }[] = [
  { edgeOp: "Fillet", label: "Fillet", testid: "chip-edgeop-fillet" },
  { edgeOp: "Chamfer", label: "Chamfer", testid: "chip-edgeop-chamfer" },
];

export function EdgeOpSegments({
  active,
  onPick,
}: {
  active: EdgeOpKind;
  onPick: (edgeOp: EdgeOpKind) => void;
}) {
  return (
    <div className="flex overflow-hidden rounded-full" role="group" aria-label="Edge op">
      {EDGE_OPS.map((o) => (
        <button
          key={o.edgeOp}
          type="button"
          data-testid={o.testid}
          aria-pressed={o.edgeOp === active}
          onClick={() => onPick(o.edgeOp)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            o.edgeOp === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * What the CHAMFER second-leg field shows when there is no second leg: the two
 * legs are EQUAL. Deliberately a glyph, not an empty field — a blank would read
 * as "unset, and I do not know what happens", while `=` states the equal-leg
 * semantics SCHEMA §7.3 gives an absent `distance2`. Typing it back (or clearing
 * the field) is how the user returns to equal-leg.
 */
const EQUAL_LEG_TEXT = "=";

/**
 * The CHAMFER second-distance field (SCHEMA §7.3, 2026-08-03 — WP-C T2a). Rendered
 * ONLY while the armed edge op is a Chamfer.
 *
 * Not a `DimensionInput`: this field's domain is `number | null`, and the
 * equal-leg state has no number to show. `DimensionInput` also owns the
 * `aria-label="Dimension value"` every spec uses to reach the FIRST distance, so
 * a second instance of it would make that locator ambiguous.
 *
 * Commit rules (mirroring the cluster's own): Enter applies the text and then
 * confirms the op; blur applies it; Esc reverts the text to the committed value.
 * Empty or `=` commits `null` — equal-leg. A value the length parser refuses is
 * reverted rather than silently read as a partial number.
 */
export function ChamferDistance2Field({
  value,
  onValue,
  onConfirm,
}: {
  value: number | null;
  onValue: (v: number | null) => void;
  onConfirm?: () => void;
}) {
  const unit = useSettingsStore((s) => s.displayUnit);
  const shown = (v: number | null) => (v === null ? EQUAL_LEG_TEXT : formatLength(v, unit));
  const [text, setText] = useState(() => shown(value));

  // Re-render the SAME value when the committed second leg or the display unit
  // changes; never commits, so a unit switch cannot dirty the document.
  useEffect(() => {
    setText(shown(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, unit]);

  const commit = (): void => {
    const trimmed = text.trim();
    if (trimmed === "" || trimmed === EQUAL_LEG_TEXT) {
      if (value !== null) onValue(null);
      setText(EQUAL_LEG_TEXT);
      return;
    }
    const n = parseLength(trimmed, unit);
    // SCHEMA §7.3 requires `distance2 > 0` when present, and the backend refuses
    // anything else — so a rejected entry reverts rather than authoring it.
    if (n === undefined || n === null || !Number.isFinite(n) || n <= 0) {
      setText(shown(value));
      return;
    }
    if (n !== value) onValue(n);
    setText(shown(n));
  };

  return (
    <span className="pointer-events-auto inline-flex items-center gap-0.5 rounded-full border border-border bg-surface px-2 font-mono text-[11px] text-ink-2 shadow-popover">
      <span aria-hidden className="text-ink-3">
        ×
      </span>
      <input
        aria-label="Second distance"
        data-testid="chip-chamfer-d2"
        className={cn(
          "bg-transparent text-right outline-none",
          unit === "mm" ? "w-9" : "w-14",
        )}
        value={text}
        inputMode="decimal"
        placeholder={EQUAL_LEG_TEXT}
        title="Second chamfer distance — '=' for equal legs"
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.stopPropagation();
            commit();
            onConfirm?.();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            setText(shown(value));
          }
        }}
      />
    </span>
  );
}

export interface EdgeOpOverflowProps {
  edgeOp: EdgeOpKind;
  onEdgeOp: (edgeOp: EdgeOpKind) => void;
  distance2: number | null;
  onDistance2: (distance2: number | null) => void;
  onConfirm?: () => void;
}

/** Short label for the active op — the collapsed chip's readout. */
const EDGE_OP_LABEL: Record<EdgeOpKind, string> = {
  Fillet: "Fillet",
  Chamfer: "Chamfer",
};

/** The `⋯` button plus the [Fillet|Chamfer] segments + chamfer second leg. */
export function EdgeOpOverflow(props: EdgeOpOverflowProps): React.ReactElement {
  return (
    <ChipOverflow
      testId="chip-fillet-overflow"
      ariaLabel="Fillet options"
      title="Fillet or chamfer, and the chamfer second leg"
      readout={EDGE_OP_LABEL[props.edgeOp]}
      marked={props.edgeOp === "Chamfer"}
    >
      <EdgeOpSegments active={props.edgeOp} onPick={props.onEdgeOp} />
      {props.edgeOp === "Chamfer" && (
        <ChamferDistance2Field
          value={props.distance2}
          onValue={props.onDistance2}
          onConfirm={props.onConfirm}
        />
      )}
    </ChipOverflow>
  );
}
