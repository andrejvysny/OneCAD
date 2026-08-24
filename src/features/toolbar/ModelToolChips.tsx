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
import { GearChipCluster } from "./GearChipCluster";
import { ExtrudeOverflow } from "./ExtrudeChipControls";
import { EdgeOpOverflow } from "./EdgeOpChipControls";
import { RevolveOverflow } from "./RevolveChipControls";
import {
  acceptCount,
  PATTERN_COUNT_MAX,
  PATTERN_COUNT_MIN,
  PATTERN_STEPPER_MAX,
} from "@/tools/modelTools/modelToolMachine";
import { useToolChipStore, toolChipStore, MODEL_TOOL_CHIP_ID } from "@/stores/toolChipStore";
import { LENGTH_SUFFIX } from "@/units/format";
import { useViewportEngine } from "@/viewport/engineBridge";
import type { BooleanOperation, OffsetDistanceType } from "@/ipc/types";
import type {
  AlignPhase,
  PatternAxis,
  MirrorPlane,
  TransformMode,
} from "@/tools/modelTools/modelToolMachine";

const CHIP_ID = MODEL_TOOL_CHIP_ID;
const BOOLEAN_OPS: BooleanOperation[] = ["Union", "Cut", "Intersect"];
const PATTERN_AXES: PatternAxis[] = ["X", "Y", "Z"];
const MIRROR_PLANES: MirrorPlane[] = ["XY", "XZ", "YZ"];

/** The armed-placement mode segments (WP-B W1). */
const TRANSFORM_MODES: { mode: TransformMode; label: string; testid: string }[] = [
  { mode: "move", label: "Move", testid: "chip-transform-move" },
  { mode: "rotate", label: "Rotate", testid: "chip-transform-rotate" },
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
/**
 * `Total` — the instance count, INCLUDING the source (U6).
 *
 * The label matters: "count" left it ambiguous whether 3 meant three instances
 * or three COPIES, and the two differ by exactly the body the user is looking
 * at. Pattern V2 keeps the source as instance zero, so `Total 3` is the source
 * plus two children — which is what the result summary then states.
 *
 * Range: the buttons step 2–12 (the common case, one click per instance) while
 * TYPING reaches the worker's 128. An out-of-range entry is refused, not
 * clamped, and the field says what the range is instead of silently disagreeing
 * with the preview.
 */
function CountStepper({ count, onCount }: { count: number; onCount: (n: number) => void }) {
  const [text, setText] = useState(String(count));
  const [rejected, setRejected] = useState(false);
  useEffect(() => {
    setText(String(count));
    setRejected(false);
  }, [count]);

  const submit = (raw: string): void => {
    const n = Number.parseInt(raw.trim(), 10);
    if (acceptCount(n) === null) {
      setRejected(true);
      return;
    }
    setRejected(false);
    onCount(n);
  };

  return (
    <div className="inline-flex items-center gap-0.5">
      <button
        type="button"
        aria-label="Fewer instances"
        disabled={count <= PATTERN_COUNT_MIN}
        onClick={() => onCount(count - 1)}
        className="flex h-5 w-5 items-center justify-center rounded-full bg-chip text-ink-3 hover:bg-hover-2 disabled:opacity-40"
      >
        −
      </button>
      <span className="text-[11.5px] text-ink-5">Total</span>
      <input
        data-testid="pattern-count"
        aria-label="Total instances"
        aria-invalid={rejected}
        title={`Total instances, including the source (${PATTERN_COUNT_MIN}–${PATTERN_COUNT_MAX})`}
        className={cn(
          "w-8 bg-transparent text-center font-mono text-[11.5px] outline-none",
          rejected ? "text-traffic-close" : "text-ink-2",
        )}
        value={text}
        inputMode="numeric"
        onChange={(e) => {
          setText(e.target.value);
          submit(e.target.value);
        }}
        onBlur={() => setText(String(count))}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit(text);
          e.stopPropagation();
        }}
      />
      <button
        type="button"
        aria-label="More instances"
        disabled={count >= PATTERN_STEPPER_MAX}
        onClick={() => onCount(count + 1)}
        className="flex h-5 w-5 items-center justify-center rounded-full bg-chip text-ink-3 hover:bg-hover-2 disabled:opacity-40"
      >
        +
      </button>
    </div>
  );
}

/** The ✕ half shared by every chip that offers an explicit cancel. */
function CancelButton({ onCancel }: { onCancel?: () => void }) {
  return (
    <button
      type="button"
      data-testid="chip-cancel"
      aria-label="Cancel"
      onClick={() => onCancel?.()}
      className="rounded-full bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
    >
      ✕
    </button>
  );
}

/**
 * The shared ✓/✕ commit/cancel pair — the ONE confirmation vocabulary for every
 * model tool (U2).
 *
 * Boolean, both patterns and mirror used to render an accent `Apply` text button
 * wired to a separate `onApply` callback, which is also why they had no Enter
 * path: two vocabularies meant two protocols. The body-lifecycle meaning that
 * button carried belongs in the result summary, where `3 total · 2 new bodies ·
 * source retained` says more than the word "Apply" ever did.
 */
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
      <CancelButton onCancel={onCancel} />
    </>
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
 * The [Fuse] toggle on the armed MIRROR cluster (WP6): the visible surface for
 * `MirrorBodyParams.fuseWithOriginal`, which decides whether the mirrored copy
 * lands as its own body or is folded back into the source.
 *
 * OFF for a fresh mirror, matching the record's own `#[serde(default)] bool` —
 * the flag was previously hard-coded there with no way to author it, so a fused
 * mirror could only be reached by re-editing a record some other lane wrote.
 */
function FuseToggle({ fuse, onToggle }: { fuse: boolean; onToggle: (fuse: boolean) => void }) {
  return (
    <button
      type="button"
      data-testid="chip-mirror-fuse"
      aria-label="Fuse with original"
      aria-pressed={fuse}
      title="Fold the mirrored copy back into the source body"
      onClick={() => onToggle(!fuse)}
      className={cn(
        "rounded-full px-2 py-1 text-[11.5px] font-medium",
        fuse ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
      )}
    >
      Fuse
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
  const fuse = useToolChipStore((s) => s.fuse);
  const alignPhase = useToolChipStore((s) => s.alignPhase);
  const showEdgeOpSegments = useToolChipStore((s) => s.showEdgeOpSegments);
  const distanceType = useToolChipStore((s) => s.distanceType);
  const distanceTypes = useToolChipStore((s) => s.distanceTypes);
  const chainTangentFaces = useToolChipStore((s) => s.chainTangentFaces);
  const valueError = useToolChipStore((s) => s.valueError);
  const distance2 = useToolChipStore((s) => s.distance2);
  const chamferAngleDeg = useToolChipStore((s) => s.chamferAngleDeg);
  const endCondition = useToolChipStore((s) => s.endCondition);
  const canUseBodyEnds = useToolChipStore((s) => s.canUseBodyEnds);
  const showEndConditions = useToolChipStore((s) => s.showEndConditions);
  const draftAngleDeg = useToolChipStore((s) => s.draftAngleDeg);
  const showDraft = useToolChipStore((s) => s.showDraft);
  const suffix = useToolChipStore((s) => s.suffix);
  const label = useToolChipStore((s) => s.label);
  const worldPos = useToolChipStore((s) => s.worldPos);
  const anchorAxisFrom = useToolChipStore((s) => s.anchorAxisFrom);
  const anchorOffsetPx = useToolChipStore((s) => s.anchorOffsetPx);
  /** Type-to-enter arm (U3): remounting on `token` is what focuses the field, and
   *  `seed` is the character that replaces the formatted value. */
  const primaryEntry = useToolChipStore((s) => s.primaryEntry);
  const targetName = useToolChipStore((s) => s.targetName);
  const toolName = useToolChipStore((s) => s.toolName);
  const onSwap = useToolChipStore((s) => s.onSwap);
  const resultSummary = useToolChipStore((s) => s.resultSummary);
  // A plain DOM host, created once; the engine owns its DOM position.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = "model-tool-chip";
    return el;
  });

  const anchorKey = worldPos ? worldPos.join(",") : "";

  useEffect(() => {
    if (!engine || kind === "none" || !worldPos) return;
    // `anchorKey` is the ARM's anchor, not a live one: a chip whose anchor tracks
    // the gesture (extrude) is moved by the controller through `engine.moveChip`,
    // never by a store write — this effect UNMOUNTS on every change, and a chip
    // detached mid-drag loses input focus and fires `commitOnBlur` on half-typed
    // text. See the header of `toolChipStore`.
    engine.mountChip(CHIP_ID, host, worldPos, {
      axisFrom: anchorAxisFrom ?? undefined,
      offsetPx: anchorOffsetPx || undefined,
      // The chip and the value arrow share this anchor, so without this the chip
      // sits ON the arrow and every press meant for the arrow hits the chip
      // instead (measured: the arrow's grab pixel resolved to `chip-cancel`).
      avoidValueHandle: true,
    });
    return () => engine.unmountChip(CHIP_ID, host);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, kind, anchorKey, host]);

  if (kind === "none" || !worldPos) return null;

  /*
   * Every armed model-tool numeric field, in one place (U3).
   *
   *   - `onPreview` fires on every parseable keystroke, so the viewport follows
   *     the typing. It routes to `onValue`, the same channel a drag uses, so the
   *     controller's existing coalescing/fencing applies unchanged.
   *   - `commitOnBlur` is OFF: an armed model tool commits on Enter or ✓ only.
   *     Nothing is lost, because the value already went out through `onPreview`.
   *   - `primaryEntry` seeds + focuses the field when the user typed on canvas.
   */
  const primaryField = (suffix: string, opts?: { onConfirm?: boolean }) => (
    <DimensionInput
      key={primaryEntry ? `primary-${primaryEntry.token}` : `primary-${anchorKey}`}
      value={value}
      suffix={suffix}
      initialText={primaryEntry?.seed}
      autoFocus={primaryEntry !== null}
      commitOnBlur={false}
      onPreview={(v) => toolChipStore.getState().onValue?.(v)}
      onCommit={(v) => toolChipStore.getState().onValue?.(v)}
      onConfirm={opts?.onConfirm ? () => toolChipStore.getState().onConfirm?.() : undefined}
    />
  );

  const numericChip = (suffix: string) => primaryField(suffix);

  /*
   * The OperationHUD frame (U4) — ONE wrapper for every armed model tool.
   *
   * Each branch below still owns its operation-specific content, but the frame
   * around it is shared, so a tool cannot quietly acquire its own tone, its own
   * validity treatment, or no accessible status at all:
   *
   *   - the border takes the WARN tone while `valueError` is set, which used to
   *     be hand-rolled in the offsetFace branch and absent everywhere else;
   *   - `resultSummary` renders in a common slot ABOVE the controls, so a
   *     body-lifecycle operation states what it will produce BEFORE Apply — the
   *     audit's "3 total instances · 2 new bodies · source retained" (D18);
   *   - that summary is also the tool's `aria-live` status. It lives here rather
   *     than in the canvas subtree, which is `aria-hidden` decoration.
   */
  const panel = (children: React.ReactNode) => (
    <div
      data-testid="operation-hud"
      className={cn(
        "pointer-events-auto inline-flex flex-col items-stretch gap-1 rounded-2xl border bg-surface px-1.5 py-1 shadow-popover",
        valueError ? "border-warn-border" : "border-border",
      )}
    >
      {resultSummary && (
        <div
          data-testid="chip-result-summary"
          role="status"
          aria-live="polite"
          className="px-1.5 pt-0.5 text-[11px] text-ink-5"
        >
          {resultSummary}
        </div>
      )}
      <div className="inline-flex items-center gap-1">{children}</div>
    </div>
  );

  // The armed extrude / revolve cluster commits on chip-input Enter via `onConfirm`
  // (apply typed value THEN confirm — single fire; the input stops propagation so
  // the controller's capture-phase Enter never double-fires).
  const clusterInput = (unit: string) => primaryField(unit, { onConfirm: true });
  const confirmButtons = (
    <ConfirmButtons
      onConfirm={() => toolChipStore.getState().onConfirm?.()}
      onCancel={() => toolChipStore.getState().onCancel?.()}
    />
  );
  // The boolean segments now live behind revolve's own `⋯` (UNIFY-UX Phase 2) —
  // keyed on the anchor so a fresh axis pick reopens collapsed.
  const revolveOverflow = showBooleanSegments ? (
    <RevolveOverflow
      key={`overflow-${anchorKey}`}
      booleanMode={booleanMode}
      canBoolean={canBoolean}
      onBooleanMode={(m) => toolChipStore.getState().onBooleanMode?.(m)}
    />
  ) : null;

  // Only the FRESH edge-op arm sets this flag; the shell shares the value-chip
  // branch below (`shellThickness`) and the edge-op re-edit renders the bare
  // numeric chip, so neither can reach the overflow. [Fillet|Chamfer] + the
  // chamfer second leg both live behind it (UNIFY-UX Phase 1) — keyed on the
  // anchor so a fresh arm reopens collapsed instead of inheriting the previous
  // arm's expanded state, mirroring extrude's `⋯`.
  const edgeOpOverflow = showEdgeOpSegments ? (
    <EdgeOpOverflow
      key={`overflow-${anchorKey}`}
      edgeOp={edgeOp}
      onEdgeOp={(k) => toolChipStore.getState().onEdgeOp?.(k)}
      distance2={distance2}
      onDistance2={(v) => toolChipStore.getState().onDistance2?.(v)}
      chamferAngleDeg={chamferAngleDeg}
      onChamferAngle={(v) => toolChipStore.getState().onChamferAngle?.(v)}
      onConfirm={() => toolChipStore.getState().onConfirm?.()}
    />
  ) : null;

  let content: React.ReactNode;
  if (kind === "extrudeDepth") {
    // THE DIMENSION AND NOTHING ELSE. The arrow states the direction, the tint
    // states Add vs Cut, and everything that is neither goes behind `⋯` — keyed
    // on the anchor so a fresh arm reopens collapsed instead of inheriting the
    // previous arm's expanded state.
    content = panel(
      <>
        {/* A distance is meaningless for the non-Blind end conditions — the
            kernel derives it — so the numeric input hides rather than showing a
            value that does not drive the result. */}
        {endCondition === "Blind" && clusterInput(LENGTH_SUFFIX)}
        <ExtrudeOverflow
          key={`overflow-${anchorKey}`}
          endCondition={endCondition}
          canUseBodyEnds={canUseBodyEnds}
          showEndConditions={showEndConditions}
          onEndCondition={(c) => toolChipStore.getState().onEndCondition?.(c)}
          draftAngleDeg={draftAngleDeg}
          showDraft={showDraft}
          onDraftAngle={(d) => toolChipStore.getState().onDraftAngle?.(d)}
          symmetric={symmetric}
          showSymmetric={showSymmetric}
          onSymmetric={(sym) => toolChipStore.getState().onSymmetric?.(sym)}
          booleanMode={booleanMode}
          canBoolean={canBoolean}
          showBooleanSegments={showBooleanSegments}
          onBooleanMode={(m) => toolChipStore.getState().onBooleanMode?.(m)}
          onConfirm={() => toolChipStore.getState().onConfirm?.()}
        />
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
        {revolveOverflow}
        {confirmButtons}
      </>,
    );
  } else if (kind === "revolveAxisPick") {
    // No value armed yet — a text-only instructional chip (UNIFY-UX Phase 2),
    // mirroring the regionSelect chip's shape minus the confirm half: there is
    // nothing here for ✓ to commit.
    content = panel(
      <>
        <span
          data-testid="chip-revolve-axis-hint"
          className="px-2 py-1 text-[11.5px] font-medium text-ink-2"
        >
          Pick an axis line
        </span>
        <button
          type="button"
          data-testid="chip-cancel"
          aria-label="Cancel"
          onClick={() => toolChipStore.getState().onCancel?.()}
          className="rounded-full bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
        >
          ✕
        </button>
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
    // Fillet / chamfer / shell, fresh arm AND re-edit: the same armed cluster the
    // extrude/revolve chips use. Release no longer commits, so the visible ✓ (or
    // Enter, which calls the same `onConfirm`) is the only way out.
    //
    // U2 deleted the bare-numeric fallback this branch used to fall back to when
    // no ✓ was wired: `armShell` and both `showFillet` sites all wire ✓/✕, so it
    // was already unreachable — and an armed operation that renders NO cancel is
    // exactly what the interaction contract forbids.
    content = panel(
      <>
        {clusterInput(LENGTH_SUFFIX)}
        {edgeOpOverflow}
        {confirmButtons}
      </>,
    );
  } else if (kind === "offsetFace") {
    // SCHEMA §7.3 OffsetFace: `[distance] [type segments] [⌒] [✓ ✕]`.
    //
    // The WARN border while `valueError` is set now comes from the shared HUD
    // frame (U4) rather than this branch's own copy of it — the value itself is
    // still never rewritten, so a refused entry leaves the last valid number in
    // the field (SCHEMA §7.3 forbids clamping, and a clamped number would
    // desynchronize the stored param from the preview the user approved).
    content = panel(
      <>
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
      </>,
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
  } else if (kind === "gear") {
    // Gear Generator G1-h. Same shape as the hole branch above.
    content = panel(
      <>
        <GearChipCluster />
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
          testid={(a) => `chip-pattern-axis-${a.toLowerCase()}`}
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        <CountStepper count={count} onCount={(n) => toolChipStore.getState().onCount?.(n)} />
        {numericChip(LENGTH_SUFFIX)}
        {confirmButtons}
      </>,
    );
  } else if (kind === "circularPattern") {
    content = panel(
      <>
        <SegmentToggle
          options={PATTERN_AXES}
          active={axis}
          label="Pattern axis"
          testid={(a) => `chip-pattern-axis-${a.toLowerCase()}`}
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        <CountStepper count={count} onCount={(n) => toolChipStore.getState().onCount?.(n)} />
        {numericChip("°")}
        {confirmButtons}
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
          testid={(p) => `chip-mirror-plane-${p.toLowerCase()}`}
          onPick={(p) => toolChipStore.getState().onPlane?.(p)}
        />
        <FuseToggle fuse={fuse} onToggle={(f) => toolChipStore.getState().onFuse?.(f)} />
        {confirmButtons}
      </>,
    );
  } else {
    // booleanOp
    content = panel(
      <>
        {/*
          * ROLE BADGES (U6). A Boolean is not symmetric — which body survives is
          * the whole decision — so the chip states both operands by NAME before
          * the user commits. Named, not colour-coded: a colour-only cue is
          * unreadable to a third of users and unreadable against a body that
          * happens to be the same colour.
          */}
        {(targetName || toolName) && (
          <span className="inline-flex items-center gap-1 px-1 text-[11px]">
            <span className="text-ink-5">Target</span>
            <span data-testid="chip-bool-target" className="font-medium text-ink-2">
              {targetName}
            </span>
            <span className="text-ink-5">Tool</span>
            <span data-testid="chip-bool-tool" className="font-medium text-ink-2">
              {toolName}
            </span>
          </span>
        )}
        {onSwap && (
          <button
            type="button"
            data-testid="chip-bool-swap"
            aria-label="Swap target and tool"
            title="Swap target and tool"
            onClick={() => toolChipStore.getState().onSwap?.()}
            className="rounded-full bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
          >
            ⇄
          </button>
        )}
        <div className="flex overflow-hidden rounded-full">
          {BOOLEAN_OPS.map((o) => (
            <button
              key={o}
              type="button"
              data-testid={`chip-boolean-${o.toLowerCase()}`}
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
        {confirmButtons}
      </>,
    );
  }

  return createPortal(content, host);
}
