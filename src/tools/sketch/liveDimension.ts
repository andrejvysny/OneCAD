/*
 * Live dimension input (SP-1) — PURE core: value types, the zoom-adaptive
 * rounding quantum, the chip input FSM, and the frame → chip descriptor pass.
 *
 * TWO ways a number reaches the geometry, and they are NOT the same promise:
 *   - TYPED  → the field is LOCKED. It drives the geometry EXACTLY and, where a
 *              wire constraint kind exists for it, also authors a DRIVING
 *              constraint (`liveDimConstraints.ts`). Never re-rounded.
 *   - CURSOR → the measured value is ROUNDED to `DimQuantum`, a zoom-adaptive
 *              granularity. Geometry only — never a constraint.
 * Locks are applied BEFORE rounding everywhere (`liveToolMachines.projectEvent`),
 * so the chip, the rubber-band preview and the committed entity are one number.
 *
 * HONEST RULE (`DimFieldSpec.drives`): a typed value ALWAYS drives geometry, but
 * it only authors a constraint where a wire kind exists for it. Ellipse major /
 * minor, arc sweep angle and polygon side count have none — those fields say so
 * rather than pretending the number is parametric.
 *
 * NO THREE import: the camera-derived grid step arrives as a plain number
 * (`chooseGridStep(distance).minor`), the same convention `computeSnap({gridStep})`
 * already uses. This module must stay loadable without a renderer.
 */
import { LENGTH_UNITS, type LengthUnitId } from "@/units/lengthUnits";
import { displayToMm, mmToDisplay } from "@/units/format";
import type { Point2 } from "@/viewport/engine/sketchBasis";

/** Every dimension a draw gesture can expose. Ids are stable — chips, locks and
 *  the constraint authoring all key off them. */
export type DimFieldId =
  | "length"
  | "angle"
  | "width"
  | "height"
  | "radius"
  | "diameter"
  | "major"
  | "minor"
  | "sides";

/** What a field's number MEANS: mm, DEGREES, or a bare integer count. */
export type DimDomain = "length" | "angle" | "count";

/** One live chip: what it reads, where it sits, and whether it is user-pinned. */
export interface ToolDimension {
  field: DimFieldId;
  /** Chip prefix — "L","∠","W","H","R","Ø","a","b","n". */
  label: string;
  domain: DimDomain;
  /** mm · DEGREES · integer, per `domain`. */
  value: number;
  /** Plane-coord anchor (the engine projects it to screen). */
  anchor: Point2;
  locked: boolean;
  /** Authors a driving constraint on commit? See the HONEST RULE above. */
  drives: boolean;
  /** Stagger index among co-anchored fields (badgeLayout technique). */
  offsetIndex: number;
}

/** Rounding granularity per domain (mm / degrees). */
export interface DimQuantum {
  length: number;
  angle: number;
}

/** User-pinned field values (mm · DEGREES · count). */
export type DimLocks = Partial<Record<DimFieldId, number>>;

/** Field values keyed by id, as `measure`/`rebuild` exchange them. */
export type DimValues = Partial<Record<DimFieldId, number>>;

/** Angles round to whole degrees at every zoom — a degree is already the unit a
 *  person thinks in, and it does not get finer as you zoom in. */
export const ANGLE_QUANTUM_DEG = 1;

/** One editable field a frame exposes; `anchor` places its chip in plane coords. */
export interface DimFieldSpec {
  field: DimFieldId;
  label: string;
  domain: DimDomain;
  drives: boolean;
  anchor(pt: Point2): Point2;
}

/**
 * The MEASURING half of a `DimFrame` — everything `describeDims` needs.
 *
 * Declared HERE rather than in `liveDimFrames.ts` so the dependency edge stays
 * one-way (`liveDimension → liveDimFrames → liveToolMachines`): the descriptor
 * pass must not import the frame table it describes.
 */
export interface DimMeasureFrame {
  /** Tab order. */
  fields: DimFieldSpec[];
  measure(pt: Point2): DimValues;
}

/**
 * Field pairs that are two VIEWS OF ONE NUMBER. Locking or rounding one must
 * never leave the other holding a contradicting value for the same geometry —
 * `liveDimStep` clears the sibling LOCK, `projectEvent` drops the sibling VALUE.
 */
export const DIM_ALIASES: ReadonlyArray<readonly [DimFieldId, DimFieldId]> = [
  ["diameter", "radius"],
];

/** The alias sibling of `field`, or null. */
export function aliasOf(field: DimFieldId): DimFieldId | null {
  for (const [a, b] of DIM_ALIASES) {
    if (a === field) return b;
    if (b === field) return a;
  }
  return null;
}

// ── quantum ───────────────────────────────────────────────────────────────────

/** Ladder mantissas. 1/2/5 (not GridPlane's 1/5/10) so the step never jumps 5×. */
const DECADE_LADDER = [1, 2, 5] as const;
/** `0.5 / 0.1` is 4.999999999999999 in binary floating point — without this, a
 *  value sitting EXACTLY on a rung falls to the rung below. */
const LADDER_EPS = 1e-9;

/**
 * Round DOWN to the nearest 1/2/5 decade step.
 *
 * NOT `GridPlane.snapToDecade`, which rounds UP on a 1/5/10 ladder: applied to a
 * quantum that would coarsen the granularity by as much as 2× (0.5 → 1), so a
 * user zoomed in far enough to see tenths would still be rounded to halves.
 * Non-positive / non-finite input yields 0 — a quantum of 0 means "do not round"
 * (`roundTo`), which is the safe direction.
 */
export function decadeFloor(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return 0;
  const pow = Math.pow(10, Math.floor(Math.log10(v)));
  const n = v / pow; // [1, 10)
  const m = n >= 5 - LADDER_EPS ? DECADE_LADDER[2] : n >= 2 - LADDER_EPS ? DECADE_LADDER[1] : DECADE_LADDER[0];
  return m * pow;
}

/**
 * The rounding granularity for the current zoom, computed in the DISPLAY unit.
 *
 * "Whole numbers" means whole DISPLAYED numbers: an inch session rounds to a
 * round number of inches, not to a ragged mm value that merely looks round in mm.
 * Target is a tenth of the minor grid cell, floored onto the 1/2/5 ladder, and
 * never finer than the unit can display (10^-decimals) — a quantum below the
 * chip's own precision would round to a number the chip cannot even print.
 */
export function dimQuantum(gridMinorMm: number, unit: LengthUnitId): DimQuantum {
  const finest = Math.pow(10, -LENGTH_UNITS[unit].decimals);
  const q = Math.max(decadeFloor(mmToDisplay(gridMinorMm, unit) / 10), finest);
  return { length: displayToMm(q, unit), angle: ANGLE_QUANTUM_DEG };
}

/** Round `v` to a multiple of `q`; `q <= 0` means "do not round". */
export const roundTo = (v: number, q: number): number => (q > 0 ? Math.round(v / q) * q : v);

/** Fold a degree value into [0, 360). */
export function norm360(deg: number): number {
  const d = deg % 360;
  return d < 0 ? d + 360 : d;
}

// ── input FSM ─────────────────────────────────────────────────────────────────

export interface LiveDimState {
  focus: DimFieldId | null;
  locks: DimLocks;
  /** Raw text of the FOCUSED field (the store's single source while editing). */
  text: string;
}

export type LiveDimEvent =
  /** First digit / `-` / `.` typed with no chip focused — opens `fields[0]`. */
  | { kind: "typeStart"; ch: string; fields: DimFieldId[] }
  | { kind: "text"; text: string }
  | { kind: "focus"; field: DimFieldId }
  | { kind: "tab"; fields: DimFieldId[]; back: boolean; value: number | null }
  | { kind: "enter"; value: number | null }
  /** Viewport click while a field is focused: lock the value, do NOT commit. */
  | { kind: "blurCommit"; value: number | null }
  | { kind: "escField" }
  | { kind: "reset" };

export interface LiveDimStep {
  state: LiveDimState;
  /** Enter on a valid value both locks the field AND commits the gesture. */
  commitGesture?: boolean;
  /** The field this step pinned (controller re-steps the machine at the cursor). */
  locked?: DimFieldId;
}

export function liveDimInit(): LiveDimState {
  return { focus: null, locks: {}, text: "" };
}

/** Next field in Tab order, wrapping; `back` for Shift+Tab. */
function nextField(fields: DimFieldId[], from: DimFieldId, back: boolean): DimFieldId | null {
  if (fields.length === 0) return null;
  const i = fields.indexOf(from);
  if (i < 0) return fields[0];
  return fields[(i + (back ? -1 : 1) + fields.length) % fields.length];
}

/** Pin `field`, dropping its alias sibling's lock (see `DIM_ALIASES`). */
function withLock(locks: DimLocks, field: DimFieldId, value: number): DimLocks {
  const next: DimLocks = { ...locks, [field]: value };
  const alias = aliasOf(field);
  if (alias) delete next[alias];
  return next;
}

function withoutLock(locks: DimLocks, field: DimFieldId): DimLocks {
  const next: DimLocks = { ...locks };
  delete next[field];
  return next;
}

/**
 * The chip input reducer. Pure — DOM focus is the priority rule (the controller
 * bails on editable targets before this is ever reached), so this only has to
 * describe what a field DOES with a key, never who owns it.
 */
export function liveDimStep(s: LiveDimState, e: LiveDimEvent): LiveDimStep {
  switch (e.kind) {
    case "reset":
      return { state: liveDimInit() };
    case "focus":
      return { state: { ...s, focus: e.field, text: "" } };
    case "typeStart":
      // Only the FIRST keystroke opens a chip; a focused field owns its own input.
      if (s.focus !== null || e.fields.length === 0) return { state: s };
      return { state: { ...s, focus: e.fields[0], text: e.ch } };
    case "text":
      return { state: s.focus === null ? s : { ...s, text: e.text } };
    case "tab": {
      if (s.focus === null) return { state: s };
      const focus = nextField(e.fields, s.focus, e.back) ?? s.focus;
      if (e.value === null) return { state: { ...s, focus, text: "" } };
      return { state: { focus, locks: withLock(s.locks, s.focus, e.value), text: "" }, locked: s.focus };
    }
    case "enter":
      // An unparseable value is NOT a commit — the chip flashes and keeps focus.
      if (s.focus === null || e.value === null) return { state: s };
      return {
        state: { focus: null, locks: withLock(s.locks, s.focus, e.value), text: "" },
        locked: s.focus,
        commitGesture: true,
      };
    case "blurCommit":
      // The field lost DOM focus either way; only a PARSEABLE value pins.
      if (s.focus === null) return { state: s };
      if (e.value === null) return { state: { ...s, focus: null, text: "" } };
      return { state: { focus: null, locks: withLock(s.locks, s.focus, e.value), text: "" }, locked: s.focus };
    case "escField":
      // Esc on a chip drops THAT field's lock only; a second Esc runs the
      // controller's existing gesture ladder.
      return s.focus === null
        ? { state: s }
        : { state: { focus: null, locks: withoutLock(s.locks, s.focus), text: "" } };
  }
}

// ── descriptors ───────────────────────────────────────────────────────────────

/** Quantized anchor grouping key — guards float noise between two chips that are
 *  really "the same" anchor (badgeLayout.ts precedent). */
const ANCHOR_QUANT = 1e-4;
const anchorKey = (p: Point2): string =>
  `${Math.round(p.x / ANCHOR_QUANT)},${Math.round(p.y / ANCHOR_QUANT)}`;

/** Stagger co-anchored chips into a row instead of stacking them. Deterministic:
 *  the frame's field list IS the Tab order, so indices never reshuffle. */
function assignOffsets(dims: ToolDimension[]): ToolDimension[] {
  const seen = new Map<string, number>();
  for (const d of dims) {
    const key = anchorKey(d.anchor);
    const n = seen.get(key) ?? 0;
    d.offsetIndex = n;
    seen.set(key, n + 1);
  }
  return dims;
}

/**
 * Assemble the chip descriptors for one frame at one cursor position.
 *
 * A LOCKED field reads its locked value verbatim — the cursor has already been
 * rebuilt through it (`projectEvent`), so measuring would only reintroduce float
 * drift into a number the user typed.
 */
export function describeDims(
  frame: DimMeasureFrame,
  cursor: Point2,
  locks: DimLocks,
): ToolDimension[] {
  const measured = frame.measure(cursor);
  return assignOffsets(
    frame.fields.map((spec) => ({
      field: spec.field,
      label: spec.label,
      domain: spec.domain,
      value: locks[spec.field] ?? measured[spec.field] ?? 0,
      anchor: spec.anchor(cursor),
      locked: locks[spec.field] !== undefined,
      drives: spec.drives,
      offsetIndex: 0,
    })),
  );
}
