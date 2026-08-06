/*
 * Tool-chip store (F-WP7 + M6b) — the small bridge that lets the imperative
 * ModelToolController drive the React overlay chips (extrude depth, fillet
 * radius, revolve angle, boolean op, and the M6b shell / pattern / mirror chips)
 * without owning DOM. The controller sets the descriptor + callbacks; a React
 * layer (ModelToolChips) renders the chip and registers its node with the
 * engine's HTML overlay so it tracks a world anchor each frame.
 *
 * `value`/`count`/`axis`/`plane`/`op` update live during a chip edit (cheap: one
 * tiny input re-renders); the `worldPos` anchor is set once per arm and stays
 * put, so no per-frame churn.
 */
import { createStore, useStore } from "zustand";
import type { BooleanOperation, HoleType, OffsetDistanceType } from "@/ipc/types";
import type { HoleFit } from "@/tools/modelTools/holeStandards";
import {
  DEFAULT_HOLE_CB_DEPTH,
  DEFAULT_HOLE_CB_DIAMETER,
  DEFAULT_HOLE_CS_ANGLE,
  DEFAULT_HOLE_CS_DIAMETER,
} from "@/tools/modelTools/holeMachine";
import type {
  AlignPhase,
  PatternAxis,
  MirrorPlane,
  BooleanMode,
  EdgeOpKind,
  ExtrudeEndCondition,
  TransformMode,
} from "@/tools/modelTools/modelToolMachine";

/** Handlers the armed extrude cluster wires (MODEL-HARDEN Wave 1 + 2). */
export interface ExtrudeChipHandlers {
  onValue: (v: number) => void;
  onSymmetric: (symmetric: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
  /** Boolean segment picked (New Body / Add / Cut — Wave 2). */
  onBooleanMode?: (mode: BooleanMode) => void;
  /** End condition picked (Blind / Through all / To next / To face — W1). */
  onEndCondition?: (end: ExtrudeEndCondition) => void;
  /** Draft angle authored in the [Draft] segment's input, in DEGREES (WP-C3). */
  onDraftAngle?: (deg: number) => void;
}

/** Extra armed-cluster options (symmetric seed; hide the ⇔ toggle in re-edit). */
export interface ExtrudeChipOpts {
  symmetric?: boolean;
  /** Seed the end-condition segment (W1; default Blind). */
  endCondition?: ExtrudeEndCondition;
  /**
   * Whether to offer the end conditions that REQUIRE an existing body. The worker
   * fails ToNext/ToFace outright without one ("ToNext requires an existing target
   * body"), so with zero bodies they are not offered rather than offered-and-doomed.
   */
  canUseBodyEnds?: boolean;
  /** Whether to render the end-condition segment group at all (fresh arm only). */
  showEndConditions?: boolean;
  /** Re-edit shows value + ✓/✕ only — no symmetric toggle (default true). */
  showSymmetric?: boolean;
  /** Seed the boolean mode segment (Wave 2; default NewBody). */
  booleanMode?: BooleanMode;
  /** Whether ≥1 existing body offers a boolean target (Wave 2; default false). */
  canBoolean?: boolean;
  /** Whether to render the New Body / Add / Cut segment group (fresh arm only). */
  showBooleanSegments?: boolean;
  /** How many regions the armed op covers (Wave 2; default 1). */
  regionCount?: number;
  /** Seed the [Draft] segment's angle in DEGREES (WP-C3; default 0). */
  draftAngleDeg?: number;
  /** Whether to render the [Draft] segment at all (default false). */
  showDraft?: boolean;
}

/** Handlers the armed revolve cluster wires (MODEL-HARDEN Wave 1 + 2). */
export interface RevolveChipHandlers {
  onValue: (v: number) => void;
  onResetAxis: () => void;
  onConfirm: () => void;
  onCancel: () => void;
  /** Boolean segment picked (New Body / Add / Cut — Wave 2). */
  onBooleanMode?: (mode: BooleanMode) => void;
}

/** Extra armed-revolve-cluster options (boolean segments — Wave 2). */
export interface RevolveChipOpts {
  booleanMode?: BooleanMode;
  canBoolean?: boolean;
  showBooleanSegments?: boolean;
}

/** Handlers the region-select chip wires (Wave 2 multi-region). */
export interface RegionSelectChipHandlers {
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * The ✓/✕ pair the armed FILLET / CHAMFER / SHELL chip wires (edge-op preview
 * wave). Optional so the parametric RE-EDIT chip — which commits straight out of
 * its numeric input and has no live preview session to confirm — keeps rendering
 * as the bare numeric chip it always was.
 */
export interface ValueChipHandlers {
  onConfirm?: () => void;
  onCancel?: () => void;
}

/**
 * Extra armed-EDGE-OP-cluster options (FILLET-CHAMFER-UNIFY). Only the FRESH
 * arm renders the [Fillet|Chamfer] segments; the parametric re-edit chip passes
 * none and stays the bare numeric chip it always was.
 */
export interface EdgeOpChipOpts {
  /** Seed the segment group's active op (default Fillet). */
  edgeOp?: EdgeOpKind;
  /** Whether to render the segment group at all (fresh arm only). */
  showEdgeOpSegments?: boolean;
  /** A segment was picked — the controller locks the type and swaps the session. */
  onEdgeOp?: (edgeOp: EdgeOpKind) => void;
  /** Seed the CHAMFER second-leg segment (`null` = equal-leg — SCHEMA §7.3). */
  distance2?: number | null;
  /** The second leg was typed or cleared back to `=` (equal-leg). */
  onDistance2?: (distance2: number | null) => void;
}

/**
 * Handlers the armed TRANSFORM cluster wires (WP-B W1):
 * `[Move|Rotate] [X|Y|Z] [value] [✓] [✕]`. `onValue` writes the ONE component the
 * current mode+axis addresses; the cluster never sees the whole placement.
 */
export interface TransformChipHandlers {
  onTransformMode: (mode: TransformMode) => void;
  onAxis: (axis: PatternAxis) => void;
  onValue: (v: number) => void;
  onConfirm: () => void;
  onCancel: () => void;
  /** Copy toggled (WP-B W2) — the visible surface for `params.copy`. */
  onCopy?: (copy: boolean) => void;
  /** [Align] pressed (WP-B W2.5) — starts the two-pick face-to-face flow. */
  onAlign?: () => void;
}

/** Extra armed-placement options (the W2 copy seed). */
export interface TransformChipOpts {
  copy?: boolean;
}

/**
 * Handlers the armed HOLE cluster wires (WP-C T3):
 * `[Simple|CBore|CSink] [Ø] [depth|Thru] (+ the active conditional pair) [Std ▾] [✓] [✕]`.
 *
 * `onValue` is the DRILL diameter — the one number every profile has, so the
 * cluster's primary input never changes meaning when the profile flips.
 */
export interface HoleChipHandlers {
  onValue: (v: number) => void;
  onHoleType: (holeType: HoleType) => void;
  /** `null` = through-all, which is a VALUE here, not a cleared field. */
  onDepth: (depth: number | null) => void;
  onCbDiameter: (v: number) => void;
  onCbDepth: (v: number) => void;
  onCsDiameter: (v: number) => void;
  onCsAngle: (deg: number) => void;
  /** A standards-picker row was chosen (thread + fit), already reduced to mm. */
  onStandard: (thread: string, fit: HoleFit) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

/** The armed hole's current numbers — everything the cluster renders. */
export interface HoleChipOpts {
  holeType?: HoleType;
  depth?: number | null;
  cbDiameter?: number;
  cbDepth?: number;
  csDiameter?: number;
  csAngleDeg?: number;
}

/**
 * Handlers the armed OFFSET FACE cluster wires (SCHEMA §7.3):
 * `[distance] [Offset|Total|Radius|Diameter] [⌒ tangent] [✓] [✕]`.
 */
export interface OffsetFaceChipHandlers {
  onValue: (v: number) => void;
  onDistanceType: (t: OffsetDistanceType) => void;
  onChainTangent: (chain: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

/** The armed offset's current state — what the cluster renders and gates on. */
export interface OffsetFaceChipOpts {
  distanceType?: OffsetDistanceType;
  chainTangentFaces?: boolean;
  /**
   * Which types the segment group offers. Planarity-driven (planar ⇒
   * Offset+Total, curved ⇒ Offset+Radius+Diameter) and narrowed to `["Offset"]`
   * for a multi-face closure, where SCHEMA §7.3 allows nothing else. Omitted ⇒
   * `["Offset"]`, which renders no choice at all — the honest default for a
   * selection whose surface type is unknown.
   */
  distanceTypes?: readonly OffsetDistanceType[];
  /** The last typed value was refused as out-of-domain (chip error state). */
  valueError?: boolean;
}

/** Handlers the armed DATUM chip wires (offset input + ✓/✕ — DATUM W1). */
export interface DatumChipHandlers {
  onValue: (v: number) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export type ChipKind =
  | "none"
  | "extrudeDepth"
  | "datumOffset"
  | "filletRadius"
  | "revolveAngle"
  | "booleanOp"
  | "shellThickness"
  /** The armed face-offset cluster (SCHEMA §7.3 OffsetFace). */
  | "offsetFace"
  | "linearPattern"
  | "circularPattern"
  | "mirror"
  | "transform"
  /** The armed machined-hole cluster (WP-C T3). */
  | "hole"
  | "regionSelect"
  | "dimension"
  /** A sketch EDIT tool's live parameter (fillet radius / offset distance —
   *  WP-C T2b): `[ <label> <value> mm ]`, open for as long as the tool is armed. */
  | "sketchValue";

export interface ToolChipState {
  kind: ChipKind;
  /** Live dimensional value (depth / radius / thickness / spacing / angle). */
  value: number;
  /** Live instance count (linear / circular pattern chips). */
  count: number;
  /** Selected world axis (pattern chips). */
  axis: PatternAxis;
  /** Selected mirror plane (mirror chip). */
  plane: MirrorPlane;
  /** Selected boolean operation (booleanOp chip). */
  op: BooleanOperation;
  /** Symmetric extrude toggle state (armed extrude cluster; Alt-drag syncs it). */
  symmetric: boolean;
  /** Armed extrude end condition (W1). */
  endCondition: ExtrudeEndCondition;
  canUseBodyEnds: boolean;
  showEndConditions: boolean;
  /** Whether the armed extrude cluster renders the ⇔ toggle (hidden in re-edit). */
  showSymmetric: boolean;
  /** Feature boolean mode — drives the Wave 2 New Body / Add / Cut segment group. */
  booleanMode: BooleanMode;
  /** Whether a boolean picker is offered (≥1 existing body) — Wave 2. */
  canBoolean: boolean;
  /** Whether the armed cluster renders the boolean segment group (fresh arm only). */
  showBooleanSegments: boolean;
  /** Which edge op the armed fillet/chamfer cluster is authoring. */
  edgeOp: EdgeOpKind;
  /** Whether the armed edge-op cluster renders the [Fillet|Chamfer] segments. */
  showEdgeOpSegments: boolean;
  /**
   * The CHAMFER-ONLY second distance (SCHEMA §7.3, 2026-08-03). `null` ⇒
   * equal-leg, which is what the `=` the segment shows means. Its segment is
   * rendered only while `edgeOp === "Chamfer"`, so a Fillet arm never displays it
   * even when the value is still parked here from an earlier flip.
   */
  distance2: number | null;
  /** How the armed offset reads its distance (SCHEMA §7.3 `distanceType`). */
  distanceType: OffsetDistanceType;
  /** Which distance types the armed offset's segment group offers. */
  distanceTypes: readonly OffsetDistanceType[];
  /** Whether the armed offset's frozen closure follows tangent faces. */
  chainTangentFaces: boolean;
  /** The last authored value was refused as out-of-domain (never clamped). */
  valueError: boolean;
  onDistanceType: ((t: OffsetDistanceType) => void) | null;
  onChainTangent: ((chain: boolean) => void) | null;
  /** Which profile the armed hole cluster is authoring (WP-C T3). */
  holeType: HoleType;
  /** Armed hole blind depth, or `null` = through-all. */
  holeDepth: number | null;
  /** Armed hole counterbore block (rendered only while `holeType === "counterbore"`). */
  cbDiameter: number;
  cbDepth: number;
  /** Armed hole countersink block (rendered only while `holeType === "countersink"`). */
  csDiameter: number;
  csAngleDeg: number;
  onHoleType: ((holeType: HoleType) => void) | null;
  onDepth: ((depth: number | null) => void) | null;
  onCbDiameter: ((v: number) => void) | null;
  onCbDepth: ((v: number) => void) | null;
  onCsDiameter: ((v: number) => void) | null;
  onCsAngle: ((deg: number) => void) | null;
  onStandard: ((thread: string, fit: HoleFit) => void) | null;
  /** How many regions the armed op covers (1 in the single-region path). */
  regionCount: number;
  /** Armed extrude draft angle in DEGREES — 0 renders the segment collapsed. */
  draftAngleDeg: number;
  /** Whether the armed extrude cluster renders the [Draft] segment. */
  showDraft: boolean;
  /** Whether the armed placement is authoring a translation or a rotation (W1). */
  transformMode: TransformMode;
  /** Placement mode segment picked (armed transform cluster — W1). */
  onTransformMode: ((mode: TransformMode) => void) | null;
  /** Whether the armed placement leaves the sources behind (WP-B W2). */
  copy: boolean;
  /** Copy toggled (armed placement cluster — W2). */
  onCopy: ((copy: boolean) => void) | null;
  /** Which align pick is outstanding, or null when the flow is off (W2.5). */
  alignPhase: AlignPhase | null;
  /** [Align] pressed (armed placement cluster — W2.5). */
  onAlign: (() => void) | null;
  /** Unit suffix for the numeric chip (mm / ° — sketch dimension chip). */
  suffix: string;
  /** Leading context label (datum chip: the picked base plane's geometric name). */
  label: string;
  /** World anchor for the overlay driver, or null. */
  worldPos: [number, number, number] | null;
  /** Committed value from the editable chip (Enter/blur). */
  onValue: ((v: number) => void) | null;
  /** Symmetric toggled (armed extrude cluster). */
  onSymmetric: ((symmetric: boolean) => void) | null;
  onEndCondition: ((end: ExtrudeEndCondition) => void) | null;
  /** Draft angle authored on the armed extrude cluster (WP-C3). */
  onDraftAngle: ((deg: number) => void) | null;
  /** Commit the armed op (chip ✓ / chip-input Enter). */
  onConfirm: (() => void) | null;
  /** Esc / cancel from the dimension chip, or ✕ from the armed cluster. */
  onCancel: (() => void) | null;
  /** Boolean op selected (standalone Boolean tool). */
  onOp: ((op: BooleanOperation) => void) | null;
  /** Boolean mode segment picked (armed extrude/revolve cluster — Wave 2). */
  onBooleanMode: ((mode: BooleanMode) => void) | null;
  /** Edge-op segment picked (armed fillet/chamfer cluster). */
  onEdgeOp: ((edgeOp: EdgeOpKind) => void) | null;
  /** Second chamfer distance typed / cleared (armed edge-op cluster). */
  onDistance2: ((distance2: number | null) => void) | null;
  /** Apply pressed (boolean / pattern / mirror chip). */
  onApply: (() => void) | null;
  /** Axis-reset pressed (revolve chip). */
  onResetAxis: (() => void) | null;
  /** World-axis toggled (pattern chips). */
  onAxis: ((axis: PatternAxis) => void) | null;
  /** Mirror plane toggled (mirror chip). */
  onPlane: ((plane: MirrorPlane) => void) | null;
  /** Instance count stepped (pattern chips). */
  onCount: ((count: number) => void) | null;

  showExtrude(
    value: number,
    worldPos: [number, number, number],
    handlers: ExtrudeChipHandlers,
    opts?: ExtrudeChipOpts,
  ): void;
  showFillet(
    value: number,
    worldPos: [number, number, number],
    onValue: (v: number) => void,
    handlers?: ValueChipHandlers,
    opts?: EdgeOpChipOpts,
  ): void;
  showRevolve(
    value: number,
    worldPos: [number, number, number],
    handlers: RevolveChipHandlers,
    opts?: RevolveChipOpts,
  ): void;
  /** Show the armed datum chip `[ <base> · offset mm ✓ ✕ ]` at the ghost origin. */
  showDatum(
    offset: number,
    worldPos: [number, number, number],
    baseLabel: string,
    handlers: DatumChipHandlers,
  ): void;
  /** Show the multi-region select chip `[ N regions ✓ ✕ ]` at the sketch centroid. */
  showRegionSelect(
    count: number,
    worldPos: [number, number, number],
    handlers: RegionSelectChipHandlers,
  ): void;
  showBoolean(
    op: BooleanOperation,
    worldPos: [number, number, number],
    onOp: (op: BooleanOperation) => void,
    onApply: () => void,
  ): void;
  /** Show the armed hole cluster at the picked point (WP-C T3). */
  showHole(
    diameter: number,
    worldPos: [number, number, number],
    handlers: HoleChipHandlers,
    opts?: HoleChipOpts,
  ): void;
  showShell(
    value: number,
    worldPos: [number, number, number],
    onValue: (v: number) => void,
    handlers?: ValueChipHandlers,
  ): void;
  /** Show the armed offset-face cluster at the operative faces' mean centre. */
  showOffsetFace(
    distance: number,
    worldPos: [number, number, number],
    handlers: OffsetFaceChipHandlers,
    opts?: OffsetFaceChipOpts,
  ): void;
  showLinearPattern(
    axis: PatternAxis,
    count: number,
    spacing: number,
    worldPos: [number, number, number],
    handlers: {
      onAxis: (axis: PatternAxis) => void;
      onCount: (count: number) => void;
      onSpacing: (spacing: number) => void;
      onApply: () => void;
    },
  ): void;
  showCircularPattern(
    axis: PatternAxis,
    count: number,
    angle: number,
    worldPos: [number, number, number],
    handlers: {
      onAxis: (axis: PatternAxis) => void;
      onCount: (count: number) => void;
      onAngle: (angle: number) => void;
      onApply: () => void;
    },
  ): void;
  showMirror(
    plane: MirrorPlane,
    worldPos: [number, number, number],
    handlers: { onPlane: (plane: MirrorPlane) => void; onApply: () => void },
  ): void;
  /** Show the armed placement cluster `[Move|Rotate][X|Y|Z][value ▸][Copy][✓][✕]`. */
  showTransform(
    mode: TransformMode,
    axis: PatternAxis,
    value: number,
    worldPos: [number, number, number],
    handlers: TransformChipHandlers,
    opts?: TransformChipOpts,
  ): void;
  /**
   * Show an armed sketch EDIT tool's parameter chip (WP-C T2b). Unlike every
   * chip above it commits NOTHING: it just holds the tool's live radius /
   * distance, so it stays open across repeated applies and `onValue` only
   * re-parameterises the next one.
   */
  showSketchValue(
    value: number,
    worldPos: [number, number, number],
    label: string,
    onValue: (v: number) => void,
  ): void;
  /** Show the sketch Dimension chip (seeded, auto-focused; Enter commits, Esc cancels). */
  showDimension(
    value: number,
    suffix: string,
    worldPos: [number, number, number],
    onValue: (v: number) => void,
    onCancel: () => void,
  ): void;
  /** Update just the live value during a drag / edit. */
  setValue(value: number): void;
  /** Update just the live instance count. */
  setCount(count: number): void;
  /** Update just the selected axis. */
  setAxis(axis: PatternAxis): void;
  /** Update just the selected mirror plane. */
  setPlane(plane: MirrorPlane): void;
  /** Update just the boolean op. */
  setOp(op: BooleanOperation): void;
  /** Update just the armed extrude symmetric toggle (Alt-drag / ⇔ toggle). */
  setSymmetric(symmetric: boolean): void;
  /** Update just the armed cluster boolean mode (New Body / Add / Cut — Wave 2). */
  setBooleanMode(mode: BooleanMode): void;
  /** Update just the armed extrude draft angle (post-clamp read-back — WP-C3). */
  setDraftAngle(deg: number): void;
  /** Update just the armed edge-op cluster's authored op (drag flip / segment pick). */
  setEdgeOp(edgeOp: EdgeOpKind): void;
  /** Update just the armed offset's distance type (segment pick read-back). */
  setDistanceType(distanceType: OffsetDistanceType): void;
  /** Update just the armed offset's tangent-chain flag. */
  setChainTangent(chainTangentFaces: boolean): void;
  /** Update just the armed offset's value-error flag (a refused entry). */
  setValueError(valueError: boolean): void;
  /** Update just the armed placement's mode (Move / Rotate). */
  setTransformMode(mode: TransformMode): void;
  /** Update just the armed placement's copy flag (Alt-drag / the Copy segment). */
  setCopy(copy: boolean): void;
  /** Update just the align sub-flow's outstanding pick (W2.5). */
  setAlignPhase(alignPhase: AlignPhase | null): void;
  /** Update just the chamfer second leg (`null` = equal-leg). */
  setDistance2(distance2: number | null): void;
  clear(): void;
}

const CLEARED = {
  kind: "none" as ChipKind,
  value: 0,
  count: 3,
  axis: "X" as PatternAxis,
  plane: "XY" as MirrorPlane,
  op: "Union" as BooleanOperation,
  symmetric: false,
  endCondition: "Blind" as ExtrudeEndCondition,
  canUseBodyEnds: false,
  showEndConditions: false,
  onEndCondition: null,
  showSymmetric: true,
  booleanMode: "NewBody" as BooleanMode,
  canBoolean: false,
  showBooleanSegments: false,
  edgeOp: "Fillet" as EdgeOpKind,
  distance2: null as number | null,
  onDistance2: null as ((distance2: number | null) => void) | null,
  showEdgeOpSegments: false,
  onEdgeOp: null,
  distanceType: "Offset" as OffsetDistanceType,
  distanceTypes: ["Offset"] as readonly OffsetDistanceType[],
  chainTangentFaces: true,
  valueError: false,
  onDistanceType: null,
  onChainTangent: null,
  holeType: "simple" as HoleType,
  holeDepth: null as number | null,
  cbDiameter: DEFAULT_HOLE_CB_DIAMETER,
  cbDepth: DEFAULT_HOLE_CB_DEPTH,
  csDiameter: DEFAULT_HOLE_CS_DIAMETER,
  csAngleDeg: DEFAULT_HOLE_CS_ANGLE,
  onHoleType: null,
  onDepth: null,
  onCbDiameter: null,
  onCbDepth: null,
  onCsDiameter: null,
  onCsAngle: null,
  onStandard: null,
  regionCount: 1,
  draftAngleDeg: 0,
  showDraft: false,
  onDraftAngle: null,
  transformMode: "move" as TransformMode,
  onTransformMode: null,
  copy: false,
  onCopy: null,
  alignPhase: null as AlignPhase | null,
  onAlign: null,
  suffix: "",
  label: "",
  worldPos: null,
  onValue: null,
  onSymmetric: null,
  onConfirm: null,
  onCancel: null,
  onOp: null,
  onBooleanMode: null,
  onApply: null,
  onResetAxis: null,
  onAxis: null,
  onPlane: null,
  onCount: null,
};

export const toolChipStore = createStore<ToolChipState>()((set) => ({
  ...CLEARED,

  showExtrude(value, worldPos, handlers, opts) {
    set({
      ...CLEARED,
      kind: "extrudeDepth",
      value,
      worldPos,
      symmetric: opts?.symmetric ?? false,
      endCondition: opts?.endCondition ?? "Blind",
      canUseBodyEnds: opts?.canUseBodyEnds ?? false,
      showEndConditions: opts?.showEndConditions ?? false,
      onEndCondition: handlers.onEndCondition ?? null,
      showSymmetric: opts?.showSymmetric ?? true,
      booleanMode: opts?.booleanMode ?? "NewBody",
      canBoolean: opts?.canBoolean ?? false,
      showBooleanSegments: opts?.showBooleanSegments ?? false,
      regionCount: opts?.regionCount ?? 1,
      draftAngleDeg: opts?.draftAngleDeg ?? 0,
      showDraft: opts?.showDraft ?? false,
      onValue: handlers.onValue,
      onSymmetric: handlers.onSymmetric,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
      onBooleanMode: handlers.onBooleanMode ?? null,
      onDraftAngle: handlers.onDraftAngle ?? null,
    });
  },
  showFillet(value, worldPos, onValue, handlers, opts) {
    set({
      ...CLEARED,
      kind: "filletRadius",
      value,
      worldPos,
      onValue,
      onConfirm: handlers?.onConfirm ?? null,
      onCancel: handlers?.onCancel ?? null,
      edgeOp: opts?.edgeOp ?? "Fillet",
      showEdgeOpSegments: opts?.showEdgeOpSegments ?? false,
      onEdgeOp: opts?.onEdgeOp ?? null,
      distance2: opts?.distance2 ?? null,
      onDistance2: opts?.onDistance2 ?? null,
    });
  },
  showRevolve(value, worldPos, handlers, opts) {
    set({
      ...CLEARED,
      kind: "revolveAngle",
      value,
      worldPos,
      booleanMode: opts?.booleanMode ?? "NewBody",
      canBoolean: opts?.canBoolean ?? false,
      showBooleanSegments: opts?.showBooleanSegments ?? false,
      onValue: handlers.onValue,
      onResetAxis: handlers.onResetAxis,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
      onBooleanMode: handlers.onBooleanMode ?? null,
    });
  },
  showDatum(offset, worldPos, baseLabel, handlers) {
    set({
      ...CLEARED,
      kind: "datumOffset",
      value: offset,
      label: baseLabel,
      worldPos,
      onValue: handlers.onValue,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
    });
  },
  showRegionSelect(count, worldPos, handlers) {
    set({
      ...CLEARED,
      kind: "regionSelect",
      count,
      worldPos,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
    });
  },
  showBoolean(op, worldPos, onOp, onApply) {
    set({ ...CLEARED, kind: "booleanOp", op, worldPos, onOp, onApply });
  },
  showHole(diameter, worldPos, handlers, opts) {
    set({
      ...CLEARED,
      kind: "hole",
      value: diameter,
      worldPos,
      holeType: opts?.holeType ?? "simple",
      // `undefined` means "not supplied"; `null` means THROUGH-ALL. `??` would
      // conflate them, so the check is explicit.
      holeDepth: opts?.depth === undefined ? null : opts.depth,
      cbDiameter: opts?.cbDiameter ?? DEFAULT_HOLE_CB_DIAMETER,
      cbDepth: opts?.cbDepth ?? DEFAULT_HOLE_CB_DEPTH,
      csDiameter: opts?.csDiameter ?? DEFAULT_HOLE_CS_DIAMETER,
      csAngleDeg: opts?.csAngleDeg ?? DEFAULT_HOLE_CS_ANGLE,
      onValue: handlers.onValue,
      onHoleType: handlers.onHoleType,
      onDepth: handlers.onDepth,
      onCbDiameter: handlers.onCbDiameter,
      onCbDepth: handlers.onCbDepth,
      onCsDiameter: handlers.onCsDiameter,
      onCsAngle: handlers.onCsAngle,
      onStandard: handlers.onStandard,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
    });
  },
  showShell(value, worldPos, onValue, handlers) {
    set({
      ...CLEARED,
      kind: "shellThickness",
      value,
      worldPos,
      onValue,
      onConfirm: handlers?.onConfirm ?? null,
      onCancel: handlers?.onCancel ?? null,
    });
  },
  showOffsetFace(distance, worldPos, handlers, opts) {
    set({
      ...CLEARED,
      kind: "offsetFace",
      value: distance,
      worldPos,
      distanceType: opts?.distanceType ?? "Offset",
      distanceTypes: opts?.distanceTypes ?? ["Offset"],
      chainTangentFaces: opts?.chainTangentFaces ?? true,
      valueError: opts?.valueError ?? false,
      onValue: handlers.onValue,
      onDistanceType: handlers.onDistanceType,
      onChainTangent: handlers.onChainTangent,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
    });
  },
  showLinearPattern(axis, count, spacing, worldPos, handlers) {
    set({
      ...CLEARED,
      kind: "linearPattern",
      axis,
      count,
      value: spacing,
      worldPos,
      onAxis: handlers.onAxis,
      onCount: handlers.onCount,
      onValue: handlers.onSpacing,
      onApply: handlers.onApply,
    });
  },
  showCircularPattern(axis, count, angle, worldPos, handlers) {
    set({
      ...CLEARED,
      kind: "circularPattern",
      axis,
      count,
      value: angle,
      worldPos,
      onAxis: handlers.onAxis,
      onCount: handlers.onCount,
      onValue: handlers.onAngle,
      onApply: handlers.onApply,
    });
  },
  showMirror(plane, worldPos, handlers) {
    set({ ...CLEARED, kind: "mirror", plane, worldPos, onPlane: handlers.onPlane, onApply: handlers.onApply });
  },
  showTransform(mode, axis, value, worldPos, handlers, opts) {
    set({
      ...CLEARED,
      kind: "transform",
      transformMode: mode,
      axis,
      value,
      worldPos,
      copy: opts?.copy ?? false,
      onTransformMode: handlers.onTransformMode,
      onAxis: handlers.onAxis,
      onValue: handlers.onValue,
      onConfirm: handlers.onConfirm,
      onCancel: handlers.onCancel,
      onCopy: handlers.onCopy ?? null,
      onAlign: handlers.onAlign ?? null,
    });
  },
  showSketchValue(value, worldPos, label, onValue) {
    set({ ...CLEARED, kind: "sketchValue", value, worldPos, label, onValue });
  },
  showDimension(value, suffix, worldPos, onValue, onCancel) {
    set({ ...CLEARED, kind: "dimension", value, suffix, worldPos, onValue, onCancel });
  },
  setValue(value) {
    set({ value });
  },
  setCount(count) {
    set({ count });
  },
  setAxis(axis) {
    set({ axis });
  },
  setPlane(plane) {
    set({ plane });
  },
  setOp(op) {
    set({ op });
  },
  setSymmetric(symmetric) {
    set({ symmetric });
  },
  setBooleanMode(booleanMode) {
    set({ booleanMode });
  },
  setDraftAngle(draftAngleDeg) {
    set({ draftAngleDeg });
  },
  setEdgeOp(edgeOp) {
    set({ edgeOp });
  },
  setDistance2(distance2) {
    set({ distance2 });
  },
  setDistanceType(distanceType) {
    set({ distanceType });
  },
  setChainTangent(chainTangentFaces) {
    set({ chainTangentFaces });
  },
  setValueError(valueError) {
    set({ valueError });
  },
  setTransformMode(transformMode) {
    set({ transformMode });
  },
  setCopy(copy) {
    set({ copy });
  },
  setAlignPhase(alignPhase) {
    set({ alignPhase });
  },
  clear() {
    set({ ...CLEARED });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useToolChipStore<T>(selector: (s: ToolChipState) => T): T {
  return useStore(toolChipStore, selector);
}
