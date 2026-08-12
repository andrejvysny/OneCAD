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
import { ExtrudeOverflow } from "./ExtrudeChipControls";
import { EdgeOpOverflow } from "./EdgeOpChipControls";
import { RevolveOverflow } from "./RevolveChipControls";
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
      data-testid="chip-apply"
      onClick={() => toolChipStore.getState().onApply?.()}
      className="rounded-full bg-accent px-2 py-1 text-[11.5px] font-medium text-on-accent hover:opacity-90"
    >
      Apply
    </button>
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
  const anchorAxisFrom = useToolChipStore((s) => s.anchorAxisFrom);
  const anchorOffsetPx = useToolChipStore((s) => s.anchorOffsetPx);
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
    // `anchorKey` is the ARM's anchor, not a live one: a chip whose anchor tracks
    // the gesture (extrude) is moved by the controller through `engine.moveChip`,
    // never by a store write — this effect UNMOUNTS on every change, and a chip
    // detached mid-drag loses input focus and fires `commitOnBlur` on half-typed
    // text. See the header of `toolChipStore`.
    engine.mountChip(CHIP_ID, host, worldPos, {
      axisFrom: anchorAxisFrom ?? undefined,
      offsetPx: anchorOffsetPx || undefined,
    });
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
            {edgeOpOverflow}
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
          testid={(a) => `chip-pattern-axis-${a.toLowerCase()}`}
          onPick={(a) => toolChipStore.getState().onAxis?.(a)}
        />
        <CountStepper count={count} onCount={(n) => toolChipStore.getState().onCount?.(n)} />
        {numericChip(LENGTH_SUFFIX)}
        <ApplyButton />
        <CancelButton onCancel={() => toolChipStore.getState().onCancel?.()} />
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
        <ApplyButton />
        <CancelButton onCancel={() => toolChipStore.getState().onCancel?.()} />
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
        <ApplyButton />
        <CancelButton onCancel={() => toolChipStore.getState().onCancel?.()} />
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
        <ApplyButton />
        <CancelButton onCancel={() => toolChipStore.getState().onCancel?.()} />
      </>,
    );
  }

  return createPortal(content, host);
}
