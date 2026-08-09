/*
 * ExtrudeChipControls — the individual controls of the extrude cluster, plus the
 * `⋯` overflow that now holds all but the dimension.
 *
 * WHY THE OVERFLOW EXISTS. A fresh extrude arm used to render a dimension, four
 * end-condition segments, a draft toggle, a symmetric toggle and three boolean
 * segments in one pill — a dozen decisions demanded BEFORE the user has seen a
 * result, most of which the drag itself answers. The chip now carries the one
 * thing a drag cannot state exactly (the number) and hides the rest behind `⋯`.
 *
 * The overflow button is also a READOUT: it shows the boolean mode the direction
 * resolved to, because a mode that changes on its own must never change out of
 * sight. Everything else that is non-default raises a dot on the same button.
 *
 * `BooleanModeSegments` is shared with the REVOLVE cluster, which still renders
 * it inline — collapsing happens in the extrude branch, not in these controls.
 */
import { useEffect, useRef, useState } from "react";
import { cn } from "@/ui/cn";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import type { BooleanMode, ExtrudeEndCondition } from "@/tools/modelTools/modelToolMachine";

/** The armed-cluster boolean segments: New Body / Add / Cut (Wave 2). */
const BOOLEAN_MODES: { mode: BooleanMode; label: string; testid: string }[] = [
  { mode: "NewBody", label: "New Body", testid: "chip-bool-newbody" },
  { mode: "Add", label: "Add", testid: "chip-bool-add" },
  { mode: "Cut", label: "Cut", testid: "chip-bool-cut" },
];



/**
 * The armed-extrude end-condition segments (MODEL-OPS W1). `Symmetric` is NOT
 * here — it stays the ⇔ toggle, so there is one control per concept.
 * `ToNext`/`ToFace` need an existing body to reach, so they disable at zero
 * bodies rather than being offered and failing at commit.
 */
const END_CONDITIONS: {
  end: ExtrudeEndCondition;
  label: string;
  testid: string;
  needsBody: boolean;
}[] = [
  { end: "Blind", label: "Blind", testid: "chip-end-blind", needsBody: false },
  { end: "ThroughAll", label: "Through all", testid: "chip-end-throughall", needsBody: true },
  { end: "ToNext", label: "To next", testid: "chip-end-tonext", needsBody: true },
  { end: "ToFace", label: "To face", testid: "chip-end-toface", needsBody: true },
];

export function EndConditionSegments({
  active,
  canUseBodyEnds,
  onPick,
}: {
  active: ExtrudeEndCondition;
  canUseBodyEnds: boolean;
  onPick: (end: ExtrudeEndCondition) => void;
}) {
  return (
    <div
      className="flex overflow-hidden rounded-full"
      role="group"
      aria-label="End condition"
      title={canUseBodyEnds ? undefined : "Through all / To next / To face need an existing body"}
    >
      {END_CONDITIONS.map((c) => {
        const disabled = c.needsBody && !canUseBodyEnds;
        return (
          <button
            key={c.end}
            type="button"
            data-testid={c.testid}
            aria-pressed={c.end === active}
            disabled={disabled}
            onClick={() => onPick(c.end)}
            className={cn(
              "px-2 py-1 text-[11.5px] font-medium",
              c.end === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
              disabled && "cursor-not-allowed opacity-40 hover:bg-chip",
            )}
          >
            {c.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The [Draft] segment on the armed extrude cluster (WP-C3). `ExtrudeParams`
 * has carried `draftAngleDeg` end-to-end since W-WP6 (the worker applies it with
 * `BRepOffsetAPI_DraftAngle`), but no UI ever authored one — this is that
 * surface.
 *
 * It stays COLLAPSED at 0 so the common no-draft extrude keeps a two-control
 * chip, and expands into a degrees input on click. A NON-ZERO draft is always
 * visible in the button label, because a drafted prism looks nearly identical to
 * a straight one at small angles and the number is the only honest readout.
 */
export function DraftSegment({
  deg,
  onDeg,
  onConfirm,
}: {
  deg: number;
  onDeg: (deg: number) => void;
  onConfirm: () => void;
}) {
  const [open, setOpen] = useState(deg !== 0);
  return (
    <>
      <button
        type="button"
        data-testid="chip-draft"
        aria-pressed={open}
        title="Draft angle applied to the side faces (degrees)"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "rounded-full px-2 py-1 text-[11.5px] font-medium",
          deg !== 0 || open ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
        )}
      >
        {deg === 0 ? "Draft" : `Draft ${deg}°`}
      </button>
      {open && (
        <span data-testid="chip-draft-input">
          <DimensionInput value={deg} suffix="°" onCommit={onDeg} onConfirm={onConfirm} />
        </span>
      )}
    </>
  );
}



/** The ⇔ symmetric toggle on the armed extrude cluster (Alt-drag syncs it). */
export function SymmetricToggle({ pressed, onToggle }: { pressed: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      data-testid="chip-symmetric"
      aria-label="Symmetric"
      aria-pressed={pressed}
      title="Symmetric (hold Alt while dragging)"
      onClick={onToggle}
      className={cn(
        "rounded-full px-2 py-1 text-[11.5px] font-medium",
        pressed ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
      )}
    >
      ⇔
    </button>
  );
}

/**
 * The New Body / Add / Cut segment group on the armed extrude/revolve cluster
 * (Wave 2). Disabled (all three) when no existing body can be a boolean target;
 * the disabled group carries the "Needs an existing body" title.
 */
export function BooleanModeSegments({
  active,
  canBoolean,
  onPick,
}: {
  active: BooleanMode;
  canBoolean: boolean;
  onPick: (mode: BooleanMode) => void;
}) {
  return (
    <div
      className="flex overflow-hidden rounded-full"
      role="group"
      aria-label="Boolean mode"
      title={canBoolean ? undefined : "Needs an existing body"}
    >
      {BOOLEAN_MODES.map((b) => (
        <button
          key={b.mode}
          type="button"
          data-testid={b.testid}
          aria-pressed={b.mode === active}
          disabled={!canBoolean && b.mode !== "NewBody"}
          onClick={() => onPick(b.mode)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            b.mode === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
            !canBoolean && b.mode !== "NewBody" && "cursor-not-allowed opacity-40 hover:bg-chip",
          )}
        >
          {b.label}
        </button>
      ))}
    </div>
  );
}


/** Short label for the resolved boolean mode — the collapsed chip's readout. */
const MODE_LABEL: Record<BooleanMode, string> = {
  NewBody: "New",
  Add: "Add",
  Cut: "Cut",
};

export interface ExtrudeOverflowProps {
  endCondition: ExtrudeEndCondition;
  canUseBodyEnds: boolean;
  showEndConditions: boolean;
  onEndCondition?: (end: ExtrudeEndCondition) => void;
  draftAngleDeg: number;
  showDraft: boolean;
  onDraftAngle?: (deg: number) => void;
  symmetric: boolean;
  showSymmetric: boolean;
  onSymmetric?: (symmetric: boolean) => void;
  booleanMode: BooleanMode;
  canBoolean: boolean;
  showBooleanSegments: boolean;
  onBooleanMode?: (mode: BooleanMode) => void;
  onConfirm?: () => void;
}

/**
 * The `⋯` button plus its popover.
 *
 * DISMISSAL is an outside press or a second click on the button. Esc deliberately
 * still cancels the TOOL: the controller owns Escape from a window listener
 * registered in capture phase at construction time, so nothing mounted later can
 * preempt it, and faking a first-Esc-closes-the-popover rule by racing listener
 * order would be worse than the consistency of Esc always meaning "cancel".
 */
export function ExtrudeOverflow(props: ExtrudeOverflowProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent): void => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    // Capture: the viewport container stops some presses before they bubble.
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  }, [open]);

  // A setting the user cannot see is a setting they cannot undo. The dot says
  // "something in here is not the default" without spelling out what.
  const marked =
    (props.showEndConditions && props.endCondition !== "Blind") ||
    (props.showDraft && props.draftAngleDeg !== 0) ||
    (props.showSymmetric && props.symmetric);

  return (
    <span ref={rootRef} className="relative inline-flex items-center">
      <button
        type="button"
        data-testid="chip-overflow"
        aria-label="Extrude options"
        aria-expanded={open}
        aria-pressed={open}
        title="End condition, draft, symmetric, boolean mode"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "relative rounded-full px-2 py-1 text-[11.5px] font-medium",
          open ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          // The same token `palette.destructive()` tints the prism with, so the
          // chip and the 3D preview say "Cut" in one colour.
          !open && props.booleanMode === "Cut" && "text-traffic-close",
        )}
      >
        <span data-testid="chip-mode-readout">
          {props.showBooleanSegments ? MODE_LABEL[props.booleanMode] : "⋯"}
        </span>
        {marked && (
          <span
            data-testid="chip-overflow-dot"
            aria-hidden="true"
            className="absolute right-0.5 top-0.5 h-1 w-1 rounded-full bg-accent"
          />
        )}
      </button>
      {open && (
        <div
          data-testid="chip-overflow-panel"
          className="absolute bottom-full left-1/2 mb-1.5 flex -translate-x-1/2 flex-col items-start gap-1.5 rounded-2xl border border-border bg-surface px-2 py-2 shadow-popover"
        >
          {props.showEndConditions && (
            <EndConditionSegments
              active={props.endCondition}
              canUseBodyEnds={props.canUseBodyEnds}
              onPick={(end) => props.onEndCondition?.(end)}
            />
          )}
          {props.showBooleanSegments && (
            <BooleanModeSegments
              active={props.booleanMode}
              canBoolean={props.canBoolean}
              onPick={(mode) => props.onBooleanMode?.(mode)}
            />
          )}
          <div className="flex items-center gap-1">
            {props.showDraft && (
              <DraftSegment
                deg={props.draftAngleDeg}
                onDeg={(deg) => props.onDraftAngle?.(deg)}
                onConfirm={() => props.onConfirm?.()}
              />
            )}
            {props.showSymmetric && props.endCondition === "Blind" && (
              <SymmetricToggle
                pressed={props.symmetric}
                onToggle={() => props.onSymmetric?.(!props.symmetric)}
              />
            )}
          </div>
        </div>
      )}
    </span>
  );
}
