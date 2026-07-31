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
import { useToolChipStore, toolChipStore } from "@/stores/toolChipStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import type { BooleanOperation } from "@/ipc/types";
import type {
  PatternAxis,
  MirrorPlane,
  BooleanMode,
  ExtrudeEndCondition,
} from "@/tools/modelTools/modelToolMachine";

const CHIP_ID = "__model_tool_chip";
const BOOLEAN_OPS: BooleanOperation[] = ["Union", "Cut", "Intersect"];
const PATTERN_AXES: PatternAxis[] = ["X", "Y", "Z"];
const MIRROR_PLANES: MirrorPlane[] = ["XY", "XZ", "YZ"];

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
      className="flex overflow-hidden rounded-sm"
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

/** A segmented toggle row (axis / plane pickers), styled like the boolean op row. */
function SegmentToggle<T extends string>({
  options,
  active,
  onPick,
  label,
}: {
  options: readonly T[];
  active: T;
  onPick: (v: T) => void;
  label: string;
}) {
  return (
    <div className="flex overflow-hidden rounded-sm" role="group" aria-label={label}>
      {options.map((o) => (
        <button
          key={o}
          type="button"
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
        className="flex h-5 w-5 items-center justify-center rounded-sm bg-chip text-ink-3 hover:bg-hover-2"
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
        className="flex h-5 w-5 items-center justify-center rounded-sm bg-chip text-ink-3 hover:bg-hover-2"
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
      className="rounded-sm bg-accent px-2 py-1 text-[11.5px] font-medium text-white hover:opacity-90"
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
        className="rounded-sm bg-accent px-2 py-1 text-[11.5px] font-medium text-white hover:opacity-90"
      >
        ✓
      </button>
      <button
        type="button"
        data-testid="chip-cancel"
        aria-label="Cancel"
        onClick={() => onCancel?.()}
        className="rounded-sm bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
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
        "rounded-sm px-2 py-1 text-[11.5px] font-medium",
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
      className="flex overflow-hidden rounded-sm"
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
  const endCondition = useToolChipStore((s) => s.endCondition);
  const canUseBodyEnds = useToolChipStore((s) => s.canUseBodyEnds);
  const showEndConditions = useToolChipStore((s) => s.showEndConditions);
  const suffix = useToolChipStore((s) => s.suffix);
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
    <div className="pointer-events-auto inline-flex items-center gap-1 rounded-md border border-border bg-white p-1 shadow-panel">
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

  const endConditionSegments = showEndConditions ? (
    <EndConditionSegments
      active={endCondition}
      canUseBodyEnds={canUseBodyEnds}
      onPick={(c) => toolChipStore.getState().onEndCondition?.(c)}
    />
  ) : null;

  let content: React.ReactNode;
  if (kind === "extrudeDepth") {
    content = panel(
      <>
        {/* A distance is meaningless for the non-Blind end conditions — the
            kernel derives it — so the numeric input hides rather than showing a
            value that does not drive the result. */}
        {endCondition === "Blind" && clusterInput("mm")}
        {endConditionSegments}
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
          className="rounded-sm bg-chip px-2 py-1 text-[11px] font-medium text-ink-3 hover:bg-hover-2"
        >
          Axis
        </button>
        {booleanSegments}
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
    // commits, so the visible confirm is the only way out other than Enter. The
    // parametric RE-EDIT chip wires no ✓ (it commits from its own input) and
    // keeps rendering as the bare numeric chip it always was.
    content = hasConfirm
      ? panel(
          <>
            {clusterInput("mm")}
            {confirmButtons}
          </>,
        )
      : numericChip("mm");
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
        {numericChip("mm")}
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
        <div className="flex overflow-hidden rounded-sm">
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
