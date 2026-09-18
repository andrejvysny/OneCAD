/*
 * Neutral secondary controls for an armed model operation (spec §4.3, S09).
 *
 * These used to live in `ModelToolChips`, the floating label's module, so the
 * inspector — the surface the spec puts secondary parameters on — imported the
 * implementation of the label to reach them. They are shared toolbar controls
 * with no opinion about WHERE they render: the inspector hosts every one of them
 * today, and the operation strip may host a subset tomorrow.
 *
 * None of them owns parameter state. Each reads the armed value it renders and
 * dispatches through the `toolChipStore` callback the controller registered.
 */
import { useEffect, useState } from "react";
import { cn } from "@/ui/cn";
import {
  acceptCount,
  PATTERN_COUNT_MAX,
  PATTERN_COUNT_MIN,
  PATTERN_STEPPER_MAX,
} from "@/tools/modelTools/modelToolMachine";
import { toolChipStore, useToolChipStore } from "@/stores/toolChipStore";
import type { BooleanOperation, OffsetDistanceType } from "@/ipc/types";
import type {
  AlignPhase,
  PatternAxis,
  MirrorPlane,
  TransformMode,
} from "@/tools/modelTools/modelToolMachine";

export const BOOLEAN_OPS: BooleanOperation[] = ["Union", "Cut", "Intersect"];
export const PATTERN_AXES: PatternAxis[] = ["X", "Y", "Z"];
export const MIRROR_PLANES: MirrorPlane[] = ["XY", "XZ", "YZ"];

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
export function SegmentToggle<T extends string>({
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
   * Per-segment `data-testid`. Playwright's e2e lane keeps using it as the
   * stable handle regardless of accessible name (WP-U10 made the chip subtree
   * role/name-reachable in the DOM tree, but a testid is still cheaper and more
   * resilient for e2e than chaining a group name into a button name). Optional
   * because the pattern/mirror chips predate that need and have no spec
   * depending on them.
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
export function CountStepper({
  count,
  onCount,
  resetKey,
}: {
  count: number;
  onCount: (n: number) => void;
  /** Changes only when a fresh armed pattern supplies a different handler. */
  resetKey: unknown;
}) {
  const [text, setText] = useState(String(count));
  const [rejected, setRejected] = useState(false);
  useEffect(() => {
    setText(String(count));
    setRejected(false);
    toolChipStore.getState().setRawValueValidity("pattern-count", true, String(count));
  }, [count, resetKey]);

  const applyCount = (next: number, nextText: string): void => {
    setText(nextText);
    setRejected(false);
    toolChipStore.getState().setRawValueValidity("pattern-count", true, nextText);
    onCount(next);
  };

  const submit = (raw: string): void => {
    if (!/^\d+$/.test(raw.trim())) {
      setRejected(true);
      toolChipStore.getState().setRawValueValidity("pattern-count", false, raw);
      return;
    }
    const n = Number(raw.trim());
    if (acceptCount(n) === null) {
      setRejected(true);
      toolChipStore.getState().setRawValueValidity("pattern-count", false, raw);
      return;
    }
    applyCount(n, raw);
  };

  return (
    <div className="inline-flex items-center gap-0.5">
      <button
        type="button"
        aria-label="Fewer instances"
        disabled={count <= PATTERN_COUNT_MIN}
        onClick={() => applyCount(count - 1, String(count - 1))}
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
        onBlur={() => undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit(text);
          e.stopPropagation();
        }}
      />
      <button
        type="button"
        aria-label="More instances"
        disabled={count >= PATTERN_STEPPER_MAX}
        onClick={() => applyCount(count + 1, String(count + 1))}
        className="flex h-5 w-5 items-center justify-center rounded-full bg-chip text-ink-3 hover:bg-hover-2 disabled:opacity-40"
      >
        +
      </button>
    </div>
  );
}

/**
 * The `[Offset | Total | Radius | Diameter]` segment group on the armed
 * offset-face operation (SCHEMA §7.3).
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
 * The tangent-chain toggle on the armed offset-face operation. ON by default: the
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

/** Inspector-only Offset Face secondaries; the primary distance stays on the label. */
export function OffsetFaceInspectorControls() {
  const distanceType = useToolChipStore((s) => s.distanceType);
  const distanceTypes = useToolChipStore((s) => s.distanceTypes);
  const chainTangentFaces = useToolChipStore((s) => s.chainTangentFaces);
  return (
    <>
      <DistanceTypeSegments
        active={distanceType}
        allowed={distanceTypes}
        onPick={(type) => toolChipStore.getState().onDistanceType?.(type)}
      />
      <TangentToggle
        pressed={chainTangentFaces}
        disabled={distanceType === "Total"}
        onToggle={() => toolChipStore.getState().onChainTangent?.(!chainTangentFaces)}
      />
    </>
  );
}

/**
 * The [Move | Rotate] segment group on the armed placement operation (WP-B W1).
 * The mode decides what the number MEANS (mm along the axis vs degrees about
 * it), so it reads left-to-right as one sentence.
 */
export function TransformModeSegments({
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
 * The [Copy] toggle on the armed placement operation (WP-B W2): the visible
 * surface for `TransformBodyParams.copy`, which decides whether a placement MOVES
 * the bodies or leaves them behind. Alt at gizmo-grab writes the same FSM flag,
 * so this button is where the user can see (and undo) that choice.
 */
export function CopyToggle({ copy, onToggle }: { copy: boolean; onToggle: (copy: boolean) => void }) {
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
 * The [Fuse] toggle on the armed MIRROR operation (WP6): the visible surface for
 * `MirrorBodyParams.fuseWithOriginal`, which decides whether the mirrored copy
 * lands as its own body or is folded back into the source.
 *
 * OFF for a fresh mirror, matching the record's own `#[serde(default)] bool` —
 * the flag was previously hard-coded there with no way to author it, so a fused
 * mirror could only be reached by re-editing a record some other lane wrote.
 */
export function FuseToggle({ fuse, onToggle }: { fuse: boolean; onToggle: (fuse: boolean) => void }) {
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
 * The [Align] segment on the armed placement operation (WP-B W2.5). Unlike every
 * other segment here it does not set a value — it hands the pointer a two-pick
 * face flow, so it reads as PRESSED for as long as that flow owns the pointer
 * and the label names the pick still outstanding. That is the only feedback it
 * can give: the picks themselves happen in the viewport.
 */
export function AlignButton({ phase, onStart }: { phase: AlignPhase | null; onStart: () => void }) {
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
