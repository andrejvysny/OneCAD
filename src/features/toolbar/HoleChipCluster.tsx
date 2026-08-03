/**
 * The armed HOLE cluster (WP-C T3):
 *
 *   [Simple|CBore|CSink] [Ø d] [depth|Thru] (+ the ACTIVE conditional pair) [Std ▾]
 *
 * Its own file rather than another branch inside `ModelToolChips.tsx` (already
 * ~900 lines): the hole is the first tool whose cluster changes SHAPE with a
 * param, so it carries two conditional sub-clusters and a popover of its own.
 * `ModelToolChips` renders this plus the shared `✓ ✕` pair inside the usual panel.
 *
 * CONDITIONAL FIELDS. Only the active profile's pair is rendered — the OTHER
 * profile's numbers stay live in the FSM (so a flip back hands them straight
 * back) but they are never shown and never emitted. A field that is not on screen
 * is not part of the record; that is the whole conditional-block contract
 * (SCHEMA §7.3).
 *
 * The chips live under the viewport's `aria-hidden` overlay root, so role/name
 * queries cannot reach them — every control carries a `data-testid`, which is the
 * only handle e2e has (the `SegmentToggle` convention).
 */

import { useEffect, useState } from "react";
import { cn } from "@/ui/cn";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { useSettingsStore } from "@/stores/settingsStore";
import { useToolChipStore, toolChipStore } from "@/stores/toolChipStore";
import { LENGTH_SUFFIX, formatLength, parseLength } from "@/units/format";
import { HOLE_CS_ANGLES } from "@/tools/modelTools/holeMachine";
import { HOLE_STANDARDS, type HoleFit } from "@/tools/modelTools/holeStandards";
import type { HoleType } from "@/ipc/types";

/** The three hole profiles, in the order a machinist reads them. */
const HOLE_TYPES: { holeType: HoleType; label: string; testid: string }[] = [
  { holeType: "simple", label: "Simple", testid: "chip-hole-simple" },
  { holeType: "counterbore", label: "CBore", testid: "chip-hole-counterbore" },
  { holeType: "countersink", label: "CSink", testid: "chip-hole-countersink" },
];

/**
 * What the depth field shows for a THROUGH-ALL hole. A glyph, not a blank: an
 * empty field would read as "unset", while `Thru` states the end condition
 * SCHEMA §7.3 gives a `null` depth. Typing it back (or clearing the field) is how
 * the user returns to through-all from a blind depth.
 */
const THROUGH_TEXT = "Thru";

/** The two ISO 273 clearance series the standards popover offers. */
const FITS: { fit: HoleFit; label: string }[] = [
  { fit: "close", label: "Close" },
  { fit: "normal", label: "Normal" },
];

function Segments<T extends string>({
  options,
  active,
  onPick,
  label,
}: {
  options: readonly { value: T; label: string; testid: string }[];
  active: T;
  onPick: (v: T) => void;
  label: string;
}) {
  return (
    <div className="flex overflow-hidden rounded-sm" role="group" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          data-testid={o.testid}
          aria-pressed={o.value === active}
          onClick={() => onPick(o.value)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            o.value === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * A length field whose domain includes `null` (through-all). Modelled on
 * `ChamferDistance2Field`, and deliberately NOT a `DimensionInput`: that
 * component owns the `aria-label="Dimension value"` locator every spec uses to
 * reach the PRIMARY value, so a second instance would make it ambiguous.
 *
 * Commit rules: Enter applies and confirms, blur applies, Esc reverts. Empty or
 * the sentinel commits `null`. A value the length parser refuses is reverted
 * rather than read as a partial number.
 */
function ThroughOrDepthField({
  value,
  onValue,
}: {
  value: number | null;
  onValue: (v: number | null) => void;
}) {
  const unit = useSettingsStore((s) => s.displayUnit);
  const shown = (v: number | null) => (v === null ? THROUGH_TEXT : formatLength(v, unit));
  const [text, setText] = useState(() => shown(value));

  useEffect(() => {
    setText(shown(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, unit]);

  const commit = (): void => {
    const trimmed = text.trim();
    if (trimmed === "" || trimmed.toLowerCase() === THROUGH_TEXT.toLowerCase()) {
      if (value !== null) onValue(null);
      setText(THROUGH_TEXT);
      return;
    }
    const n = parseLength(trimmed, unit);
    if (n === undefined || n === null || !Number.isFinite(n) || n <= 0) {
      setText(shown(value));
      return;
    }
    if (n !== value) onValue(n);
    setText(shown(n));
  };

  return (
    <span className="pointer-events-auto inline-flex items-center gap-0.5 rounded-sm border border-border bg-surface px-1 font-mono text-[11px] text-ink-2 shadow-ctrl">
      <span aria-hidden className="text-ink-3">
        ↧
      </span>
      <input
        aria-label="Hole depth"
        data-testid="chip-hole-depth"
        className={cn("bg-transparent text-right outline-none", unit === "mm" ? "w-10" : "w-14")}
        value={text}
        inputMode="decimal"
        placeholder={THROUGH_TEXT}
        title="Hole depth — 'Thru' drills all the way through"
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.stopPropagation();
            commit();
            toolChipStore.getState().onConfirm?.();
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

/** A plain labelled length field for a conditional dimension. */
function DimField({
  label,
  testid,
  value,
  onValue,
}: {
  label: string;
  testid: string;
  value: number;
  onValue: (v: number) => void;
}) {
  const unit = useSettingsStore((s) => s.displayUnit);
  const [text, setText] = useState(() => formatLength(value, unit));

  useEffect(() => {
    setText(formatLength(value, unit));
  }, [value, unit]);

  const commit = (): void => {
    const n = parseLength(text.trim(), unit);
    if (n === undefined || n === null || !Number.isFinite(n) || n <= 0) {
      setText(formatLength(value, unit));
      return;
    }
    if (n !== value) onValue(n);
    setText(formatLength(n, unit));
  };

  return (
    <span className="pointer-events-auto inline-flex items-center gap-0.5 rounded-sm border border-border bg-surface px-1 font-mono text-[11px] text-ink-2 shadow-ctrl">
      <span aria-hidden className="text-ink-3">
        {label}
      </span>
      <input
        aria-label={testid}
        data-testid={testid}
        className={cn("bg-transparent text-right outline-none", unit === "mm" ? "w-10" : "w-14")}
        value={text}
        inputMode="decimal"
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.stopPropagation();
            commit();
            toolChipStore.getState().onConfirm?.();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            setText(formatLength(value, unit));
          }
        }}
      />
    </span>
  );
}

/**
 * The standard-size picker. Purely a FILLER of raw millimetres — the pick writes
 * numbers into the params and the thread designation is never persisted (SCHEMA
 * §7.3: "standard-size TABLES … are a FRONTEND concern"). See
 * `tools/modelTools/holeStandards.ts` for each column's cited standard.
 */
function StandardsPicker({ onPick }: { onPick: (thread: string, fit: HoleFit) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="pointer-events-auto relative inline-flex">
      <button
        type="button"
        data-testid="chip-hole-std"
        aria-expanded={open}
        aria-label="Standard sizes"
        onClick={() => setOpen((v) => !v)}
        className="rounded-sm bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
      >
        Std ▾
      </button>
      {open && (
        <div
          data-testid="chip-hole-std-panel"
          className="absolute left-0 top-full z-10 mt-1 rounded-md border border-border bg-surface p-1 shadow-panel"
        >
          <div className="grid grid-cols-[auto_auto_auto] items-center gap-x-1 gap-y-0.5">
            <span aria-hidden className="px-1 text-[10px] font-medium text-ink-4" />
            {FITS.map((f) => (
              <span key={f.fit} aria-hidden className="px-1 text-[10px] font-medium text-ink-4">
                {f.label}
              </span>
            ))}
            {HOLE_STANDARDS.map((size) => (
              <FragmentRow key={size.thread} thread={size.thread} onPick={onPick} setOpen={setOpen} />
            ))}
          </div>
        </div>
      )}
    </span>
  );
}

function FragmentRow({
  thread,
  onPick,
  setOpen,
}: {
  thread: string;
  onPick: (thread: string, fit: HoleFit) => void;
  setOpen: (v: boolean) => void;
}) {
  return (
    <>
      <span className="px-1 font-mono text-[11px] text-ink-2">{thread}</span>
      {FITS.map((f) => (
        <button
          key={f.fit}
          type="button"
          data-testid={`chip-hole-std-${thread}-${f.fit}`}
          onClick={() => {
            onPick(thread, f.fit);
            setOpen(false);
          }}
          className="rounded-sm bg-chip px-1.5 py-0.5 font-mono text-[11px] text-ink-3 hover:bg-hover-2"
        >
          ·
        </button>
      ))}
    </>
  );
}

export function HoleChipCluster() {
  const diameter = useToolChipStore((s) => s.value);
  const holeType = useToolChipStore((s) => s.holeType);
  const depth = useToolChipStore((s) => s.holeDepth);
  const cbDiameter = useToolChipStore((s) => s.cbDiameter);
  const cbDepth = useToolChipStore((s) => s.cbDepth);
  const csDiameter = useToolChipStore((s) => s.csDiameter);
  const csAngleDeg = useToolChipStore((s) => s.csAngleDeg);

  return (
    <>
      <Segments
        label="Hole type"
        active={holeType}
        options={HOLE_TYPES.map((t) => ({
          value: t.holeType,
          label: t.label,
          testid: t.testid,
        }))}
        onPick={(v) => toolChipStore.getState().onHoleType?.(v)}
      />
      <DimensionInput
        value={diameter}
        suffix={LENGTH_SUFFIX}
        onCommit={(v) => toolChipStore.getState().onValue?.(v)}
        onConfirm={() => toolChipStore.getState().onConfirm?.()}
      />
      <ThroughOrDepthField
        value={depth}
        onValue={(v) => toolChipStore.getState().onDepth?.(v)}
      />
      {holeType === "counterbore" && (
        <>
          <DimField
            label="⌴Ø"
            testid="chip-hole-cb-diameter"
            value={cbDiameter}
            onValue={(v) => toolChipStore.getState().onCbDiameter?.(v)}
          />
          <DimField
            label="⌴↧"
            testid="chip-hole-cb-depth"
            value={cbDepth}
            onValue={(v) => toolChipStore.getState().onCbDepth?.(v)}
          />
        </>
      )}
      {holeType === "countersink" && (
        <>
          <DimField
            label="⌵Ø"
            testid="chip-hole-cs-diameter"
            value={csDiameter}
            onValue={(v) => toolChipStore.getState().onCsDiameter?.(v)}
          />
          {/* SCHEMA §7.3 admits exactly four included angles, so this is a
              segmented pick rather than a free field — a typed 95° would be
              refused by the backend after the fact. */}
          <Segments
            label="Countersink angle"
            active={String(csAngleDeg)}
            options={HOLE_CS_ANGLES.map((a) => ({
              value: String(a),
              label: `${a}°`,
              testid: `chip-hole-cs-${a}`,
            }))}
            onPick={(v) => toolChipStore.getState().onCsAngle?.(Number(v))}
          />
        </>
      )}
      <StandardsPicker onPick={(thread, fit) => toolChipStore.getState().onStandard?.(thread, fit)} />
    </>
  );
}
