/*
 * Model-tool state machines (PURE reducers) for the three F-WP7 tools. They own
 * ONLY the discrete phase transitions + parameter bookkeeping; the imperative
 * ModelToolController performs the emitted `effect` (begin/update preview,
 * commit, cancel). Keeping the transitions pure lets the whole interaction be
 * driven by unit-tested "pointer scripts" (mirrors the sketch toolMachine).
 *
 * Phase vocabulary mirrors toolStore.InteractionPhase:
 *   armed      — tool primed on a target (handle/chip shown, base preview live)
 *   dragging   — pointer owns the handle/edge (params track the drag)
 *   committing — pointer released; awaiting the exact L2 result before select
 */
import type { BooleanOperation, TransformBodyParams } from "@/ipc/types";
import { EDGE_OP_FLIP_HOLD, EDGE_OP_MIN_VALUE } from "@/tools/preview/filletRadius";
import { WORLD_AXIS } from "@/tools/preview/patternPreview";
import type { Vec3 } from "@/tools/preview/depthProjection";

export type ModelPhase = "idle" | "armed" | "dragging" | "committing";

/** The side-effect the controller runs after a transition. */
export type ToolEffect = "none" | "begin" | "update" | "commit" | "cancel" | "ghost";

// ── Extrude ──────────────────────────────────────────────────────────────────
//
// MODEL-HARDEN Wave 1 — the commit gesture is now professional: a pointer
// RELEASE keeps the tool ARMED (live preview + editable chip), and commit is an
// explicit `confirm` (Enter / chip-✓ / click-away). `commitFailed` returns the
// machine to `armed` so a failed commit never silently loses the user's work.
// `targetPick`/`setBooleanMode`/`pickTarget` are implemented now (pure + tested)
// but only wired to the UI in Wave 2 (single-region boolean modes).

export const DEFAULT_EXTRUDE_DEPTH = 10;

/** Boolean fusion mode the extrude/revolve commit carries (Wave 2 UI). */
export type BooleanMode = "NewBody" | "Add" | "Cut";

/**
 * The boolean default an arm is SEEDED with (SKETCH-ON-FACE HOST-BOOLEAN).
 *
 * A sketch hosted on a model face belongs to that body, so a modeling op off it
 * defaults to MODIFYING the host rather than spawning a new body (the Shapr3D
 * push/pull expectation). The controller resolves the host; the reducers only
 * carry what it decided.
 */
export interface BooleanSeed {
  mode: BooleanMode;
  targetBodyId: string;
  /**
   * EXTRUDE ONLY: while set, the DRAG DIRECTION owns the mode (away from the host
   * = Add, into it = Cut). Any explicit `setBooleanMode` clears it, so a manual
   * override sticks for the rest of the session. Revolve has no direction to read
   * — its arm seeds the mode and ignores this flag.
   */
  auto?: boolean;
}

export type ExtrudePhase =
  | "idle"
  | "armed"
  | "dragging"
  | "targetPick"
  | "facePick"
  | "committing";

/**
 * End condition for one extrude direction (MODEL-OPS W1). The worker has always
 * implemented all of these (`ExtrudeOp.cpp` `effective_distance`) and the wire
 * type has always carried `extrudeMode`; the tool only ever authored the
 * distance-driven ones.
 *
 * `Symmetric` is deliberately NOT a member: it stays the existing `symmetric`
 * toggle (Alt during the drag, or the chip switch) so there is exactly ONE
 * representation of it. The marshaller folds the two back together —
 * `extrudeMode = symmetric ? "Symmetric" : endCondition`.
 */
export type ExtrudeEndCondition = "Blind" | "ThroughAll" | "ToNext" | "ToFace";

/** Whether an end condition is driven by a dragged distance. */
export function isDistanceDriven(end: ExtrudeEndCondition): boolean {
  return end === "Blind";
}

export interface ExtrudeFsm {
  phase: ExtrudePhase;
  depth: number;
  symmetric: boolean;
  hasRegion: boolean;
  booleanMode: BooleanMode;
  /** The body an Add/Cut targets (null for NewBody, or until a target is picked). */
  targetBodyId: string | null;
  /** The drag direction still owns `booleanMode` — see {@link BooleanSeed.auto}. */
  booleanAuto: boolean;
  /** Direction-1 end condition. */
  endCondition: ExtrudeEndCondition;
  /** Draft angle in DEGREES (0 = no draft). */
  draftAngleDeg: number;
  /** Extrude both ways from the sketch plane. Mutually exclusive with `symmetric`. */
  twoDirections: boolean;
  /** Direction-2 end condition (only meaningful when `twoDirections`). */
  endCondition2: ExtrudeEndCondition;
  /** Direction-2 blind distance. */
  depth2: number;
  /**
   * `ToFace` targets, as OPAQUE semantic-ref payloads the controller supplies and
   * the marshaller forwards verbatim. The reducer never inspects them — keeping
   * it free of `@/ipc/types` and trivially testable.
   */
  targetFace: unknown | null;
  targetFace2: unknown | null;
  /** Which direction's `ToFace` target is being picked (null outside `facePick`). */
  facePickFor: 1 | 2 | null;
}

export type ExtrudeEvent =
  | { kind: "arm"; depth?: number; boolean?: BooleanSeed }
  | { kind: "grab" }
  | { kind: "drag"; depth: number; symmetric?: boolean }
  | { kind: "setDepth"; depth: number }
  | { kind: "setSymmetric"; symmetric: boolean }
  | { kind: "release" }
  // Add|Cut with a resolvable auto-target → armed; with `needsPick` → targetPick.
  | { kind: "setBooleanMode"; mode: BooleanMode; targetBodyId?: string | null; needsPick?: boolean }
  | { kind: "pickTarget"; bodyId: string }
  | { kind: "cancelTargetPick" }
  // MODEL-OPS W1 end conditions. `ToFace` enters `facePick` unless a ref is
  // supplied outright (the re-edit path, which already has the stored target).
  | { kind: "setEndCondition"; end: ExtrudeEndCondition; direction?: 1 | 2; targetFace?: unknown }
  | { kind: "setDraftAngle"; deg: number }
  | { kind: "setTwoDirections"; on: boolean }
  | { kind: "setDepth2"; depth: number }
  | { kind: "pickFace"; ref: unknown }
  | { kind: "cancelFacePick" }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface ExtrudeStep {
  state: ExtrudeFsm;
  effect: ToolEffect;
}

export function extrudeInit(): ExtrudeFsm {
  return {
    phase: "idle",
    depth: DEFAULT_EXTRUDE_DEPTH,
    symmetric: false,
    hasRegion: false,
    booleanMode: "NewBody",
    targetBodyId: null,
    booleanAuto: false,
    endCondition: "Blind",
    draftAngleDeg: 0,
    twoDirections: false,
    endCondition2: "Blind",
    depth2: DEFAULT_EXTRUDE_DEPTH,
    targetFace: null,
    targetFace2: null,
    facePickFor: null,
  };
}

/**
 * The mode a drag frame lands on while the arm is still host-seeded: pulling AWAY
 * from the host adds material, pushing INTO it cuts. Depth 0 keeps the current
 * mode (a gesture crossing zero must not blink through a third state), and a
 * SYMMETRIC extrude grows both ways so it has no direction to read.
 */
function autoBooleanMode(s: ExtrudeFsm, depth: number, symmetric: boolean): BooleanMode {
  if (!s.booleanAuto || !s.targetBodyId || symmetric) return s.booleanMode;
  if (!Number.isFinite(depth) || depth === 0) return s.booleanMode;
  return depth < 0 ? "Cut" : "Add";
}

export function extrudeStep(s: ExtrudeFsm, e: ExtrudeEvent): ExtrudeStep {
  switch (e.kind) {
    case "arm": {
      const seed = e.boolean;
      return {
        state: {
          ...extrudeInit(),
          phase: "armed",
          depth: e.depth ?? DEFAULT_EXTRUDE_DEPTH,
          hasRegion: true,
          ...(seed
            ? {
                booleanMode: seed.mode,
                targetBodyId: seed.targetBodyId,
                booleanAuto: seed.auto === true,
              }
            : {}),
        },
        effect: "begin",
      };
    }
    case "grab":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "dragging" }, effect: "none" };
    case "drag": {
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      const symmetric = e.symmetric ?? s.symmetric;
      return {
        state: {
          ...s,
          depth: e.depth,
          symmetric,
          booleanMode: autoBooleanMode(s, e.depth, symmetric),
        },
        effect: "update",
      };
    }
    case "setDepth":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, depth: e.depth }, effect: "update" };
    case "setSymmetric":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      // The worker REJECTS Symmetric combined with two directions
      // (`ExtrudeOp.cpp` "Symmetric is not valid with two directions"), so the two
      // controls are mutually exclusive here rather than failing at commit.
      return {
        state: { ...s, symmetric: e.symmetric, twoDirections: e.symmetric ? false : s.twoDirections },
        effect: "update",
      };
    case "release":
      // The professional gesture: release does NOT commit — it keeps the tool
      // armed with the final depth so the user can tweak / confirm explicitly.
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "update" };
    case "setBooleanMode": {
      if (s.phase !== "armed" && s.phase !== "targetPick") return { state: s, effect: "none" };
      // An EXPLICIT choice (chip segment, whatever the source) ends the host-seeded
      // auto lane for the rest of the session: a manual override must stick, even
      // when the user afterwards drags back through zero.
      if (e.mode === "NewBody") {
        return {
          state: { ...s, phase: "armed", booleanMode: "NewBody", targetBodyId: null, booleanAuto: false },
          effect: "update",
        };
      }
      if (e.needsPick) {
        return {
          state: { ...s, phase: "targetPick", booleanMode: e.mode, targetBodyId: null, booleanAuto: false },
          effect: "none",
        };
      }
      return {
        state: {
          ...s,
          phase: "armed",
          booleanMode: e.mode,
          targetBodyId: e.targetBodyId ?? null,
          booleanAuto: false,
        },
        effect: "update",
      };
    }
    case "pickTarget":
      if (s.phase !== "targetPick") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed", targetBodyId: e.bodyId }, effect: "update" };
    case "cancelTargetPick":
      if (s.phase !== "targetPick") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed", booleanMode: "NewBody", targetBodyId: null }, effect: "update" };
    case "setEndCondition": {
      if (s.phase !== "armed" && s.phase !== "facePick") return { state: s, effect: "none" };
      const dir = e.direction ?? 1;
      const face = e.targetFace ?? null;
      // ToFace needs a target: enter facePick unless the caller already has one
      // (the re-edit path re-arms with the stored ref).
      if (e.end === "ToFace" && face === null) {
        return {
          state: {
            ...s,
            phase: "facePick",
            facePickFor: dir,
            ...(dir === 1 ? { endCondition: "ToFace" as const } : { endCondition2: "ToFace" as const }),
          },
          effect: "none",
        };
      }
      const next: ExtrudeFsm =
        dir === 1
          ? { ...s, phase: "armed", facePickFor: null, endCondition: e.end, targetFace: face }
          : { ...s, phase: "armed", facePickFor: null, endCondition2: e.end, targetFace2: face };
      return { state: next, effect: "update" };
    }
    case "setDraftAngle":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, draftAngleDeg: e.deg }, effect: "update" };
    case "setTwoDirections":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      // Mutually exclusive with `symmetric` — see `setSymmetric`.
      return {
        state: { ...s, twoDirections: e.on, symmetric: e.on ? false : s.symmetric },
        effect: "update",
      };
    case "setDepth2":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, depth2: e.depth }, effect: "update" };
    case "pickFace": {
      if (s.phase !== "facePick") return { state: s, effect: "none" };
      const next: ExtrudeFsm =
        s.facePickFor === 2
          ? { ...s, phase: "armed", facePickFor: null, endCondition2: "ToFace", targetFace2: e.ref }
          : { ...s, phase: "armed", facePickFor: null, endCondition: "ToFace", targetFace: e.ref };
      return { state: next, effect: "update" };
    }
    case "cancelFacePick": {
      if (s.phase !== "facePick") return { state: s, effect: "none" };
      // Abandoning the pick falls back to Blind for THAT direction — never leave
      // a ToFace armed with no target, which the worker would reject at commit.
      const next: ExtrudeFsm =
        s.facePickFor === 2
          ? { ...s, phase: "armed", facePickFor: null, endCondition2: "Blind", targetFace2: null }
          : { ...s, phase: "armed", facePickFor: null, endCondition: "Blind", targetFace: null };
      return { state: next, effect: "update" };
    }
    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "commitFailed":
      // The commit threw / returned no body — back to armed so the controller can
      // re-arm the preview (work preserved) and surface the reason.
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };
    case "settle":
      return { state: extrudeInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: extrudeInit(), effect: "cancel" };
  }
}

// ── Edge op (Fillet / Chamfer) ───────────────────────────────────────────────
//
// FILLET-CHAMFER-UNIFY: ONE machine drives both edge ops. The DRAG DIRECTION
// picks which one while the arm is direction-driven (`auto`) — away from the
// body rounds it, into it bevels — and the [Fillet|Chamfer] chip segments are
// the explicit override (which locks the type for the session). `auto` is armed
// ONLY where the direction is honest: the bisector tier. The bbox proxy points
// INTO material on a concave edge, so an auto flip there would silently author
// the op the user did not ask for; those arms seed `auto:false` and the chip is
// the type control (see tools/preview/edgeDirection.ts).
//
// EDGE-OP PREVIEW wave: the edge ops join the professional commit gesture the
// extrude/revolve lanes already use. A pointer RELEASE keeps the tool ARMED (the
// live kernel preview + the editable chip stay), and commit is an EXPLICIT
// `confirm` (Enter / chip-✓). `commitFailed` returns the machine to `armed` so an
// OCCT-refused radius never loses the user's edge selection.
//
// Click-away is deliberately NOT part of this gesture for fillet/shell: the armed
// tool claims EVERY left press in the viewport as a value drag (there is no
// handle hit-test to distinguish "grabbed the gizmo" from "clicked away"), so a
// click-away confirm would be indistinguishable from a zero-distance drag.

export const DEFAULT_FILLET_RADIUS = 2;

/** A chamfer distance is not a fillet radius, hence its own default. */
export const DEFAULT_CHAMFER_DISTANCE = 1;

/**
 * Which edge op the ONE edge lane is authoring (FILLET-CHAMFER-UNIFY). Fillet
 * and Chamfer are the same gesture over the same `FilletChamferParams` (SCHEMA
 * §7.3), distinguished only by `opType`/`mode` at the wire — so one machine
 * with a discriminator, not two byte-identical reducers to drift apart.
 */
export type EdgeOpKind = "Fillet" | "Chamfer";

export interface FilletFsm {
  phase: ModelPhase;
  /** MAGNITUDE of the radius/distance — always ≥ EDGE_OP_MIN_VALUE, never signed.
   *  The drag's SIGN picks {@link edgeOp}; it never reaches the wire. */
  radius: number;
  edgeCount: number;
  /** The op this arm commits (drives `opType`, so a change is a session swap). */
  edgeOp: EdgeOpKind;
  /** The DRAG DIRECTION still owns `edgeOp` — see {@link autoEdgeOp}. Any explicit
   *  `setEdgeOp` clears it, so a manual override sticks for the rest of the session. */
  auto: boolean;
  /** Whether the user has authored a size yet (drag or typed). A pristine arm
   *  reseeds to the picked op's default on `setEdgeOp`; a touched one keeps the
   *  user's number. */
  touched: boolean;
}

export type FilletEvent =
  | {
      kind: "arm";
      edgeCount: number;
      radius?: number;
      edgeOp?: EdgeOpKind;
      auto?: boolean;
      /** Seed the arm as already-authored. A RE-EDIT passes `true`: the seeded
       *  size IS the committed size, i.e. the user's own number, so a later
       *  segment pick must keep it instead of reseeding to a tool default. */
      touched?: boolean;
    }
  | { kind: "grabEdge" }
  /** `signed` is the raw drag projection: positive = away from the body. */
  | { kind: "drag"; signed: number }
  | { kind: "setRadius"; radius: number }
  | { kind: "setEdgeOp"; edgeOp: EdgeOpKind }
  | { kind: "release" }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface FilletStep {
  state: FilletFsm;
  effect: ToolEffect;
}

export function filletInit(): FilletFsm {
  return {
    phase: "idle",
    radius: DEFAULT_FILLET_RADIUS,
    edgeCount: 0,
    edgeOp: "Fillet",
    auto: false,
    touched: false,
  };
}

/**
 * The op a drag frame lands on while the arm is still direction-driven: dragging
 * AWAY from the body rounds it (Fillet), pushing INTO it bevels (Chamfer) — the
 * Shapr3D convention. Mirror of {@link autoBooleanMode} for the extrude lane.
 *
 * `EDGE_OP_FLIP_HOLD` is a HYSTERESIS band, not a dead zone around zero: a
 * gesture crossing zero must not strobe the authored type, because every flip
 * tears the kernel preview session down and reopens it under the new `opType`
 * (`beginPreview` freezes it). A non-finite value keeps the current type for
 * the same reason a zero does.
 */
function autoEdgeOp(s: FilletFsm, signed: number): EdgeOpKind {
  if (!s.auto) return s.edgeOp;
  if (!Number.isFinite(signed)) return s.edgeOp;
  if (Math.abs(signed) < EDGE_OP_FLIP_HOLD) return s.edgeOp;
  return signed < 0 ? "Chamfer" : "Fillet";
}

export function filletStep(s: FilletFsm, e: FilletEvent): FilletStep {
  switch (e.kind) {
    case "arm":
      if (e.edgeCount <= 0) return { state: filletInit(), effect: "none" };
      return {
        state: {
          ...filletInit(),
          phase: "armed",
          radius: e.radius ?? DEFAULT_FILLET_RADIUS,
          edgeCount: e.edgeCount,
          edgeOp: e.edgeOp ?? "Fillet",
          auto: e.auto === true,
          touched: e.touched === true,
        },
        effect: "begin",
      };
    case "grabEdge":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "dragging" }, effect: "none" };
    case "drag": {
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      if (s.auto) {
        return {
          state: {
            ...s,
            radius: Math.max(EDGE_OP_MIN_VALUE, Math.abs(e.signed)),
            edgeOp: autoEdgeOp(s, e.signed),
            touched: true,
          },
          effect: "update",
        };
      }
      // A LOCKED type projects the drag onto its own half-line rather than taking
      // the magnitude: dragging a locked Chamfer the "wrong" way must bottom out at
      // the minimum and STAY there, not V-bounce back up as |signed| would.
      const sign = s.edgeOp === "Chamfer" ? -1 : 1;
      return {
        state: { ...s, radius: Math.max(EDGE_OP_MIN_VALUE, sign * e.signed), touched: true },
        effect: "update",
      };
    }
    case "setRadius":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      // A TYPED value never re-types the op (TODO.md:32 precedent): the number is a
      // magnitude, and the sign convention belongs to the gesture, not the keyboard.
      return {
        state: { ...s, radius: Math.max(EDGE_OP_MIN_VALUE, Math.abs(e.radius)), touched: true },
        effect: "update",
      };
    case "setEdgeOp": {
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      // An EXPLICIT choice (chip segment) ends the direction-driven lane for the
      // rest of the session — a manual override must stick even when the user
      // afterwards drags back through zero (host-boolean precedent).
      // PRISTINE RESEED: an untouched arm takes the picked op's own default; once
      // the user has authored a size, that size is theirs and survives the swap.
      const radius = s.touched
        ? s.radius
        : e.edgeOp === "Chamfer"
          ? DEFAULT_CHAMFER_DISTANCE
          : DEFAULT_FILLET_RADIUS;
      return { state: { ...s, edgeOp: e.edgeOp, auto: false, radius }, effect: "update" };
    }
    case "release":
      // Release does NOT commit — it keeps the tool armed at the dragged size so
      // the kernel preview stays live and the user confirms explicitly.
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "update" };
    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "commitFailed":
      // The kernel refused the op — back to armed so the edge selection + size
      // survive and the controller can re-arm the preview and name the reason.
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };
    case "settle":
      return { state: filletInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: filletInit(), effect: "cancel" };
  }
}

// Chamfer has NO reducer of its own — it is a value of `FilletFsm.edgeOp` above.
// The worker has always implemented it (`FilletChamferOp.cpp execute_chamfer`,
// sharing `FilletChamferParams` with Fillet and distinguished by `mode`); only
// the tool was missing.

// ── Revolve ──────────────────────────────────────────────────────────────────
//
// Revolve adds an `axisPick` phase before the angle drag: the user first clicks a
// sketch LINE to set the axis of revolution, then the tool arms at 360°. A pointer
// RELEASE from an angle drag keeps the tool armed (MODEL-HARDEN Wave 1); commit is
// the explicit `confirm` (Enter / chip-✓ / click-away). The old `quickCommit`
// (plain-click → 360°) is DELETED — the controller now confirms an armed revolve
// on click-away, which reproduces the old muscle memory at the 360° default.

export const DEFAULT_REVOLVE_ANGLE = 360;

/** A confirm below this (degrees) is a degenerate revolve — the machine ignores it. */
export const REVOLVE_MIN_ANGLE = 0.5;

export type RevolvePhase = "idle" | "axisPick" | "armed" | "dragging" | "targetPick" | "committing";

export interface RevolveFsm {
  phase: RevolvePhase;
  /** Revolution angle in DEGREES (0–360). */
  angle: number;
  hasRegion: boolean;
  /** The chosen sketch-line axis id (null until an axis is picked). */
  axisLineId: string | null;
  booleanMode: BooleanMode;
  /** The body an Add/Cut targets (null for NewBody, or until a target is picked). */
  targetBodyId: string | null;
}

export type RevolveEvent =
  | {
      kind: "arm";
      angle?: number;
      hasRegion?: boolean;
      hasAxis?: boolean;
      axisLineId?: string | null;
      boolean?: BooleanSeed;
    }
  | { kind: "pickAxis"; lineId: string; valid: boolean }
  | { kind: "resetAxis" }
  | { kind: "grab" }
  | { kind: "drag"; angle: number }
  | { kind: "setAngle"; angle: number }
  | { kind: "release" }
  | { kind: "setBooleanMode"; mode: BooleanMode; targetBodyId?: string | null; needsPick?: boolean }
  | { kind: "pickTarget"; bodyId: string }
  | { kind: "cancelTargetPick" }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface RevolveStep {
  state: RevolveFsm;
  effect: ToolEffect;
}

export function revolveInit(): RevolveFsm {
  return {
    phase: "idle",
    angle: DEFAULT_REVOLVE_ANGLE,
    hasRegion: false,
    axisLineId: null,
    booleanMode: "NewBody",
    targetBodyId: null,
  };
}

export function revolveStep(s: RevolveFsm, e: RevolveEvent): RevolveStep {
  switch (e.kind) {
    case "arm": {
      if (e.hasRegion === false) return { state: revolveInit(), effect: "none" };
      const angle = e.angle ?? DEFAULT_REVOLVE_ANGLE;
      // A host-seeded arm defaults to modifying its host body. There is no
      // direction logic here (a revolve sweeps around an axis, it does not push
      // into or away from the host), so `BooleanSeed.auto` is deliberately unread
      // — the seeded mode holds until the chip changes it.
      const base = {
        angle,
        hasRegion: true,
        booleanMode: e.boolean?.mode ?? ("NewBody" as BooleanMode),
        targetBodyId: e.boolean?.targetBodyId ?? null,
      };
      // A re-edit seeds an existing axis (param-only edit) → skip axis-pick.
      if (e.hasAxis) {
        return { state: { ...base, phase: "armed", axisLineId: e.axisLineId ?? null }, effect: "begin" };
      }
      return { state: { ...base, phase: "axisPick", axisLineId: null }, effect: "none" };
    }
    case "pickAxis":
      if (s.phase !== "axisPick") return { state: s, effect: "none" };
      if (!e.valid) return { state: s, effect: "none" }; // reject: stay in axis-pick
      return { state: { ...s, phase: "armed", axisLineId: e.lineId }, effect: "begin" };
    case "resetAxis":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, phase: "axisPick", axisLineId: null }, effect: "none" };
    case "grab":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "dragging" }, effect: "none" };
    case "drag":
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, angle: e.angle }, effect: "update" };
    case "setAngle":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, angle: e.angle }, effect: "update" };
    case "release":
      // Release keeps the tool armed at the final angle (no implicit commit).
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "update" };
    case "setBooleanMode": {
      if (s.phase !== "armed" && s.phase !== "targetPick") return { state: s, effect: "none" };
      if (e.mode === "NewBody") {
        return { state: { ...s, phase: "armed", booleanMode: "NewBody", targetBodyId: null }, effect: "update" };
      }
      if (e.needsPick) {
        return { state: { ...s, phase: "targetPick", booleanMode: e.mode, targetBodyId: null }, effect: "none" };
      }
      return {
        state: { ...s, phase: "armed", booleanMode: e.mode, targetBodyId: e.targetBodyId ?? null },
        effect: "update",
      };
    }
    case "pickTarget":
      if (s.phase !== "targetPick") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed", targetBodyId: e.bodyId }, effect: "update" };
    case "cancelTargetPick":
      if (s.phase !== "targetPick") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed", booleanMode: "NewBody", targetBodyId: null }, effect: "update" };
    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      // Degenerate at ~0°: no transition — the controller nudges "Set a non-zero angle".
      if (Math.abs(s.angle) < REVOLVE_MIN_ANGLE) return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "commitFailed":
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };
    case "settle":
      return { state: revolveInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: revolveInit(), effect: "cancel" };
  }
}

// ── Region multi-select (Wave 2 wiring; pure + tested now) ────────────────────
//
// A finished sketch with several closed regions lets the user select N of them,
// each committed as a separate op (Wave 2). This is the pure ordered/dedup
// selection reducer the controller will drive; exported + tested now so the
// Wave 2 controller wiring lands against a proven core.

export interface RegionSelectState {
  active: boolean;
  /** Selected region ids, in click order, deduped. */
  selected: string[];
}

export type RegionSelectEvent =
  | { kind: "enter" }
  | { kind: "toggle"; id: string }
  | { kind: "confirm" }
  | { kind: "cancel" };

export interface RegionSelectStep {
  state: RegionSelectState;
  /** `confirm` fires only with ≥1 region selected; `cancel` always resets. */
  effect: "none" | "confirm" | "cancel";
}

export function regionSelectInit(): RegionSelectState {
  return { active: false, selected: [] };
}

export function regionSelectStep(s: RegionSelectState, e: RegionSelectEvent): RegionSelectStep {
  switch (e.kind) {
    case "enter":
      return { state: { active: true, selected: [] }, effect: "none" };
    case "toggle": {
      if (!s.active) return { state: s, effect: "none" };
      const has = s.selected.includes(e.id);
      const selected = has ? s.selected.filter((x) => x !== e.id) : [...s.selected, e.id];
      return { state: { ...s, selected }, effect: "none" };
    }
    case "confirm":
      if (!s.active || s.selected.length === 0) return { state: s, effect: "none" };
      return { state: s, effect: "confirm" };
    case "cancel":
      return { state: regionSelectInit(), effect: "cancel" };
  }
}

// ── Boolean ──────────────────────────────────────────────────────────────────

export type BooleanPhase = "idle" | "pickTool" | "armed" | "committing";

export interface BooleanFsm {
  phase: BooleanPhase;
  op: BooleanOperation;
  targetBodyId: string | null;
  toolBodyId: string | null;
}

export type BooleanEvent =
  | { kind: "start"; targetBodyId: string }
  | { kind: "pickTool"; toolBodyId: string }
  | { kind: "setOp"; op: BooleanOperation }
  | { kind: "apply" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface BooleanStep {
  state: BooleanFsm;
  effect: ToolEffect;
}

export function booleanInit(): BooleanFsm {
  return { phase: "idle", op: "Union", targetBodyId: null, toolBodyId: null };
}

export function booleanStep(s: BooleanFsm, e: BooleanEvent): BooleanStep {
  switch (e.kind) {
    case "start":
      return {
        state: { phase: "pickTool", op: "Union", targetBodyId: e.targetBodyId, toolBodyId: null },
        effect: "none",
      };
    case "pickTool":
      if (s.phase !== "pickTool" || e.toolBodyId === s.targetBodyId) return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed", toolBodyId: e.toolBodyId }, effect: "ghost" };
    case "setOp":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, op: e.op }, effect: "none" };
    case "apply":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "settle":
      return { state: booleanInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: booleanInit(), effect: "cancel" };
  }
}

// ── Shell ────────────────────────────────────────────────────────────────────
//
// Shell mirrors Fillet: it arms from a FACE selection (the selected faces are the
// removed/open faces), then a vertical drag (or the mm chip) sets the wall
// thickness. Release keeps the tool ARMED; commit is the explicit `confirm`
// (Enter / chip-✓) — see the Fillet block for why click-away is excluded.
//
// There is still no cheap L1 preview (hollowing needs OCCT); the visible preview
// is the kernel's exact candidate from the shared PreviewOp lane.

export const DEFAULT_SHELL_THICKNESS = 2;

export interface ShellFsm {
  phase: ModelPhase;
  thickness: number;
  faceCount: number;
}

export type ShellEvent =
  | { kind: "arm"; faceCount: number; thickness?: number }
  | { kind: "grab" }
  | { kind: "drag"; thickness: number }
  | { kind: "setThickness"; thickness: number }
  | { kind: "release" }
  | { kind: "confirm" }
  | { kind: "commitFailed" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface ShellStep {
  state: ShellFsm;
  effect: ToolEffect;
}

export function shellInit(): ShellFsm {
  return { phase: "idle", thickness: DEFAULT_SHELL_THICKNESS, faceCount: 0 };
}

export function shellStep(s: ShellFsm, e: ShellEvent): ShellStep {
  switch (e.kind) {
    case "arm":
      if (e.faceCount <= 0) return { state: shellInit(), effect: "none" };
      return {
        state: { phase: "armed", thickness: e.thickness ?? DEFAULT_SHELL_THICKNESS, faceCount: e.faceCount },
        effect: "begin",
      };
    case "grab":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "dragging" }, effect: "none" };
    case "drag":
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, thickness: e.thickness }, effect: "update" };
    case "setThickness":
      if (s.phase !== "armed" && s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, thickness: e.thickness }, effect: "update" };
    case "release":
      // Release keeps the tool armed at the dragged thickness (no implicit commit).
      if (s.phase !== "dragging") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "update" };
    case "confirm":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "commitFailed":
      if (s.phase !== "committing") return { state: s, effect: "none" };
      return { state: { ...s, phase: "armed" }, effect: "none" };
    case "settle":
      return { state: shellInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: shellInit(), effect: "cancel" };
  }
}

// ── Linear pattern ─────────────────────────────────────────────────────────
//
// LinearPattern arms with a BODY selected; the user picks a world axis (X/Y/Z
// chip), an instance count (2–12 stepper) and spacing (mm chip). A live GHOST of
// translated body clones renders as any chip changes; Apply commits. There is no
// drag-to-commit — orbit stays free so the 3D ghost can be inspected (the
// spacing DRAG affordance is deferred; the chip covers it — see the WP report).

export type PatternAxis = "X" | "Y" | "Z";

export type ConfigPhase = "idle" | "armed" | "committing";

export interface LinearPatternFsm {
  phase: ConfigPhase;
  axis: PatternAxis;
  count: number;
  spacing: number;
  bodyId: string | null;
}

export type LinearPatternEvent =
  | { kind: "arm"; bodyId?: string; axis?: PatternAxis; count?: number; spacing?: number }
  | { kind: "setAxis"; axis: PatternAxis }
  | { kind: "setCount"; count: number }
  | { kind: "setSpacing"; spacing: number }
  | { kind: "apply" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface LinearPatternStep {
  state: LinearPatternFsm;
  effect: ToolEffect;
}

export const DEFAULT_PATTERN_COUNT = 3;
export const DEFAULT_LINEAR_SPACING = 20;

function clampCount(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_PATTERN_COUNT;
  return Math.max(2, Math.min(12, Math.round(n)));
}

export function linearPatternInit(): LinearPatternFsm {
  return { phase: "idle", axis: "X", count: DEFAULT_PATTERN_COUNT, spacing: DEFAULT_LINEAR_SPACING, bodyId: null };
}

export function linearPatternStep(s: LinearPatternFsm, e: LinearPatternEvent): LinearPatternStep {
  switch (e.kind) {
    case "arm":
      if (!e.bodyId) return { state: linearPatternInit(), effect: "none" };
      return {
        state: {
          phase: "armed",
          axis: e.axis ?? "X",
          count: clampCount(e.count ?? DEFAULT_PATTERN_COUNT),
          spacing: e.spacing ?? DEFAULT_LINEAR_SPACING,
          bodyId: e.bodyId,
        },
        effect: "ghost",
      };
    case "setAxis":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, axis: e.axis }, effect: "ghost" };
    case "setCount":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, count: clampCount(e.count) }, effect: "ghost" };
    case "setSpacing":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, spacing: e.spacing }, effect: "ghost" };
    case "apply":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "settle":
      return { state: linearPatternInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: linearPatternInit(), effect: "cancel" };
  }
}

// ── Circular pattern ─────────────────────────────────────────────────────────
//
// Same shape as LinearPattern but the axis defaults to world Z and the numeric
// param is a TOTAL sweep angle (degrees). Ghost clones are rotated about the axis.

export interface CircularPatternFsm {
  phase: ConfigPhase;
  axis: PatternAxis;
  count: number;
  angle: number;
  bodyId: string | null;
}

export type CircularPatternEvent =
  | { kind: "arm"; bodyId?: string; axis?: PatternAxis; count?: number; angle?: number }
  | { kind: "setAxis"; axis: PatternAxis }
  | { kind: "setCount"; count: number }
  | { kind: "setAngle"; angle: number }
  | { kind: "apply" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface CircularPatternStep {
  state: CircularPatternFsm;
  effect: ToolEffect;
}

export const DEFAULT_CIRCULAR_ANGLE = 360;

function clampCircularAngle(a: number): number {
  if (!Number.isFinite(a)) return DEFAULT_CIRCULAR_ANGLE;
  return Math.max(1, Math.min(360, a));
}

export function circularPatternInit(): CircularPatternFsm {
  return { phase: "idle", axis: "Z", count: DEFAULT_PATTERN_COUNT, angle: DEFAULT_CIRCULAR_ANGLE, bodyId: null };
}

export function circularPatternStep(s: CircularPatternFsm, e: CircularPatternEvent): CircularPatternStep {
  switch (e.kind) {
    case "arm":
      if (!e.bodyId) return { state: circularPatternInit(), effect: "none" };
      return {
        state: {
          phase: "armed",
          axis: e.axis ?? "Z",
          count: clampCount(e.count ?? DEFAULT_PATTERN_COUNT),
          angle: clampCircularAngle(e.angle ?? DEFAULT_CIRCULAR_ANGLE),
          bodyId: e.bodyId,
        },
        effect: "ghost",
      };
    case "setAxis":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, axis: e.axis }, effect: "ghost" };
    case "setCount":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, count: clampCount(e.count) }, effect: "ghost" };
    case "setAngle":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, angle: clampCircularAngle(e.angle) }, effect: "ghost" };
    case "apply":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "settle":
      return { state: circularPatternInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: circularPatternInit(), effect: "cancel" };
  }
}

// ── Mirror body ──────────────────────────────────────────────────────────────
//
// MirrorBody arms with a BODY selected; the user picks a world mirror plane
// (XY/XZ/YZ chip). A ghost of the mirrored clone renders; Apply commits.
// (Datum-plane picking is deferred to a later WP — see the WP report.)

export type MirrorPlane = "XY" | "XZ" | "YZ";

export interface MirrorFsm {
  phase: ConfigPhase;
  plane: MirrorPlane;
  bodyId: string | null;
}

export type MirrorEvent =
  | { kind: "arm"; bodyId?: string; plane?: MirrorPlane }
  | { kind: "setPlane"; plane: MirrorPlane }
  | { kind: "apply" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface MirrorStep {
  state: MirrorFsm;
  effect: ToolEffect;
}

export function mirrorInit(): MirrorFsm {
  return { phase: "idle", plane: "XY", bodyId: null };
}

export function mirrorStep(s: MirrorFsm, e: MirrorEvent): MirrorStep {
  switch (e.kind) {
    case "arm":
      if (!e.bodyId) return { state: mirrorInit(), effect: "none" };
      return { state: { phase: "armed", plane: e.plane ?? "XY", bodyId: e.bodyId }, effect: "ghost" };
    case "setPlane":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, plane: e.plane }, effect: "ghost" };
    case "apply":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "settle":
      return { state: mirrorInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: mirrorInit(), effect: "cancel" };
  }
}

// ── Transform body (WP-B W1; SCHEMA §7.3 `TransformBody`) ────────────────────
//
// Arms with a BODY SELECTION (one or many). The chip cluster is
// `[Move|Rotate] [X|Y|Z] [value] [✓] [✕]`: the mode picks WHAT the number means,
// the axis picks WHICH component it writes, and the value is a live view of that
// one component of the record's CUMULATIVE params.
//
// The state below is the record's canonical form, not a delta. A placement is ONE
// re-edited record per intent (the fold rule), so `translate` / `angleDeg` are
// always the full stored values and typing REPLACES the addressed component —
// there is deliberately NO incremental composition math in V1 (a second gesture
// after a 30mm move types the new absolute, not "+20"). Holding the whole vector
// rather than a scalar is what makes an axis switch honest: moving X 30 then
// switching to Y shows the stored dy (0), and committing keeps dx = 30.
//
// `center` is FROZEN at first authoring (SCHEMA §7.3) — seeded from the targets'
// combined bbox centre on a fresh arm, from the stored params on a fold/re-edit,
// and never re-derived, so repeated edits cannot drift the pivot.

export type TransformMode = "move" | "rotate";

export interface TransformFsm {
  phase: ConfigPhase;
  mode: TransformMode;
  /** The active axis: the translate COMPONENT in move mode, the rotation axis in rotate mode. */
  axis: PatternAxis;
  /** Cumulative world translation (the record's canonical `translate`). */
  translate: Vec3;
  /** Cumulative rotation angle in DEGREES about `rotAxis`. */
  angleDeg: number;
  /** The stored rotation axis — kept separate so a move never disturbs the rotation. */
  rotAxis: PatternAxis;
  /** The bodies being placed; mirrors `params.targets` in order. */
  targets: string[];
  /** The frozen pivot (world). */
  center: Vec3;
  /** `true` places COPIES and preserves the sources. No W1 UI authors it (see the WP note). */
  copy: boolean;
}

/** The full stored shape a fold / re-edit seeds (everything but `phase`). */
export interface TransformSeed {
  mode?: TransformMode;
  axis?: PatternAxis;
  translate?: Vec3;
  angleDeg?: number;
  rotAxis?: PatternAxis;
  center?: Vec3;
  copy?: boolean;
}

/**
 * Which gizmo handle a drag grabbed (WP-B W2). A PLANE is named by its NORMAL —
 * the "Z" quad translates in XY — so every handle is keyed by one axis letter.
 */
export interface TransformGrab {
  kind: "axis" | "plane" | "ring";
  axis: PatternAxis;
}

export type TransformEvent =
  | ({ kind: "arm"; targets: string[] } & TransformSeed)
  /** Re-seed an ALREADY-armed placement (a re-edit's stored params land async). */
  | ({ kind: "seed" } & TransformSeed)
  | { kind: "setMode"; mode: TransformMode }
  | { kind: "setAxis"; axis: PatternAxis }
  | { kind: "setValue"; value: number }
  /** A gizmo handle was grabbed: the handle DECIDES the mode + axis (W2). */
  | { kind: "grab"; grab: TransformGrab; copy?: boolean }
  /** A drag frame wrote the whole translation (a plane drag moves two axes). */
  | { kind: "dragTranslate"; translate: Vec3 }
  /** A ring drag frame wrote the angle; the ring's axis is already claimed. */
  | { kind: "dragAngle"; angleDeg: number }
  | { kind: "setCopy"; copy: boolean }
  | { kind: "apply" }
  | { kind: "settle" }
  | { kind: "cancel" };

export interface TransformStep {
  state: TransformFsm;
  effect: ToolEffect;
}

export function transformInit(): TransformFsm {
  return {
    phase: "idle",
    mode: "move",
    axis: "X",
    translate: [0, 0, 0],
    angleDeg: 0,
    rotAxis: "Z",
    targets: [],
    center: [0, 0, 0],
    copy: false,
  };
}

/** Index of a world axis into a Vec3 / the translate triple. */
export function axisIndex(axis: PatternAxis): 0 | 1 | 2 {
  return axis === "X" ? 0 : axis === "Y" ? 1 : 2;
}

/**
 * The number the chip shows for the CURRENT mode + axis. In rotate mode an axis
 * that is not the stored rotation axis reads 0 — there is one rotation, and
 * asking about a different axis honestly has no angle yet.
 */
export function transformValue(s: TransformFsm): number {
  if (s.mode === "move") return s.translate[axisIndex(s.axis)];
  return s.axis === s.rotAxis ? s.angleDeg : 0;
}

/** Merge a seed over a state, dropping the fields the caller left undefined. */
function applySeed(s: TransformFsm, e: TransformSeed): TransformFsm {
  return {
    ...s,
    mode: e.mode ?? s.mode,
    axis: e.axis ?? s.axis,
    translate: e.translate ? [...e.translate] : s.translate,
    angleDeg: e.angleDeg ?? s.angleDeg,
    rotAxis: e.rotAxis ?? s.rotAxis,
    center: e.center ? [...e.center] : s.center,
    copy: e.copy ?? s.copy,
  };
}

/**
 * The mode + axis a grabbed handle claims. The gizmo is the chip's peer, not its
 * master: grabbing a ring IS choosing Rotate about that axis, so the FSM records
 * it and the chip segments re-render from the FSM — one writer, no drift.
 *
 * A PLANE quad writes two components at once but the chip can only show one, so
 * it keeps the axis already selected when that axis lies IN the plane (the user's
 * standing choice of which number to watch) and otherwise falls back to the first
 * in-plane axis. Never the normal: that component is exactly the one a plane drag
 * cannot move, so showing it would peg the chip at a constant.
 */
function grabbedModeAxis(s: TransformFsm, grab: TransformGrab): { mode: TransformMode; axis: PatternAxis } {
  if (grab.kind === "ring") return { mode: "rotate", axis: grab.axis };
  if (grab.kind === "axis") return { mode: "move", axis: grab.axis };
  const inPlane = (["X", "Y", "Z"] as PatternAxis[]).filter((a) => a !== grab.axis);
  return { mode: "move", axis: inPlane.includes(s.axis) ? s.axis : inPlane[0] };
}

export function transformStep(s: TransformFsm, e: TransformEvent): TransformStep {
  switch (e.kind) {
    case "arm": {
      if (e.targets.length === 0) return { state: transformInit(), effect: "none" };
      const armed = applySeed(transformInit(), e);
      return { state: { ...armed, phase: "armed", targets: [...e.targets] }, effect: "ghost" };
    }
    case "seed":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: applySeed(s, e), effect: "ghost" };
    case "setMode":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, mode: e.mode }, effect: "ghost" };
    case "setAxis":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, axis: e.axis }, effect: "ghost" };
    case "setValue": {
      if (s.phase !== "armed") return { state: s, effect: "none" };
      const v = Number.isFinite(e.value) ? e.value : 0;
      if (s.mode === "rotate") {
        // Typing an angle also CLAIMS the axis: the record carries one rotation,
        // so authoring about Y is what makes Y the rotation axis.
        return { state: { ...s, angleDeg: v, rotAxis: s.axis }, effect: "ghost" };
      }
      const translate: Vec3 = [...s.translate];
      translate[axisIndex(s.axis)] = v;
      return { state: { ...s, translate }, effect: "ghost" };
    }
    case "grab": {
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, ...grabbedModeAxis(s, e.grab), copy: e.copy ?? s.copy }, effect: "ghost" };
    }
    case "dragTranslate": {
      if (s.phase !== "armed") return { state: s, effect: "none" };
      if (!e.translate.every((c) => Number.isFinite(c))) return { state: s, effect: "none" };
      return { state: { ...s, translate: [...e.translate] }, effect: "ghost" };
    }
    case "dragAngle": {
      if (s.phase !== "armed") return { state: s, effect: "none" };
      if (!Number.isFinite(e.angleDeg)) return { state: s, effect: "none" };
      // Same rule as typing an angle: authoring about an axis CLAIMS it.
      return { state: { ...s, angleDeg: e.angleDeg, rotAxis: s.axis }, effect: "ghost" };
    }
    case "setCopy":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, copy: e.copy }, effect: "ghost" };
    case "apply":
      if (s.phase !== "armed") return { state: s, effect: "none" };
      return { state: { ...s, phase: "committing" }, effect: "commit" };
    case "settle":
      return { state: transformInit(), effect: "none" };
    case "cancel":
      if (s.phase === "idle") return { state: s, effect: "none" };
      return { state: transformInit(), effect: "cancel" };
  }
}

/** True iff the placement is the identity — a legal, geometry-free no-op. */
export function isIdentityPlacement(s: TransformFsm): boolean {
  return s.translate.every((c) => c === 0) && s.angleDeg === 0;
}

/**
 * The canonical `TransformBodyParams` for an armed placement. The rotation always
 * carries the FROZEN centre (even for a pure translation, where the pivot is
 * unused but must survive the round-trip so the next re-edit recomposes against
 * the same point).
 */
export function transformParamsOf(s: TransformFsm): TransformBodyParams {
  return {
    targets: [...s.targets],
    translate: [s.translate[0], s.translate[1], s.translate[2]],
    rotate: {
      center: [s.center[0], s.center[1], s.center[2]],
      axis: [...WORLD_AXIS[s.rotAxis]],
      angleDeg: s.angleDeg,
    },
    copy: s.copy,
  };
}
