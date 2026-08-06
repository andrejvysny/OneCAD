/*
 * ModelToolChips — the floating overlay chips for the model tools (F-WP7 +
 * M6b): extrude depth, fillet radius, revolve angle, the boolean op picker, and
 * the M6b shell-thickness / linear-pattern / circular-pattern / mirror chips.
 * Content is React; POSITIONING is imperative — an engine-owned host node is
 * registered with the HTML overlay driver so it tracks a world anchor every
 * frame with no React re-render.
 *
 * The host node is created once (never part of React's managed layout); the
 * engine appends it to the overlay and the driver transforms it. We `createPortal`
 * the chip content INTO that host, so React only manages the content, never the
 * moved node — avoiding the "removeChild: not a child" reconciliation crash.
 *
 * The multi-control chips (boolean op, patterns, mirror) are intentionally
 * minimal button rows — a proper op popover / gizmo lands with the param-dialog
 * work (design TODO acknowledged).
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/ui/cn";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { HoleChipCluster } from "./HoleChipCluster";
import { useSettingsStore } from "@/stores/settingsStore";
import { useToolChipStore, toolChipStore } from "@/stores/toolChipStore";
import { LENGTH_SUFFIX, formatLength, parseLength } from "@/units/format";
import { useViewportEngine } from "@/viewport/engineBridge";
import type { BooleanOperation, OffsetDistanceType } from "@/ipc/types";
import type {
  AlignPhase,
  PatternAxis,
  MirrorPlane,
  BooleanMode,
  EdgeOpKind,
  ExtrudeEndCondition,
  TransformMode,
} from "@/tools/modelTools/modelToolMachine";

const CHIP_ID = "__model_tool_chip";
const BOOLEAN_OPS: BooleanOperation[] = ["Union", "Cut", "Intersect"];
const PATTERN_AXES: PatternAxis[] = ["X", "Y", "Z"];
const MIRROR_PLANES: MirrorPlane[] = ["XY", "XZ", "YZ"];

/** The armed-placement mode segments (WP-B W1). */
const TRANSFORM_MODES: { mode: TransformMode; label: string; testid: string }[] = [
  { mode: "move", label: "Move", testid: "chip-transform-move" },
  { mode: "rotate", label: "Rotate", testid: "chip-transform-rotate" },
];

/** The armed-cluster boolean segments: New Body / Add / Cut (Wave 2). */
const BOOLEAN_MODES: { mode: BooleanMode; label: string; testid: string }[] = [
  { mode: "NewBody", label: "New Body", testid: "chip-bool-newbody" },
  { mode: "Add", label: "Add", testid: "chip-bool-add" },
  { mode: "Cut", label: "Cut", testid: "chip-bool-cut" },
];

/** The armed-edge-op segments (FILLET-CHAMFER-UNIFY): the explicit override of
 *  the drag direction's automatic choice. */
const EDGE_OPS: { edgeOp: EdgeOpKind; label: string; testid: string }[] = [
  { edgeOp: "Fillet", label: "Fillet", testid: "chip-edgeop-fillet" },
  { edgeOp: "Chamfer", label: "Chamfer", testid: "chip-edgeop-chamfer" },
];

/**
 * The armed OFFSET-FACE distance-type segments (SCHEMA §7.3). WHICH of these are
 * rendered is decided by the controller and passed through the chip store — a
 * planar face offers `Offset`/`Total`, a cylindrical one `Offset`/`Radius`/
 * `Diameter`, and a multi-face closure only `Offset`.
 */
const OFFSET_DISTANCE_TYPES: { type: OffsetDistanceType; label: string; testid: string }[] = [
  { type: "Offset", label: "Offset", testid: "chip-offset-type-offset" },
  { type: "Total", label: "Total", testid: "chip-offset-type-total" },
  { type: "Radius", label: "Radius", testid: "chip-offset-type-radius" },
  { type: "Diameter", label: "Diameter", testid: "chip-offset-type-diameter" },
];

/**
 * What the CHAMFER second-leg field shows when there is no second leg: the two
 * legs are EQUAL. Deliberately a glyph, not an empty field — a blank would read
 * as "unset, and I do not know what happens", while `=` states the equal-leg
 * semantics SCHEMA §7.3 gives an absent `distance2`. Typing it back (or clearing
 * the field) is how the user returns to equal-leg.
 */
const EQUAL_LEG_TEXT = "=";

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

function EndConditionSegments({
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
function DraftSegment({
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

/** A segmented toggle row (axis / plane pickers), styled like the boolean op row. */
function SegmentToggle<T extends string>({
  options,
  active,
  onPick,
  label,
  testid,
}: {
  options: readonly T[];
  active: T;
  onPick: (v: T) => void;
  label: string;
  /**
   * Per-segment `data-testid`. The chips live under the viewport's overlay root,
   * which is `aria-hidden` (it decorates a canvas), so role/name queries cannot
   * reach them — a testid is the ONLY handle e2e has. Optional because the
   * pattern/mirror chips predate that need and have no spec depending on them.
   */
  testid?: (v: T) => string;
}) {
  return (
    <div className="flex overflow-hidden rounded-full" role="group" aria-label={label}>
      {options.map((o) => (
        <button
          key={o}
          type="button"
          data-testid={testid?.(o)}
          aria-pressed={o === active}
          onClick={() => onPick(o)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            o === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          )}
        >
          {o}
        </button>
      ))}
    </div>
  );
}

/** A −/n/+ count stepper for the pattern instance count. */
function CountStepper({ count, onCount }: { count: number; onCount: (n: number) => void }) {
  return (
    <div className="inline-flex items-center gap-0.5">
      <button
        type="button"
        aria-label="Fewer instances"
        onClick={() => onCount(count - 1)}
        className="flex h-5 w-5 items-center justify-center rounded-full bg-chip text-ink-3 hover:bg-hover-2"
      >
        −
      </button>
      <span
        data-testid="pattern-count"
        className="min-w-[16px] text-center font-mono text-[11.5px] text-ink-2"
      >
        {count}
      </span>
      <button
        type="button"
        aria-label="More instances"
        onClick={() => onCount(count + 1)}
        className="flex h-5 w-5 items-center justify-center rounded-full bg-chip text-ink-3 hover:bg-hover-2"
      >
        +
      </button>
    </div>
  );
}

/** The accent Apply button shared by the boolean / pattern / mirror chips. */
function ApplyButton() {
  return (
    <button
      type="button"
      onClick={() => toolChipStore.getState().onApply?.()}
      className="rounded-full bg-accent px-2 py-1 text-[11.5px] font-medium text-on-accent hover:opacity-90"
    >
      Apply
    </button>
  );
}

/** The shared ✓/✕ commit/cancel pair on the armed extrude / revolve cluster. */
function ConfirmButtons({ onConfirm, onCancel }: { onConfirm?: () => void; onCancel?: () => void }) {
  return (
    <>
      <button
        type="button"
        data-testid="chip-confirm"
        aria-label="Confirm"
        onClick={() => onConfirm?.()}
        className="rounded-full bg-accent px-2 py-1 text-[11.5px] font-medium text-on-accent hover:opacity-90"
      >
        ✓
      </button>
      <button
        type="button"
        data-testid="chip-cancel"
        aria-label="Cancel"
        onClick={() => onCancel?.()}
        className="rounded-full bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
      >
        ✕
      </button>
    </>
  );
}

/** The ⇔ symmetric toggle on the armed extrude cluster (Alt-drag syncs it). */
function SymmetricToggle({ pressed, onToggle }: { pressed: boolean; onToggle: () => void }) {
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
function BooleanModeSegments({
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

/**
 * The [Fillet | Chamfer] segment group on the armed edge-op cluster. Both
 * segments are always available — unlike the boolean modes there is no
 * precondition to check (any picked edge can take either op; whether OCCT
 * accepts the SIZE is what the live preview failure reports). A drag that
 * re-types the op moves the pressed segment, so this doubles as the readout of
 * what the direction just chose.
 */
function EdgeOpSegments({
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
 * The `[Offset | Total | Radius | Diameter]` segment group on the armed
 * offset-face cluster (SCHEMA §7.3).
 *
 * Only the types in `allowed` are RENDERED — an unavailable one is absent, not
 * disabled: unlike the boolean modes (which disable at zero bodies and say so in
 * a title), a `Radius` on a planar face is not "not yet possible", it is not a
 * thing. Offering it greyed would suggest the face could grow a radius.
 *
 * A group with a single member renders nothing at all: one segment is not a
 * choice, and the `Offset`-only multi-face case has no decision to present.
 */
function DistanceTypeSegments({
  active,
  allowed,
  onPick,
}: {
  active: OffsetDistanceType;
  allowed: readonly OffsetDistanceType[];
  onPick: (t: OffsetDistanceType) => void;
}) {
  const shown = OFFSET_DISTANCE_TYPES.filter((o) => allowed.includes(o.type));
  if (shown.length < 2) return null;
  return (
    <div className="flex overflow-hidden rounded-full" role="group" aria-label="Distance type">
      {shown.map((o) => (
        <button
          key={o.type}
          type="button"
          data-testid={o.testid}
          aria-pressed={o.type === active}
          onClick={() => onPick(o.type)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            o.type === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * The tangent-chain toggle on the armed offset-face cluster. ON by default: the
 * kernel auto-propagates an offset across G1-tangent junctions and CANNOT hold a
 * tangent neighbour fixed (spike-characterized), so switching it off is a
 * declaration that the closure had better already be complete — and
 * `PrepareOffsetFace` refuses with `chainMismatch` when it is not.
 *
 * Hidden for `Total`, which is single-face with the chain off by definition.
 */
function TangentToggle({
  pressed,
  disabled,
  onToggle,
}: {
  pressed: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      data-testid="chip-offset-tangent"
      aria-label="Follow tangent faces"
      aria-pressed={pressed}
      disabled={disabled}
      title={
        disabled
          ? "A Total thickness measures one face against its opposite — no chain"
          : "Include tangent-connected faces"
      }
      onClick={onToggle}
      className={cn(
        "rounded-full px-2 py-1 text-[11.5px] font-medium",
        pressed ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
        disabled && "cursor-not-allowed opacity-40 hover:bg-chip",
      )}
    >
      ⌒
    </button>
  );
}

/**
 * The CHAMFER second-distance field (SCHEMA §7.3, 2026-08-03 — WP-C T2a). Rendered
 * ONLY while the armed edge op is a Chamfer, immediately right of the first
 * distance, so the cluster reads `[d1] [d2] [Fillet|Chamfer] [✓ ✕]`.
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
function ChamferDistance2Field({
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

/**
 * The [Move | Rotate] segment group on the armed placement cluster (WP-B W1).
 * The mode decides what the number to its right MEANS (mm along the axis vs
 * degrees about it), so it reads left-to-right as one sentence.
 */
function TransformModeSegments({
  active,
  onPick,
}: {
  active: TransformMode;
  onPick: (mode: TransformMode) => void;
}) {
  return (
    <div className="flex overflow-hidden rounded-full" role="group" aria-label="Placement mode">
      {TRANSFORM_MODES.map((m) => (
        <button
          key={m.mode}
          type="button"
          data-testid={m.testid}
          aria-pressed={m.mode === active}
          onClick={() => onPick(m.mode)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            m.mode === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          )}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}

/**
 * The [Copy] toggle on the armed placement cluster (WP-B W2): the visible
 * surface for `TransformBodyParams.copy`, which decides whether a placement MOVES
 * the bodies or leaves them behind. Alt at gizmo-grab writes the same FSM flag,
 * so this button is where the user can see (and undo) that choice.
 */
function CopyToggle({ copy, onToggle }: { copy: boolean; onToggle: (copy: boolean) => void }) {
  return (
    <button
      type="button"
      data-testid="chip-transform-copy"
      aria-pressed={copy}
      title="Keep the originals and place copies (Alt-drag)"
      onClick={() => onToggle(!copy)}
      className={cn(
        "rounded-full px-2 py-1 text-[11.5px] font-medium",
        copy ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
      )}
    >
      Copy
    </button>
  );
}

/**
 * The [Align] segment on the armed placement cluster (WP-B W2.5). Unlike every
 * other segment here it does not set a value — it hands the pointer a two-pick
 * face flow, so it reads as PRESSED for as long as that flow owns the pointer
 * and the label names the pick still outstanding. That is the only feedback the
 * chip can give: the picks themselves happen in the viewport.
 */
function AlignButton({ phase, onStart }: { phase: AlignPhase | null; onStart: () => void }) {
  const label = phase === "pickMoving" ? "Pick face" : phase === "pickDest" ? "Pick target" : "Align";
  return (
    <button
      type="button"
      data-testid="chip-transform-align"
      aria-pressed={phase !== null}
      title="Align a face flush onto a face of another body"
      onClick={onStart}
      className={cn(
        "rounded-full px-2 py-1 text-[11.5px] font-medium",
        phase !== null ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
      )}
    >
      {label}
    </button>
  );
}

export function ModelToolChips() {
  const engine = useViewportEngine();
  const kind = useToolChipStore((s) => s.kind);
  const value = useToolChipStore((s) => s.value);
  const count = useToolChipStore((s) => s.count);
  const axis = useToolChipStore((s) => s.axis);
  const plane = useToolChipStore((s) => s.plane);
  const op = useToolChipStore((s) => s.op);
  const symmetric = useToolChipStore((s) => s.symmetric);
  const showSymmetric = useToolChipStore((s) => s.showSymmetric);
  const booleanMode = useToolChipStore((s) => s.booleanMode);
  const canBoolean = useToolChipStore((s) => s.canBoolean);
  const showBooleanSegments = useToolChipStore((s) => s.showBooleanSegments);
  const edgeOp = useToolChipStore((s) => s.edgeOp);
  const transformMode = useToolChipStore((s) => s.transformMode);
  const copy = useToolChipStore((s) => s.copy);
  const alignPhase = useToolChipStore((s) => s.alignPhase);
  const showEdgeOpSegments = useToolChipStore((s) => s.showEdgeOpSegments);
  const distanceType = useToolChipStore((s) => s.distanceType);
  const distanceTypes = useToolChipStore((s) => s.distanceTypes);
  const chainTangentFaces = useToolChipStore((s) => s.chainTangentFaces);
  const valueError = useToolChipStore((s) => s.valueError);
  const distance2 = useToolChipStore((s) => s.distance2);
  const endCondition = useToolChipStore((s) => s.endCondition);
  const canUseBodyEnds = useToolChipStore((s) => s.canUseBodyEnds);
  const showEndConditions = useToolChipStore((s) => s.showEndConditions);
  const draftAngleDeg = useToolChipStore((s) => s.draftAngleDeg);
  const showDraft = useToolChipStore((s) => s.showDraft);
  const suffix = useToolChipStore((s) => s.suffix);
  const label = useToolChipStore((s) => s.label);
  const worldPos = useToolChipStore((s) => s.worldPos);
  /** Whether the armed owner wired a ✓ (fillet/shell: fresh arm yes, re-edit no). */
  const hasConfirm = useToolChipStore((s) => s.onConfirm !== null);
  // A plain DOM host, created once; the engine owns its DOM position.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = "model-tool-chip";
    return el;
  });

  const anchorKey = worldPos ? worldPos.join(",") : "";

  useEffect(() => {
    if (!engine || kind === "none" || !worldPos) return;
    engine.mountChip(CHIP_ID, host, worldPos);
    return () => engine.unmountChip(CHIP_ID, host);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, kind, anchorKey, host]);

  if (kind === "none" || !worldPos) return null;

  const numericChip = (suffix: string) => (
    <DimensionInput value={value} suffix={suffix} onCommit={(v) => toolChipStore.getState().onValue?.(v)} />
  );

  const panel = (children: React.ReactNode) => (
    <div className="pointer-events-auto inline-flex items-center gap-1 rounded-full border border-border bg-surface px-1.5 py-1 shadow-popover">
      {children}
    </div>
  );

  // The armed extrude / revolve cluster commits on chip-input Enter via `onConfirm`
  // (apply typed value THEN confirm — single fire; the input stops propagation so
  // the controller's capture-phase Enter never double-fires).
  const clusterInput = (unit: string) => (
    <DimensionInput
      value={value}
      suffix={unit}
      onCommit={(v) => toolChipStore.getState().onValue?.(v)}
      onConfirm={() => toolChipStore.getState().onConfirm?.()}
    />
  );
  const confirmButtons = (
    <ConfirmButtons
      onConfirm={() => toolChipStore.getState().onConfirm?.()}
      onCancel={() => toolChipStore.getState().onCancel?.()}
    />
  );
  const booleanSegments = showBooleanSegments ? (
    <BooleanModeSegments
      active={booleanMode}
      canBoolean={canBoolean}
      onPick={(m) => toolChipStore.getState().onBooleanMode?.(m)}
    />
  ) : null;

  // Only the FRESH edge-op arm sets this flag; the shell shares the value-chip
  // branch below (`shellThickness`) and the edge-op re-edit renders the bare
  // numeric chip, so neither can reach the segments.
  const edgeOpSegments = showEdgeOpSegments ? (
    <EdgeOpSegments active={edgeOp} onPick={(k) => toolChipStore.getState().onEdgeOp?.(k)} />
  ) : null;

  // The second chamfer distance rides with the segments and ONLY while the armed
  // op is a Chamfer — a Fillet has no second leg, and SCHEMA §7.3 forbids it from
  // carrying one, so showing the field there would offer an edit the wire refuses.
  // Keyed on the anchor so a fresh arm opens at `=` rather than inheriting the
  // previous arm's typed text.
  const chamferD2 =
    showEdgeOpSegments && edgeOp === "Chamfer" ? (
      <ChamferDistance2Field
        key={`d2-${anchorKey}`}
        value={distance2}
        onValue={(v) => toolChipStore.getState().onDistance2?.(v)}
        onConfirm={() => toolChipStore.getState().onConfirm?.()}
      />
    ) : null;

  const endConditionSegments = showEndConditions ? (
    <EndConditionSegments
      active={endCondition}
      canUseBodyEnds={canUseBodyEnds}
      onPick={(c) => toolChipStore.getState().onEndCondition?.(c)}
    />
  ) : null;

  // Keyed on the anchor so a fresh arm reopens collapsed (or seeded, on a
  // re-edit) instead of inheriting the previous arm's expanded state.
  const draftSegment = showDraft ? (
    <DraftSegment
      key={`draft-${anchorKey}`}
      deg={draftAngleDeg}
      onDeg={(d) => toolChipStore.getState().onDraftAngle?.(d)}
      onConfirm={() => toolChipStore.getState().onConfirm?.()}
    />
  ) : null;

  let content: React.ReactNode;
  if (kind === "extrudeDepth") {
    content = panel(
      <>
        {/* A distance is meaningless for the non-Blind end conditions — the
            kernel derives it — so the numeric input hides rather than showing a
            value that does not drive the result. */}
        {endCondition === "Blind" && clusterInput(LENGTH_SUFFIX)}
        {endConditionSegments}
        {draftSegment}
        {showSymmetric && endCondition === "Blind" && (
          <SymmetricToggle
            pressed={symmetric}
            onToggle={() => toolChipStore.getState().onSymmetric?.(!symmetric)}
          />
        )}
        {booleanSegments}
        {confirmButtons}
      </>,
    );
  } else if (kind === "revolveAngle") {
    content = panel(
      <>
        {clusterInput("°")}
        <button
          type="button"
          onClick={() => toolChipStore.getState().onResetAxis?.()}
          className="rounded-full bg-chip px-2 py-1 text-[11px] font-medium text-ink-3 hover:bg-hover-2"
        >
          Axis
        </button>
        {booleanSegments}
        {confirmButtons}
      </>,
    );
  } else if (kind === "datumOffset") {
    // Armed datum plane (DATUM W1): the picked base plane's GEOMETRIC name +
    // the offset, committed by ✓ or by Enter in the input (same armed-cluster
    // gesture the extrude/revolve chips use).
    content = panel(
      <>
        <span
          data-testid="chip-datum-base"
          className="px-2 py-1 text-[11.5px] font-medium text-ink-2"
        >
          {label}
        </span>
        {clusterInput(LENGTH_SUFFIX)}
        {confirmButtons}
      </>,
    );
  } else if (kind === "regionSelect") {
    // Multi-region select chip: `[ N regions ✓ ✕ ]` at the sketch centroid.
    content = panel(
      <>
        <span
          data-testid="chip-region-count"
          className="px-2 py-1 text-[11.5px] font-medium text-ink-2"
        >
          {count} region{count === 1 ? "" : "s"}
        </span>
        <ConfirmButtons
          onConfirm={() => toolChipStore.getState().onConfirm?.()}
          onCancel={() => toolChipStore.getState().onCancel?.()}
        />
      </>,
    );
  } else if (kind === "filletRadius" || kind === "shellThickness") {
    // Edge-op preview wave: a FRESH fillet/chamfer/shell arm wires ✓/✕ and gets
    // the same armed cluster the extrude/revolve chips use — release no longer
    // commits, so the visible confirm is the only way out other than Enter.
    //
    // The EDGE-OP re-edit wires ✓ too (W3): its type is editable there, and a pure
    // Fillet⇄Chamfer flip changes no number, so an input-only commit trigger could
    // never fire for one. The SHELL re-edit has no type to flip and keeps the bare
    // numeric chip it always was.
    content = hasConfirm
      ? panel(
          <>
            {clusterInput(LENGTH_SUFFIX)}
            {chamferD2}
            {edgeOpSegments}
            {confirmButtons}
          </>,
        )
      : numericChip(LENGTH_SUFFIX);
  } else if (kind === "offsetFace") {
    // SCHEMA §7.3 OffsetFace: `[distance] [type segments] [⌒] [✓ ✕]`.
    //
    // The panel takes a WARN border while `valueError` is set. The value itself is
    // never rewritten — a refused entry leaves the last valid number in the field
    // (SCHEMA §7.3 forbids clamping, and a clamped number would desynchronize the
    // stored param from the preview the user approved), so the border is the only
    // signal that the last thing typed did not take.
    content = (
      <div
        className={cn(
          "pointer-events-auto inline-flex items-center gap-1 rounded-full border bg-surface px-1.5 py-1 shadow-popover",
          valueError ? "border-warn-border" : "border-border",
        )}
      >
        {clusterInput(LENGTH_SUFFIX)}
        <DistanceTypeSegments
          active={distanceType}
          allowed={distanceTypes}
          onPick={(t) => toolChipStore.getState().onDistanceType?.(t)}
        />
        <TangentToggle
          pressed={chainTangentFaces}
          disabled={distanceType === "Total"}
          onToggle={() => toolChipStore.getState().onChainTangent?.(!chainTangentFaces)}
        />
        {confirmButtons}
      </div>
    );
  } else if (kind === "hole") {
    // WP-C T3. The cluster's own shape depends on `holeType`, so it lives in its
    // own component; only the shared ✓/✕ pair is added here.
    content = panel(
      <>
        <HoleChipCluster />
        {confirmButtons}
      </>,
    );
  } else if (kind === "sketchValue") {
    // WP-C T2b: an armed sketch EDIT tool's live parameter (fillet radius /
    // offset distance). It commits nothing — the geometry commits on a viewport
    // click — so the chip stays open across repeated applies and is keyed on the
    // ANCHOR alone: it mounts (and auto-focuses) once per arm, so the value can
    // be typed straight away without the field re-grabbing focus after each edit.
    content = panel(
      <>
        <span data-testid="chip-sketch-label" className="px-2 py-1 text-[11.5px] font-medium text-ink-2">
          {label}
        </span>
        <DimensionInput
          key={`sketchValue-${anchorKey}`}
          value={value}
          suffix={LENGTH_SUFFIX}
          autoFocus
          onCommit={(v) => toolChipStore.getState().onValue?.(v)}
        />
      </>,
    );
  } else if (kind === "dimension") {
    // Sketch Dimension tool: seeded + auto-focused; Enter commits, Esc cancels,
    // and a canvas click must NOT blur-commit (a 2nd line click upgrades a length
    // into an angle), so `commitOnBlur` is off. Keying by anchor+value remounts
    // (re-focuses) on each new pick — e.g. when a length upgrades to an angle —
    // but stays stable while typing (the value prop is unchanged mid-edit).
    content = (
      <DimensionInput
        key={`dim-${anchorKey}-${value}`}
        value={value}
        suffix={suffix}
        autoFocus
        commitOnBlur={false}
        onCommit={(v) => toolChipStore.getState().onValue?.(v)}
        onCancel={() => toolChipStore.getState().onCancel?.()}
      />
    );
  } else if (kind === "linearPattern") {
    content = panel(
      <>
        <SegmentToggle
          options={PATTERN_AXES}
          active={axis}
          label="Pattern axis"
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        <CountStepper count={count} onCount={(n) => toolChipStore.getState().onCount?.(n)} />
        {numericChip(LENGTH_SUFFIX)}
        <ApplyButton />
      </>,
    );
  } else if (kind === "circularPattern") {
    content = panel(
      <>
        <SegmentToggle
          options={PATTERN_AXES}
          active={axis}
          label="Pattern axis"
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        <CountStepper count={count} onCount={(n) => toolChipStore.getState().onCount?.(n)} />
        {numericChip("°")}
        <ApplyButton />
      </>,
    );
  } else if (kind === "transform") {
    // Armed placement (WP-B W1): mode · axis · the one component that pair
    // addresses · ✓/✕. Enter in the input commits, exactly like the extrude and
    // revolve clusters (apply the typed value THEN confirm, single fire).
    content = panel(
      <>
        <TransformModeSegments
          active={transformMode}
          onPick={(m) => toolChipStore.getState().onTransformMode?.(m)}
        />
        <SegmentToggle
          options={PATTERN_AXES}
          active={axis}
          label="Placement axis"
          testid={(a) => `chip-axis-${a.toLowerCase()}`}
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        {clusterInput(transformMode === "rotate" ? "°" : LENGTH_SUFFIX)}
        <CopyToggle copy={copy} onToggle={(c) => toolChipStore.getState().onCopy?.(c)} />
        <AlignButton phase={alignPhase} onStart={() => toolChipStore.getState().onAlign?.()} />
        {confirmButtons}
      </>,
    );
  } else if (kind === "mirror") {
    content = panel(
      <>
        <SegmentToggle
          options={MIRROR_PLANES}
          active={plane}
          label="Mirror plane"
          onPick={(p) => toolChipStore.getState().onPlane?.(p)}
        />
        <ApplyButton />
      </>,
    );
  } else {
    // booleanOp
    content = panel(
      <>
        <div className="flex overflow-hidden rounded-full">
          {BOOLEAN_OPS.map((o) => (
            <button
              key={o}
              type="button"
              onClick={() => toolChipStore.getState().onOp?.(o)}
              className={cn(
                "px-2 py-1 text-[11.5px] font-medium",
                o === op ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
              )}
            >
              {o}
            </button>
          ))}
        </div>
        <ApplyButton />
      </>,
    );
  }

  return createPortal(content, host);
}
