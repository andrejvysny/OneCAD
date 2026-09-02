/**
 * The Hole tool's pure reducer (WP-C T3; SCHEMA §7.3 `Hole`).
 *
 * Lives in its own module rather than in `modelToolMachine.ts`: that file is
 * already 1400+ lines, and the hole is the first tool whose params have
 * CONDITIONAL blocks — the `cb*`/`cs*` clearing rule below is the whole reason
 * this reducer is worth testing in isolation.
 *
 * Same contract as every other machine here: pure `(state, event) → {state, effect}`;
 * the controller performs the effect. The reducer never imports `@/ipc/client` and
 * treats the picked face ref as OPAQUE (`unknown`), exactly like Extrude's ToFace
 * target — that keeps it free of the wire types and trivially testable.
 *
 * PHASES. `idle → facePick → armed → committing`. The pick phase PRECEDES `armed`
 * (the Revolve axis-pick precedent): a hole has no meaning until a seat is chosen,
 * so there is nothing to preview and nothing to commit before the face click. A
 * second click on the same face MOVES the hole rather than re-arming — the seat is
 * the expensive choice, the position is cheap.
 *
 * CONDITIONAL BLOCKS. `holeType` selects which of `cb*` / `cs*` the record may
 * carry, and the Rust session REJECTS a record carrying the other one (a stale
 * conditional would resurrect on the next profile flip). The FSM therefore keeps
 * both sets of numbers as live state — flipping counterbore → countersink →
 * counterbore hands the user their counterbore back — and it is
 * {@link holeParamsOf} that emits ONLY the active block. State is memory; the
 * record is the truth.
 */

import type { HoleParams, HoleThread, HoleType, SemanticRef } from "@/ipc/types";
import type { ModelPhase, ToolEffect } from "./modelToolMachine";
import type { HoleStandardPatch } from "./holeStandards";

/** `idle → facePick → armed → committing`. */
export type HolePhase = ModelPhase | "facePick";

/** The countersink included angles SCHEMA §7.3 admits (degrees). */
export const HOLE_CS_ANGLES = [82, 90, 100, 120] as const;

/** No dimension smaller than the worker's `kMinValue` may be authored. */
export const HOLE_MIN_VALUE = 0.01;

/**
 * M6 medium-series clearance (ISO 273) — the single most common hole in a
 * hand-authored metric part, and consistent with the counterbore/countersink
 * defaults below so a fresh arm is already a coherent M6 feature.
 */
export const DEFAULT_HOLE_DIAMETER = 6.6;
/** DIN 974-1 row-1 counterbore for an M6 socket-head cap screw. */
export const DEFAULT_HOLE_CB_DIAMETER = 11;
export const DEFAULT_HOLE_CB_DEPTH = 6.8;
/** DIN 74 form A countersink for M6. */
export const DEFAULT_HOLE_CS_DIAMETER = 12.4;
export const DEFAULT_HOLE_CS_ANGLE = 90;

export interface HoleFsm {
  phase: HolePhase;
  /** The picked host FACE ref, opaque to the reducer (a `SemanticRef` in practice). */
  face: unknown | null;
  /** The body the face belongs to — the record's `targetBodyId`. */
  targetBodyId: string;
  /** World-frozen hole centre; `null` until the face click supplies one. */
  point: [number, number, number] | null;
  holeType: HoleType;
  diameter: number;
  /** Blind depth, or `null` = THROUGH-ALL (the default: the commonest hole). */
  depth: number | null;
  /** Counterbore block — retained across profile flips, EMITTED only when active. */
  cbDiameter: number;
  cbDepth: number;
  /** Countersink block — same retention rule. */
  csDiameter: number;
  csAngleDeg: number;
  /**
   * A tapped hole (WP-T1). Presence-discriminated and ORTHOGONAL to
   * `holeType` — unlike `cb*`/`cs*` it survives every profile flip untouched.
   * `null` = unthreaded.
   */
  thread: HoleThread | null;
  /** Absent on a re-edited legacy Hole; fresh authoring uses strict V2. */
  resultPolicyVersion?: 2;
  /**
   * Whether the user has authored a diameter yet. A pristine arm takes a standards
   * preset or a profile default wholesale; once a size is theirs it survives
   * (the Fillet `touched` precedent).
   */
  touched: boolean;
}

export type HoleEvent =
  /** Tool activated — wait for a face click. */
  | { kind: "start" }
  /** A planar face was accepted. Arms the tool at `point`. */
  | { kind: "pickFace"; face: unknown; targetBodyId: string; point: [number, number, number] }
  /** Another click on the SAME face — move the hole, stay armed. */
  | { kind: "movePoint"; point: [number, number, number] }
  | { kind: "setHoleType"; holeType: HoleType }
  | { kind: "setDiameter"; diameter: number }
  /** `null` = through-all. */
  | { kind: "setDepth"; depth: number | null }
  | { kind: "setCbDiameter"; value: number }
  | { kind: "setCbDepth"; value: number }
  | { kind: "setCsDiameter"; value: number }
  | { kind: "setCsAngle"; angleDeg: number }
  /** WP-T1: `null` clears the thread (back to an unthreaded hole). */
  | { kind: "setThread"; thread: HoleThread | null }
  /** A standards-picker choice, already reduced to raw mm. */
  | { kind: "applyStandard"; patch: HoleStandardPatch }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface HoleStep {
  state: HoleFsm;
  effect: ToolEffect;
}

export function holeInit(): HoleFsm {
  return {
    phase: "idle",
    face: null,
    targetBodyId: "",
    point: null,
    holeType: "simple",
    diameter: DEFAULT_HOLE_DIAMETER,
    depth: null,
    cbDiameter: DEFAULT_HOLE_CB_DIAMETER,
    cbDepth: DEFAULT_HOLE_CB_DEPTH,
    csDiameter: DEFAULT_HOLE_CS_DIAMETER,
    csAngleDeg: DEFAULT_HOLE_CS_ANGLE,
    thread: null,
    resultPolicyVersion: 2,
    touched: false,
  };
}

/** Clamp a dimension to the authorable domain; a non-finite value is refused. */
function dim(v: number, fallback: number): number {
  if (!Number.isFinite(v)) return fallback;
  return Math.max(HOLE_MIN_VALUE, v);
}

/** True iff `s` is in a phase whose params may be edited. */
function editable(s: HoleFsm): boolean {
  return s.phase === "armed";
}

export function holeStep(s: HoleFsm, e: HoleEvent): HoleStep {
  switch (e.kind) {
    case "start":
      // Keeps nothing from a prior arm — a new hole is a new seat.
      return { state: { ...holeInit(), phase: "facePick" }, effect: "none" };

    case "pickFace":
      if (s.phase !== "facePick" && s.phase !== "armed") return { state: s, effect: "none" };
      return {
        state: {
          ...s,
          phase: "armed",
          face: e.face,
          targetBodyId: e.targetBodyId,
          point: e.point,
        },
        effect: "begin",
      };

    case "movePoint":
      // Re-positioning an armed hole on its own face — no new preview session,
      // just newer params on the live one.
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, point: e.point }, effect: "update" };

    case "setHoleType": {
      if (!editable(s)) return { state: s, effect: "none" };
      if (e.holeType === s.holeType) return { state: s, effect: "none" };
      // The other profile's numbers are KEPT (see the module doc) — only which
      // block `holeParamsOf` emits changes.
      return { state: { ...s, holeType: e.holeType }, effect: "update" };
    }

    case "setDiameter":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, diameter: dim(e.diameter, s.diameter), touched: true },
        effect: "update",
      };

    case "setDepth": {
      if (!editable(s)) return { state: s, effect: "none" };
      // `null` is a REAL value here (through-all), not a missing one.
      const depth = e.depth === null ? null : dim(e.depth, s.depth ?? DEFAULT_HOLE_DIAMETER);
      return { state: { ...s, depth }, effect: "update" };
    }

    case "setCbDiameter":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, cbDiameter: dim(e.value, s.cbDiameter) }, effect: "update" };

    case "setCbDepth":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, cbDepth: dim(e.value, s.cbDepth) }, effect: "update" };

    case "setCsDiameter":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, csDiameter: dim(e.value, s.csDiameter) }, effect: "update" };

    case "setCsAngle":
      if (!editable(s)) return { state: s, effect: "none" };
      // SCHEMA §7.3 admits exactly four included angles. A value outside the set
      // is a caller bug (the chip renders only these four), not a number to store:
      // the backend would refuse the whole op.
      if (!HOLE_CS_ANGLES.includes(e.angleDeg as (typeof HOLE_CS_ANGLES)[number])) {
        return { state: s, effect: "none" };
      }
      return { state: { ...s, csAngleDeg: e.angleDeg }, effect: "update" };

    case "setThread":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, thread: e.thread }, effect: "update" };

    case "applyStandard": {
      if (!editable(s)) return { state: s, effect: "none" };
      const p = e.patch;
      return {
        state: {
          ...s,
          diameter: dim(p.diameter, s.diameter),
          cbDiameter: p.cbDiameter === undefined ? s.cbDiameter : dim(p.cbDiameter, s.cbDiameter),
          cbDepth: p.cbDepth === undefined ? s.cbDepth : dim(p.cbDepth, s.cbDepth),
          csDiameter: p.csDiameter === undefined ? s.csDiameter : dim(p.csDiameter, s.csDiameter),
          csAngleDeg: p.csAngleDeg ?? s.csAngleDeg,
          // A standards pick is AUTHORITATIVE for thread presence: `p.thread` is
          // set only when the caller's thread toggle is on (`holeStandardPatch`'s
          // `threaded` arg), so an untreaded pick clears any prior thread rather
          // than leaving it stranded against a diameter it no longer matches.
          thread: p.thread ?? null,
          // A preset IS an authored size — a later profile flip must not undo it.
          touched: true,
        },
        effect: "update",
      };
    }

    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      if (s.face === null || s.point === null) return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };

    case "commitFailed":
      // The kernel refused — back to armed with the seat and the numbers intact.
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };

    case "settle":
      return { state: holeInit(), effect: "none" };

    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: holeInit(), effect: "cancel" };
  }
}

/**
 * The canonical `HoleParams` for an armed hole — the ONE builder both the exact
 * preview and the commit go through (SCHEMA §7.6: "preview callers MUST NOT
 * maintain a second ad-hoc mapper").
 *
 * Emits ONLY the active conditional block, and emits it as `null` otherwise
 * rather than omitting the key: `null` is what the Rust `Option<Scalar>` reads
 * back as, and spelling it makes "this hole is not a countersink" an authored
 * fact instead of an omission.
 *
 * Throws when the seat is missing — a hole without a face or a point is not a
 * partial op to send, it is a caller bug (the `previewOps.ts` throw convention).
 */
export function holeParamsOf(s: HoleFsm): HoleParams {
  if (s.face === null || s.point === null) {
    throw new Error("Hole requires a picked face and a point");
  }
  if (!s.targetBodyId) throw new Error("Hole requires a target body");
  const cb = s.holeType === "counterbore";
  const cs = s.holeType === "countersink";
  return {
    targetBodyId: s.targetBodyId,
    face: s.face as SemanticRef,
    point: [s.point[0], s.point[1], s.point[2]],
    holeType: s.holeType,
    diameter: s.diameter,
    depth: s.depth,
    cbDiameter: cb ? s.cbDiameter : null,
    cbDepth: cb ? s.cbDepth : null,
    csDiameter: cs ? s.csDiameter : null,
    csAngleDeg: cs ? s.csAngleDeg : null,
    ...(s.thread ? { thread: s.thread } : {}),
    ...(s.resultPolicyVersion === 2 ? { resultPolicyVersion: 2 as const } : {}),
  };
}

/** The numeric `.value` of a wire `Scalar` (`{value, expr?}`), or a bare number. */
function wireScalar(v: unknown): number | undefined {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (v && typeof v === "object" && "value" in (v as Record<string, unknown>)) {
    const n = (v as { value: unknown }).value;
    if (typeof n === "number" && Number.isFinite(n)) return n;
  }
  return undefined;
}

/**
 * A stored `thread` block (WP-T1), or `null` for an unthreaded hole / a block
 * this build cannot make sense of (missing/malformed — restoring nothing is
 * the honest outcome, matching {@link holeFsmFromParams}'s own refusal rule).
 * `majorDiameterMm` is a PLAIN wire number (not a Scalar); `pitchMm`/`depthMm`
 * are.
 */
function threadFromWire(stored: unknown): HoleThread | null {
  if (!stored || typeof stored !== "object") return null;
  const t = stored as Record<string, unknown>;
  if (t.standard !== "ISO261") return null;
  if (typeof t.designation !== "string" || t.designation.length === 0) return null;
  if (typeof t.majorDiameterMm !== "number" || !Number.isFinite(t.majorDiameterMm)) return null;
  const pitchMm = wireScalar(t.pitchMm);
  if (pitchMm === undefined) return null;
  const detail = t.detail;
  if (detail !== "cosmetic" && detail !== "simplified" && detail !== "modeled") return null;
  return {
    standard: "ISO261",
    designation: t.designation,
    majorDiameterMm: t.majorDiameterMm,
    pitchMm,
    depthMm: wireScalar(t.depthMm) ?? null,
    detail,
  };
}

/**
 * Seed an ARMED `HoleFsm` from a record's stored wire params (`getOperationParams`).
 *
 * A hole re-edit restores EVERY field, including the frozen seat — unlike the
 * value-only re-edits, the user never re-picks the face. Returns `null` when the
 * params are missing or carry no usable seat: refusing is the honest outcome, and
 * arming a hole against a seat that is not there would drill somewhere arbitrary.
 *
 * The INACTIVE profile's numbers come back as `null` on the wire (that is the
 * contract), so they fall back to the defaults — which is correct: there is
 * nothing authored to restore, and a flip to that profile should start from a
 * sensible size rather than from zero.
 */
export function holeFsmFromParams(stored: Record<string, unknown> | undefined): HoleFsm | null {
  if (!stored) return null;
  const face = stored.face;
  if (!face || typeof face !== "object") return null;
  const primary = (face as { primary?: { bodyId?: unknown } }).primary;
  const bodyId =
    typeof primary?.bodyId === "string" && primary.bodyId.length > 0 ? primary.bodyId : undefined;
  const targetBodyId =
    typeof stored.targetBodyId === "string" && stored.targetBodyId.length > 0
      ? stored.targetBodyId
      : bodyId;
  if (!targetBodyId) return null;
  const raw = stored.point;
  if (!Array.isArray(raw) || raw.length !== 3 || raw.some((c) => typeof c !== "number")) return null;
  const diameter = wireScalar(stored.diameter);
  if (diameter === undefined) return null;
  const policy = stored.resultPolicyVersion;
  if (policy !== undefined && policy !== 2) return null;
  const holeType = stored.holeType;
  const base = holeInit();
  return {
    ...base,
    phase: "armed",
    face: face as HoleFsm["face"],
    targetBodyId,
    point: [raw[0] as number, raw[1] as number, raw[2] as number],
    holeType:
      holeType === "counterbore" || holeType === "countersink" ? holeType : "simple",
    diameter,
    // `null`/absent depth is THROUGH-ALL — a real stored value, not a gap.
    depth: wireScalar(stored.depth) ?? null,
    cbDiameter: wireScalar(stored.cbDiameter) ?? base.cbDiameter,
    cbDepth: wireScalar(stored.cbDepth) ?? base.cbDepth,
    csDiameter: wireScalar(stored.csDiameter) ?? base.csDiameter,
    csAngleDeg: wireScalar(stored.csAngleDeg) ?? base.csAngleDeg,
    thread: threadFromWire(stored.thread),
    resultPolicyVersion: policy === 2 ? 2 : undefined,
    // A restored size is an authored one.
    touched: true,
  };
}

/**
 * The mono value text a hole's history row shows — mirrors `dto.rs
 * feature_value_text` exactly, including the WP-T1 threaded form
 * (`designation` appended as `Ø5.0 · M6x1`).
 */
export function holeValueText(diameter: number, designation?: string): string {
  const base = `Ø${diameter.toFixed(1)}`;
  return designation ? `${base} · ${designation}` : base;
}
