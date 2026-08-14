/**
 * The Gear tool's pure reducer (Gear Generator G1; SCHEMA §7.3 `Gear`).
 *
 * Same contract as every other machine here: pure `(state, event) → {state, effect}`;
 * the controller performs the effect. The reducer never imports `@/ipc/client` and
 * treats the picked face ref as OPAQUE (`unknown`), exactly like Hole's seat —
 * that keeps it free of the wire types and trivially testable.
 *
 * PHASES. `idle → facePick → armed → committing`, Hole's shape. The pick phase
 * precedes `armed` because a gear has to sit somewhere before there is anything
 * to preview.
 *
 * TWO PLACEMENT MODES, and only one may be live at a time (SCHEMA §7.3):
 *   - a picked planar FACE, whose inward normal becomes the gear's axis, or
 *   - an explicit frozen FRAME, which the tool offers as "place on the XY datum"
 *     so a gear can be authored in an empty document with no body to click.
 * The reducer keeps them mutually exclusive by construction: choosing one clears
 * the other, so {@link gearParamsOf} can never emit both.
 *
 * WHAT IS DELIBERATELY NOT HERE. `helixAngleDeg` / `doubleHelix` are refused by
 * the Rust session and the worker in this version, so the FSM does not carry
 * them as editable state at all — offering a control that always fails would be
 * worse than not offering it. They reappear here when the sweep infrastructure
 * lands, alongside their schema change.
 */

import type { GearParams, GearRecipe, InvoluteExternalParams, SemanticRef } from "@/ipc/types";
import type { ModelPhase, ToolEffect } from "./modelToolMachine";

/** `idle → facePick → armed → committing`. */
export type GearPhase = ModelPhase | "facePick";

/** No dimension smaller than the worker's `kMinValue` may be authored. */
export const GEAR_MIN_VALUE = 0.01;

/**
 * A 20-tooth, module-2, 20° spur gear: the most ordinary metric gear there is,
 * and a coherent whole rather than a set of independent defaults — 20 teeth is
 * above the undercut threshold at 20°, so a fresh arm needs no corrections.
 */
export const DEFAULT_GEAR_TEETH = 20;
export const DEFAULT_GEAR_MODULE = 2;
export const DEFAULT_GEAR_HEIGHT = 5;
export const DEFAULT_GEAR_PRESSURE_ANGLE = 20;
/** The ISO-standard dedendum clearance coefficient. */
export const DEFAULT_GEAR_CLEARANCE = 0.25;
/** Spline samples per curve segment — upstream's default, and visually smooth. */
export const DEFAULT_GEAR_SAMPLE_COUNT = 20;
export const DEFAULT_GEAR_AXLE_HOLE_DIAMETER = 10;
export const DEFAULT_GEAR_OFFSET_HOLE_DIAMETER = 10;
export const DEFAULT_GEAR_OFFSET_HOLE_OFFSET = 10;

/** Teeth below this cannot form a valid involute gear (the worker refuses too). */
export const GEAR_MIN_TEETH = 3;

export interface GearFsm {
  phase: GearPhase;
  recipe: GearRecipe;

  /** The picked seat FACE ref, opaque to the reducer (a `SemanticRef` in practice). */
  face: unknown | null;
  /**
   * An explicit frozen frame, used when the gear is placed on a datum instead of
   * a picked face. Mutually exclusive with {@link face}.
   */
  frame: { origin: [number, number, number]; axis: [number, number, number]; xDir: [number, number, number] } | null;
  /** World-frozen gear centre; `null` until a placement supplies one. */
  point: [number, number, number] | null;

  teeth: number;
  module: number;
  height: number;
  pressureAngleDeg: number;
  shift: number;
  clearance: number;
  head: number;
  backlash: number;
  undercut: boolean;
  propertiesFromTool: boolean;
  sampleCount: number;

  axleHole: boolean;
  axleHoleDiameter: number;
  offsetHole: boolean;
  offsetHoleDiameter: number;
  offsetHoleOffset: number;
}

export type GearEvent =
  /** Tool activated — wait for a placement. */
  | { kind: "start" }
  /** A planar face was accepted. Arms the tool at `point`. */
  | { kind: "pickFace"; face: unknown; point: [number, number, number] }
  /** Place on an explicit frozen frame (the datum path). */
  | { kind: "pickFrame"; origin: [number, number, number]; axis: [number, number, number]; xDir: [number, number, number] }
  /** Another click on the SAME face — move the gear, stay armed. */
  | { kind: "movePoint"; point: [number, number, number] }
  | { kind: "setRecipe"; recipe: GearRecipe }
  | { kind: "setTeeth"; teeth: number }
  | { kind: "setModule"; module: number }
  | { kind: "setHeight"; height: number }
  | { kind: "setPressureAngle"; angleDeg: number }
  | { kind: "setShift"; shift: number }
  | { kind: "setClearance"; clearance: number }
  | { kind: "setHead"; head: number }
  | { kind: "setBacklash"; backlash: number }
  | { kind: "setUndercut"; undercut: boolean }
  | { kind: "setPropertiesFromTool"; on: boolean }
  | { kind: "setSampleCount"; sampleCount: number }
  | { kind: "setAxleHole"; on: boolean }
  | { kind: "setAxleHoleDiameter"; value: number }
  | { kind: "setOffsetHole"; on: boolean }
  | { kind: "setOffsetHoleDiameter"; value: number }
  | { kind: "setOffsetHoleOffset"; value: number }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface GearStep {
  state: GearFsm;
  effect: ToolEffect;
}

export function gearInit(): GearFsm {
  return {
    phase: "idle",
    recipe: "involuteExternal",
    face: null,
    frame: null,
    point: null,
    teeth: DEFAULT_GEAR_TEETH,
    module: DEFAULT_GEAR_MODULE,
    height: DEFAULT_GEAR_HEIGHT,
    pressureAngleDeg: DEFAULT_GEAR_PRESSURE_ANGLE,
    shift: 0,
    clearance: DEFAULT_GEAR_CLEARANCE,
    head: 0,
    backlash: 0,
    undercut: false,
    propertiesFromTool: false,
    sampleCount: DEFAULT_GEAR_SAMPLE_COUNT,
    axleHole: false,
    axleHoleDiameter: DEFAULT_GEAR_AXLE_HOLE_DIAMETER,
    offsetHole: false,
    offsetHoleDiameter: DEFAULT_GEAR_OFFSET_HOLE_DIAMETER,
    offsetHoleOffset: DEFAULT_GEAR_OFFSET_HOLE_OFFSET,
  };
}

/** Clamp a dimension to the authorable domain; a non-finite value is refused. */
function dim(v: number, fallback: number): number {
  if (!Number.isFinite(v)) return fallback;
  return Math.max(GEAR_MIN_VALUE, v);
}

/** A COEFFICIENT may legitimately be zero or negative (profile shift); only
 *  non-finite is refused. Clamping these to a positive floor the way `dim` does
 *  would silently forbid a negative shift, which is a real gear. */
function coeff(v: number, fallback: number): number {
  return Number.isFinite(v) ? v : fallback;
}

/** True iff `s` is in a phase whose params may be edited. */
function editable(s: GearFsm): boolean {
  return s.phase === "armed";
}

export function gearStep(s: GearFsm, e: GearEvent): GearStep {
  switch (e.kind) {
    case "start":
      // Keeps nothing from a prior arm — a new gear is a new placement.
      return { state: { ...gearInit(), phase: "facePick" }, effect: "none" };

    case "pickFace":
      if (s.phase !== "facePick" && s.phase !== "armed") return { state: s, effect: "none" };
      // Choosing a face CLEARS any frame: the two placement modes are mutually
      // exclusive on the wire, so they are mutually exclusive in state too.
      return {
        state: { ...s, phase: "armed", face: e.face, frame: null, point: e.point },
        effect: "begin",
      };

    case "pickFrame":
      if (s.phase !== "facePick" && s.phase !== "armed") return { state: s, effect: "none" };
      return {
        state: {
          ...s,
          phase: "armed",
          face: null,
          frame: { origin: e.origin, axis: e.axis, xDir: e.xDir },
          // SCHEMA §7.3: with a frame, `point` MUST equal the frame origin.
          point: e.origin,
        },
        effect: "begin",
      };

    case "movePoint":
      if (!editable(s)) return { state: s, effect: "none" };
      // A framed gear's point is pinned to its origin; moving it would break the
      // schema's point==origin rule, so the frame moves with it.
      if (s.frame) {
        return {
          state: { ...s, point: e.point, frame: { ...s.frame, origin: e.point } },
          effect: "update",
        };
      }
      return { state: { ...s, point: e.point }, effect: "update" };

    case "setRecipe":
      if (!editable(s)) return { state: s, effect: "none" };
      if (e.recipe === s.recipe) return { state: s, effect: "none" };
      return { state: { ...s, recipe: e.recipe }, effect: "update" };

    case "setTeeth": {
      if (!editable(s)) return { state: s, effect: "none" };
      const teeth = Number.isFinite(e.teeth)
        ? Math.max(GEAR_MIN_TEETH, Math.round(e.teeth))
        : s.teeth;
      return { state: { ...s, teeth }, effect: "update" };
    }

    case "setModule":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, module: dim(e.module, s.module) }, effect: "update" };

    case "setHeight":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, height: dim(e.height, s.height) }, effect: "update" };

    case "setPressureAngle": {
      if (!editable(s)) return { state: s, effect: "none" };
      // The open interval (0, 90) is the worker's own domain; clamping into it
      // beats emitting a value that will certainly be refused.
      const a = Number.isFinite(e.angleDeg)
        ? Math.min(89.9, Math.max(0.1, e.angleDeg))
        : s.pressureAngleDeg;
      return { state: { ...s, pressureAngleDeg: a }, effect: "update" };
    }

    case "setShift":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, shift: coeff(e.shift, s.shift) }, effect: "update" };

    case "setClearance":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, clearance: Math.max(0, coeff(e.clearance, s.clearance)) },
        effect: "update",
      };

    case "setHead":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, head: coeff(e.head, s.head) }, effect: "update" };

    case "setBacklash":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, backlash: Math.max(0, coeff(e.backlash, s.backlash)) },
        effect: "update",
      };

    case "setUndercut":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, undercut: e.undercut }, effect: "update" };

    case "setPropertiesFromTool":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, propertiesFromTool: e.on }, effect: "update" };

    case "setSampleCount": {
      if (!editable(s)) return { state: s, effect: "none" };
      const n = Number.isFinite(e.sampleCount)
        ? Math.max(2, Math.round(e.sampleCount))
        : s.sampleCount;
      return { state: { ...s, sampleCount: n }, effect: "update" };
    }

    case "setAxleHole":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, axleHole: e.on }, effect: "update" };

    case "setAxleHoleDiameter":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, axleHoleDiameter: dim(e.value, s.axleHoleDiameter) },
        effect: "update",
      };

    case "setOffsetHole":
      if (!editable(s)) return { state: s, effect: "none" };
      return { state: { ...s, offsetHole: e.on }, effect: "update" };

    case "setOffsetHoleDiameter":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, offsetHoleDiameter: dim(e.value, s.offsetHoleDiameter) },
        effect: "update",
      };

    case "setOffsetHoleOffset":
      if (!editable(s)) return { state: s, effect: "none" };
      return {
        state: { ...s, offsetHoleOffset: dim(e.value, s.offsetHoleOffset) },
        effect: "update",
      };

    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      if (s.point === null) return { state: s, effect: "none" };
      if (s.face === null && s.frame === null) return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };

    case "commitFailed":
      // The kernel refused — back to armed with the placement and numbers intact.
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };

    case "settle":
      return { state: gearInit(), effect: "none" };

    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: gearInit(), effect: "cancel" };
  }
}

/**
 * The canonical `GearParams` for an armed gear — the ONE builder both the exact
 * preview and the commit go through (SCHEMA §7.6: "preview callers MUST NOT
 * maintain a second ad-hoc mapper").
 *
 * Emits ONLY the active recipe block and spells every inactive one `null`
 * rather than omitting the key: `null` is what the Rust `Option<...>` reads back
 * as, and spelling it makes "this gear is not a cycloid" an authored fact
 * instead of an omission.
 *
 * Throws when the placement is missing — a gear with nowhere to sit is not a
 * partial op to send, it is a caller bug (the `previewOps.ts` throw convention).
 */
export function gearParamsOf(s: GearFsm): GearParams {
  if (s.point === null) throw new Error("Gear requires a placement point");
  if (s.face === null && s.frame === null) {
    throw new Error("Gear requires a placement face or frame");
  }
  if (s.face !== null && s.frame !== null) {
    // Unreachable through the reducer (each pick clears the other), so this is a
    // guard against a hand-built state rather than a user path.
    throw new Error("Gear placement cannot carry both a face and a frame");
  }

  const involute: InvoluteExternalParams | null =
    s.recipe === "involuteExternal"
      ? {
          teeth: s.teeth,
          module: s.module,
          height: s.height,
          pressureAngleDeg: s.pressureAngleDeg,
          shift: s.shift,
          // Refused by the session/worker in this version; emitted as the only
          // values they accept so the payload shape is already correct.
          helixAngleDeg: 0,
          doubleHelix: false,
          propertiesFromTool: s.propertiesFromTool,
          undercut: s.undercut,
          backlash: s.backlash,
          clearance: s.clearance,
          head: s.head,
          sampleCount: s.sampleCount,
          axleHole: s.axleHole,
          axleHoleDiameter: s.axleHole ? s.axleHoleDiameter : null,
          offsetHole: s.offsetHole,
          offsetHoleDiameter: s.offsetHole ? s.offsetHoleDiameter : null,
          offsetHoleOffset: s.offsetHole ? s.offsetHoleOffset : null,
        }
      : null;

  return {
    recipe: s.recipe,
    placement: {
      face: s.face === null ? null : (s.face as SemanticRef),
      frame: s.frame === null ? null : { ...s.frame },
      point: [s.point[0], s.point[1], s.point[2]],
    },
    involuteExternal: involute,
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

function wireNumber(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function wireBool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}

/**
 * Seed an ARMED `GearFsm` from a record's stored wire params
 * (`getOperationParams`).
 *
 * A gear re-edit restores EVERY field including the frozen placement — the user
 * never re-picks it. Returns `null` when the params carry no usable placement:
 * refusing is the honest outcome, and arming a gear against a placement that is
 * not there would put a body somewhere arbitrary.
 *
 * A bore's dimension comes back `null` when the bore is off (that is the wire
 * contract), so it falls back to the default — correct, because there is nothing
 * authored to restore and switching the bore on should start from a sensible
 * size rather than from zero.
 */
export function gearFsmFromParams(stored: Record<string, unknown> | undefined): GearFsm | null {
  if (!stored) return null;
  const placement = stored.placement;
  if (!placement || typeof placement !== "object") return null;
  const pl = placement as Record<string, unknown>;

  const point = pl.point;
  if (!Array.isArray(point) || point.length !== 3 || !point.every((n) => typeof n === "number")) {
    return null;
  }

  const face = pl.face && typeof pl.face === "object" ? pl.face : null;
  const frameRaw = pl.frame && typeof pl.frame === "object" ? (pl.frame as Record<string, unknown>) : null;
  let frame: GearFsm["frame"] = null;
  if (frameRaw) {
    const triple = (v: unknown): [number, number, number] | null =>
      Array.isArray(v) && v.length === 3 && v.every((n) => typeof n === "number")
        ? [v[0] as number, v[1] as number, v[2] as number]
        : null;
    const origin = triple(frameRaw.origin);
    const axis = triple(frameRaw.axis);
    const xDir = triple(frameRaw.xDir);
    if (!origin || !axis || !xDir) return null;
    frame = { origin, axis, xDir };
  }
  // Exactly one placement mode must be restorable, matching the wire contract.
  if ((face === null) === (frame === null)) return null;

  const base = gearInit();
  const inv =
    stored.involuteExternal && typeof stored.involuteExternal === "object"
      ? (stored.involuteExternal as Record<string, unknown>)
      : null;

  return {
    ...base,
    phase: "armed",
    recipe: "involuteExternal",
    face,
    frame,
    point: [point[0] as number, point[1] as number, point[2] as number],
    teeth: inv ? wireNumber(inv.teeth, base.teeth) : base.teeth,
    module: inv ? (wireScalar(inv.module) ?? base.module) : base.module,
    height: inv ? (wireScalar(inv.height) ?? base.height) : base.height,
    pressureAngleDeg: inv
      ? (wireScalar(inv.pressureAngleDeg) ?? base.pressureAngleDeg)
      : base.pressureAngleDeg,
    shift: inv ? wireNumber(inv.shift, base.shift) : base.shift,
    clearance: inv ? wireNumber(inv.clearance, base.clearance) : base.clearance,
    head: inv ? wireNumber(inv.head, base.head) : base.head,
    backlash: inv ? wireNumber(inv.backlash, base.backlash) : base.backlash,
    undercut: inv ? wireBool(inv.undercut, base.undercut) : base.undercut,
    propertiesFromTool: inv
      ? wireBool(inv.propertiesFromTool, base.propertiesFromTool)
      : base.propertiesFromTool,
    sampleCount: inv ? wireNumber(inv.sampleCount, base.sampleCount) : base.sampleCount,
    axleHole: inv ? wireBool(inv.axleHole, base.axleHole) : base.axleHole,
    axleHoleDiameter: inv
      ? (wireScalar(inv.axleHoleDiameter) ?? base.axleHoleDiameter)
      : base.axleHoleDiameter,
    offsetHole: inv ? wireBool(inv.offsetHole, base.offsetHole) : base.offsetHole,
    offsetHoleDiameter: inv
      ? (wireScalar(inv.offsetHoleDiameter) ?? base.offsetHoleDiameter)
      : base.offsetHoleDiameter,
    offsetHoleOffset: inv
      ? (wireScalar(inv.offsetHoleOffset) ?? base.offsetHoleOffset)
      : base.offsetHoleOffset,
  };
}

/** The chip's headline read-out — mirrors the projection's gear naming. */
export function gearValueText(teeth: number, module: number): string {
  return `${teeth}T m${module}`;
}
