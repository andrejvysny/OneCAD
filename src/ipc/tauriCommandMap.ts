/*
 * OperationOp → EditCommand wire mapping (SCHEMA §7.3 / src-tauri edit::command).
 *
 * The frontend authors high-level `OperationOp`s (Extrude / Fillet / Boolean).
 * The Rust `apply_edit_command` command consumes an `EditCommand` (serde tag
 * `"cmd"`, camelCase, camelCase fields) whose `AddOperation` variant carries a
 * full `OperationRecord`. The Rust deserializer DEFAULTS every record field
 * except `recordId` and the op's `{opType, params}` (verified against
 * `document/record.rs`), so a minimal real record is `{recordId, opType, params}`.
 *
 * ── Reference-id reconciliation (the key F-WP8 → M2 flag) ─────────────────────
 * Rust ids (`SketchId`/`BodyId`/`RegionId`/`ElementId`/`RecordId`) are
 * `#[serde(transparent)]` UUIDs. The refs an op needs come from different lanes:
 *   • BODY refs (Boolean target/tool) — arrive as REAL UUIDs on `document-changed`
 *     (`BodyMeshRef.bodyId`), so Boolean maps fully real. ✔
 *   • PARAMETRIC edit target (`featureId`) — a projection feature `id` IS the
 *     record's `RecordId` UUID, so an edit maps to `UpdateOperationParams`. ✔
 *   • SKETCH/REGION refs (Extrude profile) — come from the sketch SOLVER lane,
 *     which is LOCAL until R-WP12. Until then `sketchId`/`regionId` are local
 *     strings; the shape below is faithful but the backend rejects the non-UUID
 *     ids. R-WP12 makes them real → Extrude commit works with no shape change.
 *   • ELEMENT/EDGE refs (Fillet `edgeIds`) — are snapshot TopoKeys promoted to
 *     `ElementId` via `AcquireElementIds` (UNSUPPORTED in V1). Same story.
 *
 * The mapper therefore emits the STRUCTURALLY-REAL command every time; which ops
 * the live backend accepts today is purely a function of whether their input
 * lanes are wired (Boolean now; Extrude/Fillet with R-WP12 + AcquireElementIds).
 */
import type {
  AxisRef,
  BooleanParams,
  CircularPatternParams,
  ExtrudeMode,
  ExtrudeParams,
  FeatureBooleanMode,
  FilletParams,
  LinearPatternParams,
  MirrorBodyParams,
  OperationOp,
  RevolveParams,
  SemanticRef,
  HoleParams,
  ShellParams,
  TransformBodyParams,
} from "./types";

/** A dimension value on the wire (Rust `Scalar {value, expr?}`). */
interface WireScalar {
  value: number;
}

interface WireExtrudeParams {
  profile?: { sketchId: string; regionId: string };
  distance: WireScalar;
  draftAngleDeg: WireScalar;
  extrudeMode: ExtrudeMode;
  booleanMode: FeatureBooleanMode;
  targetBodyId?: string;
  twoDirections: boolean;
  extrudeMode2: ExtrudeMode;
  distance2: WireScalar;
  /**
   * `ToFace` targets — TYPED semantic refs (SCHEMA §7.3, amended 2026-07-16), not
   * bare ids: a bare `targetFaceId` carries no anchor/intent, so a ToFace target
   * would be un-repairable across parametric edits (Invariants 2/3). Absent for
   * every non-`ToFace` extrude.
   */
  targetFace?: WireElementRef;
  targetFace2?: WireElementRef;
}

/** Rust `AxisRef` (serde internally-tagged on `kind`, camelCase fields). */
type WireAxisRef =
  | { kind: "sketchLine"; sketchId: string; lineId: string }
  | { kind: "edge"; bodyId: string; edgeId: string };

interface WireRevolveParams {
  profile?: { sketchId: string; regionId: string };
  /** Rust `angleDeg` Scalar — DEGREES (no radians conversion). */
  angleDeg: WireScalar;
  axis?: WireAxisRef;
  booleanMode: FeatureBooleanMode;
  targetBodyId?: string;
}

interface WireFilletParams {
  radius: WireScalar;
  /**
   * Rust `ChamferParams::distance2` (SCHEMA §7.3, 2026-08-03). Emitted ONLY for
   * a Chamfer that has one — the field is skip-none on the Rust side, so an
   * equal-leg chamfer must marshal without the key or its wire form stops being
   * byte-identical to every document authored before the field existed.
   */
  distance2?: WireScalar;
  edgeIds: string[];
  /**
   * Typed per-edge semantic refs (Rust `FilletParams::edges` — one `ElementRef`
   * per `edgeIds` entry). CRITICAL: `edgeIds` (bare) and `edges` (typed) MUST stay
   * in lockstep — any command that rewrites one rewrites BOTH (record.rs FilletParams
   * / the M4b dual-edge rule). Optional so a legacy/bare-id fillet still marshals.
   */
  edges?: WireElementRef[];
  chainTangentEdges: boolean;
}

/** Rust `ElementRef` (refs.rs — identity + evidence + anchor; camelCase). */
export interface WireElementRef {
  primary?: { bodyId: string; elementId: string; kind: "face" | "edge" | "vertex" };
  anchor?: { worldPoint: [number, number, number]; surfaceUv?: [number, number] };
}

interface WireBooleanParams {
  operation: BooleanParams["operation"];
  targetBodyId: string;
  toolBodyId: string;
}

/** A world 3-vector on the wire (Rust `Vec3` — `try_from = "[f64; 3]"`, so `[x,y,z]`). */
type WireVec3 = [number, number, number];

/**
 * Rust `HoleParams` (record.rs; SCHEMA §7.3 `Hole`).
 *
 * Every dimension is a `Scalar` and every CONDITIONAL block is spelled explicitly
 * as `null` when inactive — the Rust field is `Option<Scalar>` WITHOUT
 * `skip_serializing_if`, and an authored `null` is what "this hole is not a
 * countersink" looks like on the wire. `depth: null` is the through-all end
 * condition, not a missing field.
 */
interface WireHoleParams {
  targetBodyId: string;
  face: WireElementRef;
  point: WireVec3;
  holeType: HoleParams["holeType"];
  diameter: WireScalar;
  depth: WireScalar | null;
  cbDiameter: WireScalar | null;
  cbDepth: WireScalar | null;
  csDiameter: WireScalar | null;
  csAngleDeg: WireScalar | null;
}

/** Rust `ShellParams` (record.rs). `openFaces` are bare ElementIds/TopoKeys. */
interface WireShellParams {
  thickness: WireScalar;
  openFaces: string[];
  targetBodyId?: string;
}

/**
 * Rust `LinearPatternParams` (record.rs). SCHEMA truth: the C++ flat `dirX/Y/Z`
 * is a single `direction: Vec3` here (the UI axis chip picks a world axis), and
 * `count` is a bare `u32` (NOT a Scalar). `fuseResult` defaults true on the Rust
 * side (`default_true`).
 */
interface WireLinearPatternParams {
  sourceBodyId?: string;
  direction: WireVec3;
  spacing: WireScalar;
  count: number;
  fuseResult: boolean;
}

/**
 * Rust `CircularPatternParams` (record.rs). The C++ flat point/dir become
 * `axisOrigin`/`axisDirection` Vec3s; `angleDeg` is a Scalar, `count` a bare u32.
 */
interface WireCircularPatternParams {
  sourceBodyId?: string;
  axisOrigin: WireVec3;
  axisDirection: WireVec3;
  angleDeg: WireScalar;
  count: number;
  fuseResult: boolean;
}

/**
 * Rust `MirrorBodyParams` (record.rs). The C++ flat plane point/normal become
 * `planePoint`/`planeNormal` Vec3s; `fuseWithOriginal` defaults FALSE on Rust.
 */
interface WireMirrorBodyParams {
  sourceBodyId?: string;
  planePoint: WireVec3;
  planeNormal: WireVec3;
  fuseWithOriginal: boolean;
}

/**
 * Rust `TransformBodyParams` / `TransformRotation` (record.rs). Each `translate`
 * component and `angleDeg` is expression-capable and therefore a Scalar;
 * `center`/`axis` are plain Vec3s.
 *
 * `targets` is `Vec<BodyId>` with the PLAIN derive — unlike `sourceBodyId` on
 * the pattern params there is no `de_opt_body_id` leniency — so every id must
 * reach it through [`bareBodyId`] or the whole EditCommand is rejected.
 */
interface WireTransformBodyParams {
  targets: string[];
  translate: [WireScalar, WireScalar, WireScalar];
  rotate: { center: WireVec3; axis: WireVec3; angleDeg: WireScalar };
  copy: boolean;
}

/** A known op on the wire — adjacently tagged `{opType, params}` (SCHEMA §7.3). */
type WireOperation = (
  | { opType: "Extrude"; params: WireExtrudeParams }
  | { opType: "Revolve"; params: WireRevolveParams }
  | { opType: "Fillet"; params: WireFilletParams }
  | { opType: "Chamfer"; params: WireFilletParams }
  | { opType: "Boolean"; params: WireBooleanParams }
  | { opType: "Shell"; params: WireShellParams }
  | { opType: "LinearPattern"; params: WireLinearPatternParams }
  | { opType: "CircularPattern"; params: WireCircularPatternParams }
  | { opType: "MirrorBody"; params: WireMirrorBodyParams }
  | { opType: "TransformBody"; params: WireTransformBodyParams }
  | { opType: "Hole"; params: WireHoleParams }
) & { opId?: string };

/** A minimal real `OperationRecord` (every other field defaults on the Rust side). */
interface WireOperationRecord {
  recordId: string;
  opType: WireOperation["opType"];
  params: WireOperation["params"];
}

/**
 * A typed input-slot path (Rust `InputPath`, internally tagged on `"path"`,
 * camelCase). Only the fillet-edge arm is authored by M4b.
 */
export type WireInputPath = { path: "filletEdges"; index: number };

/**
 * The payload of an `EditOperationInput` (Rust `InputRef`, externally tagged,
 * camelCase). M4b authors only the `element` arm (fillet/chamfer edge rebind).
 */
export type WireInputRef = { element: WireElementRef };

/**
 * What a `setVisibility` targets — Rust `VisibilityTarget`, EXTERNALLY tagged
 * camelCase (`edit/command.rs`): `{"body": "<uuid>"}` / `{"sketch": "<uuid>"}`.
 */
export type WireVisibilityTarget = { body: string } | { sketch: string };

/**
 * The core `DatumPlane` struct as `AddDatumPlane` carries it (camelCase).
 *
 * These are exactly the fields the core serde REQUIRES — `basePlaneId` and the
 * typed refs default, everything here does not. `resolvedPlane`/`resolvedValid`
 * are required by the struct but are OVERWRITTEN by `DocumentSession::add_datum`
 * (Rust is the basis authority), so `buildAddDatumPlane` sends a placeholder and
 * the frontend must read the real frame back off the projection.
 */
export interface WireDatumPlane {
  id: string;
  name: string;
  /** PascalCase `DatumKind` token. V1 authors only `"OffsetFromPlane"`. */
  kind: "OffsetFromPlane" | "OffsetFromFace" | "AngledFromEdge" | "ThreePoint";
  /** `"XY"` | `"XZ"` | `"YZ"`, or another datum's id (chained offsets). */
  basePlaneId: string;
  offset: number;
  angleDeg: number;
  resolvedPlane: {
    origin: [number, number, number];
    xAxis: [number, number, number];
    yAxis: [number, number, number];
    normal: [number, number, number];
  };
  resolvedValid: boolean;
}

/** The `EditCommand` variants this WP emits (serde tag `"cmd"`, camelCase). */
export type WireEditCommand =
  | { cmd: "addOperation"; record: WireOperationRecord; atCursor: boolean }
  | { cmd: "updateOperationParams"; record: string; op: WireOperation }
  | { cmd: "editOperationInput"; record: string; path: WireInputPath; reference: WireInputRef }
  | { cmd: "removeOperation"; record: string }
  | { cmd: "setRollback"; cursor: number }
  | { cmd: "setOperationSuppression"; record: string; suppressed: boolean; cascade: boolean }
  // ── Body/sketch METADATA (TRUST wave) — the only user-authored facts about a
  // body or sketch. All three are `RegenHint::None` on the Rust side
  // (`edit/session.rs` dirty/regen table), so they publish a projection and fire
  // NO regen — see the metadata-only transport in `tauriClient.applyEditCommand`.
  | { cmd: "setVisibility"; target: WireVisibilityTarget; visible: boolean }
  | { cmd: "renameBody"; body: string; name: string }
  | { cmd: "renameSketch"; sketch: string; name: string }
  // ── Datum planes (DATUM W1) — also `RegenHint::None`, same metadata-only
  // transport. `addDatumPlane` carries the RAW core `DatumPlane` struct, so it
  // must satisfy that struct's serde (see `buildAddDatumPlane`).
  | { cmd: "addDatumPlane"; datum: WireDatumPlane }
  | { cmd: "deleteDatum"; datum: string };

const scalar = (n: number): WireScalar => ({ value: n });

/**
 * Normalize a body id to the BARE uuid the core `EditCommand` serde expects
 * (`BodyId` is `#[serde(transparent)]`, so it (de)serializes as the bare uuid).
 *
 * The frontend mostly HOLDS bare uuids — `document-changed`/`projection-updated`
 * emit `body.to_string()` (bare), so tree/scene/selection body ids are bare. The
 * one exception is `promoteSelection`, whose `PromotedElementDto.bodyId` comes back
 * in the worker `body_<uuid>` WIRE form (`body_id_wire`). Any body id that flows
 * into a typed ref bound for `apply_edit_command` MUST be stripped here, or the core
 * rejects the whole command (`"body_…"` is not a uuid). Idempotent: a bare uuid — or
 * a mock id like `body1` (no underscore) — passes through unchanged.
 */
export function bareBodyId(id: string): string {
  return id.startsWith("body_") ? id.slice("body_".length) : id;
}

/** Mint a client-side record id (real UUID; V1 has no server-side pre-mint step). */
export function mintRecordId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  // Fallback (jsdom without randomUUID): RFC-4122 v4 from getRandomValues.
  const b = c.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0"));
  return `${h[0]}${h[1]}${h[2]}${h[3]}-${h[4]}${h[5]}-${h[6]}${h[7]}-${h[8]}${h[9]}-${h[10]}${h[11]}${h[12]}${h[13]}${h[14]}${h[15]}`;
}

function extrudeParams(p: ExtrudeParams): WireExtrudeParams {
  const wire: WireExtrudeParams = {
    distance: scalar(p.distance),
    draftAngleDeg: scalar(p.draftAngleDeg ?? 0),
    extrudeMode: p.extrudeMode ?? "Blind",
    booleanMode: p.booleanMode ?? "NewBody",
    twoDirections: p.twoDirections ?? false,
    extrudeMode2: p.extrudeMode2 ?? "Blind",
    distance2: scalar(p.distance2 ?? 0),
  };
  if (p.targetBodyId !== undefined) wire.targetBodyId = p.targetBodyId;
  // ToFace targets only — a non-ToFace extrude must carry no target face at all
  // (SCHEMA §7.3 "Absent for non-ToFace extrudes"), so a mode switch away from
  // ToFace cannot leave a stale ref behind to be silently resolved.
  if (p.extrudeMode === "ToFace" && p.targetFace) {
    wire.targetFace = faceElementRef(p.targetFace);
  }
  if (p.twoDirections && p.extrudeMode2 === "ToFace" && p.targetFace2) {
    wire.targetFace2 = faceElementRef(p.targetFace2);
  }
  return wire;
}

/**
 * A picked FACE as the typed `ElementRef` the wire carries. Mirrors
 * [`edgeElementRef`] — same bare-uuid body normalization, since a promoted pick
 * returns the worker `body_<uuid>` form that the core `EditCommand` serde rejects.
 */
export function faceElementRef(ref: SemanticRef): WireElementRef {
  const out: WireElementRef = {
    primary: {
      bodyId: bareBodyId(ref.primary.bodyId),
      elementId: ref.primary.elementId ?? "",
      kind: "face",
    },
  };
  if (ref.anchor?.worldPoint) {
    out.anchor = { worldPoint: ref.anchor.worldPoint };
    if (ref.anchor.surfaceUv) out.anchor.surfaceUv = ref.anchor.surfaceUv;
  }
  return out;
}

function axisRef(a: AxisRef): WireAxisRef {
  return a.kind === "sketchLine"
    ? { kind: "sketchLine", sketchId: a.sketchId, lineId: a.lineId }
    : { kind: "edge", bodyId: a.bodyId, edgeId: a.edgeId };
}

function revolveParams(p: RevolveParams): WireRevolveParams {
  const wire: WireRevolveParams = {
    // Rust `angleDeg` is DEGREES — pass through unchanged (unit pinned).
    angleDeg: scalar(p.angleDeg),
    booleanMode: p.booleanMode ?? "NewBody",
  };
  if (p.axis !== undefined) wire.axis = axisRef(p.axis);
  if (p.targetBodyId !== undefined) wire.targetBodyId = p.targetBodyId;
  return wire;
}

function filletParams(p: FilletParams, inputs?: SemanticRef[]): WireFilletParams {
  const edgeIds = [...p.edgeIds];
  const wire: WireFilletParams = {
    radius: scalar(p.radius),
    // Chamfer shares FilletChamferParams in C++, but the vertical-slice tool only
    // authors Fillet; a Chamfer would map to opType "Chamfer" (future).
    edgeIds,
    chainTangentEdges: p.chainTangentEdges ?? true,
  };
  // SCHEMA §7.3 (2026-08-03): the second leg is CHAMFER-ONLY. Gating on `mode`
  // here is the marshalling-seam half of that rule (core's session validator is
  // the other): a caller that leaves a stale `distance2` on a params object it
  // has just re-typed to Fillet marshals a plain fillet, never a record core has
  // to reject.
  if (p.mode === "Chamfer" && p.distance2 !== undefined && p.distance2 > 0) {
    wire.distance2 = scalar(p.distance2);
  }
  // R-WP2.1 dual rule: carry the typed `edges` in LOCKSTEP with `edgeIds`, built from
  // the op's per-edge SemanticRefs (`OperationOp.inputs`). Each typed ref supplies the
  // operated body — `record.rs::derive_inputs` recovers it from `primary.body`, and
  // `wire.rs::edge_input_refs` sends it as `primary.bodyId`, so a UI-selected fillet
  // reaches the worker with a body (`FilletChamferOp::target_body_of`). Without the
  // typed `edges` a pure-`edgeIds` fillet derives NO body (M4a note) and the worker
  // fails "Fillet requires body input". `primary.element` MUST equal `edgeIds[i]`
  // (session.rs F2 lockstep); the anchor world-point rides so the worker's ladder can
  // resolve the edge. The core `ElementRef.primary` REQUIRES a bodyId, so the typed
  // `edges` is emitted only when every edge input carries one — otherwise the op
  // marshals as a bare-`edgeIds` fillet (the unchanged legacy path).
  const edgeInputs = (inputs ?? []).filter((r) => r.primary.kind === "edge");
  if (edgeInputs.length === edgeIds.length && edgeInputs.every((r) => r.primary.bodyId)) {
    wire.edges = edgeIds.map((id, i) =>
      edgeElementRef(edgeInputs[i].primary.bodyId, id, edgeInputs[i].anchor?.worldPoint),
    );
  }
  return wire;
}

/**
 * SCHEMA §7.3 `Hole`. The host FACE marshals as a typed `ElementRef` exactly like
 * a fillet edge — identity + the anchor world point — so the worker's ladder can
 * rebind the seat after a parametric edit instead of drilling into whatever face
 * happens to carry the old id. `bodyId` is normalized to the bare-uuid core form
 * ([`bareBodyId`]) for the same reason `edgeElementRef` does it: a pick carries a
 * bare uuid, a promoted selection carries the `body_<uuid>` wire form, and the
 * core `EditCommand` serde accepts only the former.
 *
 * The conditional blocks are gated on `holeType`, NOT on presence: a caller that
 * leaves a stale `cbDiameter` on params it has just re-typed to `countersink`
 * marshals a clean countersink rather than a record the Rust session must reject.
 */
function holeParams(p: HoleParams): WireHoleParams {
  const cb = p.holeType === "counterbore";
  const cs = p.holeType === "countersink";
  const optional = (active: boolean, v: number | null | undefined): WireScalar | null =>
    active && typeof v === "number" && Number.isFinite(v) ? scalar(v) : null;
  const face: WireElementRef = {
    primary: {
      bodyId: bareBodyId(p.face.primary.bodyId),
      elementId: p.face.primary.elementId ?? "",
      kind: "face",
    },
  };
  if (p.face.anchor?.worldPoint) face.anchor = { worldPoint: p.face.anchor.worldPoint };
  return {
    targetBodyId: bareBodyId(p.targetBodyId),
    face,
    point: [...p.point],
    holeType: p.holeType,
    diameter: scalar(p.diameter),
    depth: typeof p.depth === "number" && Number.isFinite(p.depth) ? scalar(p.depth) : null,
    cbDiameter: optional(cb, p.cbDiameter),
    cbDepth: optional(cb, p.cbDepth),
    csDiameter: optional(cs, p.csDiameter),
    csAngleDeg: optional(cs, p.csAngleDeg),
  };
}

function booleanParams(p: BooleanParams): WireBooleanParams {
  return {
    operation: p.operation,
    targetBodyId: p.targetBodyId,
    toolBodyId: p.toolBodyId,
  };
}

function shellParams(p: ShellParams): WireShellParams {
  const wire: WireShellParams = {
    thickness: scalar(p.thickness),
    openFaces: [...p.openFaces],
  };
  if (p.targetBodyId !== undefined) wire.targetBodyId = p.targetBodyId;
  return wire;
}

function linearPatternParams(p: LinearPatternParams): WireLinearPatternParams {
  const wire: WireLinearPatternParams = {
    direction: [...p.direction],
    spacing: scalar(p.spacing),
    count: p.count, // bare u32 — NOT a Scalar
    fuseResult: p.fuseResult ?? true,
  };
  if (p.sourceBodyId !== undefined) wire.sourceBodyId = p.sourceBodyId;
  return wire;
}

function circularPatternParams(p: CircularPatternParams): WireCircularPatternParams {
  const wire: WireCircularPatternParams = {
    axisOrigin: [...p.axisOrigin],
    axisDirection: [...p.axisDirection],
    angleDeg: scalar(p.angleDeg),
    count: p.count, // bare u32 — NOT a Scalar
    fuseResult: p.fuseResult ?? true,
  };
  if (p.sourceBodyId !== undefined) wire.sourceBodyId = p.sourceBodyId;
  return wire;
}

function mirrorBodyParams(p: MirrorBodyParams): WireMirrorBodyParams {
  const wire: WireMirrorBodyParams = {
    planePoint: [...p.planePoint],
    planeNormal: [...p.planeNormal],
    fuseWithOriginal: p.fuseWithOriginal ?? false,
  };
  if (p.sourceBodyId !== undefined) wire.sourceBodyId = p.sourceBodyId;
  return wire;
}

function transformBodyParams(p: TransformBodyParams): WireTransformBodyParams {
  return {
    targets: p.targets.map(bareBodyId),
    translate: [scalar(p.translate[0]), scalar(p.translate[1]), scalar(p.translate[2])],
    rotate: {
      center: [...p.rotate.center],
      axis: [...p.rotate.axis],
      angleDeg: scalar(p.rotate.angleDeg),
    },
    copy: p.copy,
  };
}

/** Build the `{opType, params}` wire op for an OperationOp (no ids yet). */
export function wireOperation(op: OperationOp): WireOperation {
  const identity = op.opId ? { opId: op.opId } : {};
  switch (op.opType) {
    case "Extrude": {
      const params = extrudeParams(op.params);
      // The profile is a SketchRegionRef; the ids are real once R-WP12 lands.
      if (op.sketchId && op.regionId) {
        params.profile = { sketchId: op.sketchId, regionId: op.regionId };
      }
      return { ...identity, opType: "Extrude", params };
    }
    case "Revolve": {
      const params = revolveParams(op.params);
      // The profile is a SketchRegionRef (ids real once R-WP12 lands, as Extrude).
      if (op.sketchId && op.regionId) {
        params.profile = { sketchId: op.sketchId, regionId: op.regionId };
      }
      return { ...identity, opType: "Revolve", params };
    }
    case "Fillet":
      return { ...identity, opType: "Fillet", params: filletParams(op.params, op.inputs) };
    // Chamfer shares FilletChamferParams in C++ / SCHEMA §7.3 and has always been
    // implemented in the worker (`execute_chamfer`); `mode` on the frontend params
    // is what distinguishes the two authoring paths.
    case "Chamfer":
      return { ...identity, opType: "Chamfer", params: filletParams(op.params, op.inputs) };
    case "Boolean":
      return { ...identity, opType: "Boolean", params: booleanParams(op.params) };
    case "Shell":
      return { ...identity, opType: "Shell", params: shellParams(op.params) };
    case "LinearPattern":
      return { ...identity, opType: "LinearPattern", params: linearPatternParams(op.params) };
    case "CircularPattern":
      return { ...identity, opType: "CircularPattern", params: circularPatternParams(op.params) };
    case "MirrorBody":
      return { ...identity, opType: "MirrorBody", params: mirrorBodyParams(op.params) };
    case "TransformBody":
      return { ...identity, opType: "TransformBody", params: transformBodyParams(op.params) };
    case "Hole":
      return { ...identity, opType: "Hole", params: holeParams(op.params) };
  }
}

/**
 * Map an OperationOp to the `EditCommand` payload for `apply_edit_command`.
 * A `featureId` (a projection feature's `RecordId`) re-targets an existing op via
 * `UpdateOperationParams`; otherwise a fresh op is appended via `AddOperation`.
 */
export function operationToEditCommand(op: OperationOp): WireEditCommand {
  const operation = wireOperation(op);
  const committedOperation = { ...operation };
  delete committedOperation.opId;
  if (op.featureId !== undefined) {
    return { cmd: "updateOperationParams", record: op.featureId, op: committedOperation };
  }
  return {
    cmd: "addOperation",
    record: { recordId: op.opId ?? mintRecordId(), ...committedOperation },
    // H7a: author AT the rollback cursor, always — the core insert (verified
    // byte-equivalent to frontier append when cursor==len, see
    // Timeline::insert_at_cursor) makes a frontier edit a no-op change, and a
    // rolled-back edit lands mid-history instead of joining the timeline PAST
    // the cursor as a permanently-inert draft that never regenerates.
    atCursor: true,
  };
}

/** The wire `params` object for an OperationOp (the EditCommand `op.params` shape). */
export function wireParamsOf(op: OperationOp): Record<string, unknown> {
  return wireOperation(op).params as unknown as Record<string, unknown>;
}

/** A scalar-only re-edit patch (only the changed dimension(s), keyed by wire field). */
export type ScalarPatch = Record<string, WireScalar>;

/**
 * `UpdateOperationParams` that changes ONLY the given field(s) of a stored op,
 * preserving every OTHER param (revolve `axis`, shell `openFaces`, fillet `edges` /
 * `edgeIds`, boolean `targetBodyId` / `toolBodyId`, `profile`) VERBATIM.
 * `storedParams` is the op's params JSON from `get_operation_params` (already the
 * EditCommand `op.params` serde shape).
 *
 * A parametric re-edit (double-click a feature → change one dimension, or swap a
 * Boolean's operation) cannot rebuild these non-scalar inputs from the projection,
 * so a whole-params replace would silently clobber them (drop the picked revolve
 * axis / wipe the shell's open faces + target). Merging the patch into the stored
 * params here is the fix.
 *
 * The patch is `Record<string, unknown>` because not every re-editable field is a
 * `{value}` scalar — a Boolean's `operation` is a bare enum string. [`ScalarPatch`]
 * is still the shape every DIMENSION re-edit passes.
 *
 * CALLER CONTRACT: the merge is SHALLOW (one `{...stored, ...patch}` spread), so a
 * patch key must be a TOP-LEVEL params field and its value must be complete —
 * patching `{profile: {regionId}}` would replace the whole stored `profile` object
 * (dropping its `sketchId`), not merge into it.
 */
export function updateScalarParamsCommand(
  recordId: string,
  opType: WireOperation["opType"],
  storedParams: Record<string, unknown>,
  patch: Record<string, unknown>,
): WireEditCommand {
  const op = { opType, params: { ...storedParams, ...patch } } as unknown as WireOperation;
  return { cmd: "updateOperationParams", record: recordId, op };
}

/** Human label for a committed/undone op, for the status-bar hint. */
export function opLabelFor(op: OperationOp): string {
  switch (op.opType) {
    case "Boolean":
      return op.params.operation;
    case "LinearPattern":
      return "Linear Pattern";
    case "CircularPattern":
      return "Circular Pattern";
    case "MirrorBody":
      return "Mirror";
    // Matches `dto.rs default_label` — the history row for a TransformBody reads
    // "Move", so the status hint must not say "TransformBody".
    case "TransformBody":
      return "Move";
    default:
      return op.opType;
  }
}

// ── M4b: raw EditCommand builders (repair rebind + history affordances) ────────
//
// These map straight onto the Rust `EditCommand` vocabulary (edit/command.rs) so
// `client.applyEditCommand(cmd)` can send them verbatim. The record/cursor ids
// are the projection feature ids (a feature's `id` IS its `RecordId` UUID) and
// the timeline cursor (= applied op count; history/timeline.rs).

/** The current fillet params a rebind rewrites (the SUBSET M4b touches). */
export interface CurrentFilletParams {
  radius: number;
  edgeIds: string[];
  /** Typed refs, parallel to `edgeIds` (may be shorter for a legacy fillet). */
  edges?: WireElementRef[];
  chainTangentEdges?: boolean;
}

/**
 * Build the typed edge `ElementRef` for a rebound edge (primary + anchor). The
 * `bodyId` is normalized to the bare-uuid core form ([`bareBodyId`]) so BOTH the
 * fresh-fillet path (picks carry bare uuids) and the M4b repair rebind path
 * (`promoteSelection` returns the `body_<uuid>` wire form) marshal a body id the
 * core `EditCommand` serde accepts.
 */
export function edgeElementRef(
  bodyId: string,
  elementId: string,
  worldPos?: [number, number, number],
): WireElementRef {
  const ref: WireElementRef = { primary: { bodyId: bareBodyId(bodyId), elementId, kind: "edge" } };
  if (worldPos) ref.anchor = { worldPoint: worldPos };
  return ref;
}

/**
 * PURE dual-field fillet-edge rewrite (M4b pinned rule): replace ONLY slot
 * `index` in BOTH `edgeIds` (bare) and `edges` (typed), leaving every sibling
 * edge untouched. `edgeIds[index]` becomes the minted `elementId`; `edges[index]`
 * becomes the typed `ElementRef`. Both arrays are grown to `index + 1` if the
 * current fillet stored fewer entries (legacy/short). Returns the full new
 * `WireFilletParams` an `UpdateOperationParams` carries.
 */
export function rewriteFilletEdgeParams(
  current: CurrentFilletParams,
  index: number,
  ref: WireElementRef,
): WireFilletParams {
  const elementId = ref.primary?.elementId ?? "";
  const edgeIds = [...current.edgeIds];
  const edges = [...(current.edges ?? [])];
  // Grow both arrays so slot `index` exists (keep them the SAME length).
  const len = Math.max(edgeIds.length, edges.length, index + 1);
  while (edgeIds.length < len) edgeIds.push("");
  while (edges.length < len) edges.push({});
  edgeIds[index] = elementId; // bare id (lockstep)
  edges[index] = ref; // typed ref (lockstep)
  return {
    radius: scalar(current.radius),
    edgeIds,
    edges,
    chainTangentEdges: current.chainTangentEdges ?? true,
  };
}

/** `UpdateOperationParams` for a rewritten Fillet (the pinned dual-field path). */
export function updateFilletParamsCommand(
  recordId: string,
  params: WireFilletParams,
): WireEditCommand {
  return { cmd: "updateOperationParams", record: recordId, op: { opType: "Fillet", params } };
}

/**
 * `EditOperationInput` for a single fillet edge slot (the backend-designated
 * fillet-edge rebind — command.rs `InputPath::FilletEdges`, which populates BOTH
 * `edge_ids[index]` and `edges[index]` in lockstep server-side). Needs only the
 * slot index + the new ref, so it works WITHOUT the frontend knowing the fillet's
 * full current edge set (which the projection does not expose).
 */
export function filletEdgeRebindCommand(
  recordId: string,
  index: number,
  ref: WireElementRef,
): WireEditCommand {
  return {
    cmd: "editOperationInput",
    record: recordId,
    path: { path: "filletEdges", index },
    reference: { element: ref },
  };
}

/** `SetOperationSuppression` — suppress/un-suppress `recordId` (optional cascade). */
export function suppressOperationCommand(
  recordId: string,
  suppressed: boolean,
  cascade = false,
): WireEditCommand {
  return { cmd: "setOperationSuppression", record: recordId, suppressed, cascade };
}

/** `SetRollback` — move the rollback cursor (= applied op count; timeline.rs). */
export function rollbackToCursorCommand(cursor: number): WireEditCommand {
  return { cmd: "setRollback", cursor: Math.max(0, Math.floor(cursor)) };
}

/** `RemoveOperation` — delete `recordId` from the timeline. */
export function removeOperationCommand(recordId: string): WireEditCommand {
  return { cmd: "removeOperation", record: recordId };
}

// ── Body / sketch metadata (TRUST wave) ───────────────────────────────────────
//
// The tree's eye + rename were purely local zustand flips, so any rehydration
// (projection-updated / reopen) silently reverted them. These map onto the core
// `SetVisibility` / `RenameBody` / `RenameSketch` commands, which the backend now
// validates against regen-minted bodies and persists through save (see
// `src-tauri/tests/body_metadata.rs`).
//
// Body ids go through `bareBodyId`: `BodyId` is `#[serde(transparent)]`, so the
// core wants the BARE uuid — a `body_<uuid>` worker-wire id is not a uuid and the
// whole command would be rejected. Sketch ids are already bare (the projection
// keys sketches by `SketchId.to_string()`).

/** `SetVisibility{Body}` — show/hide a body in the tree + viewport. */
export function setBodyVisibilityCommand(bodyId: string, visible: boolean): WireEditCommand {
  return { cmd: "setVisibility", target: { body: bareBodyId(bodyId) }, visible };
}

/** `SetVisibility{Sketch}` — show/hide a sketch's always-visible layer. */
export function setSketchVisibilityCommand(sketchId: string, visible: boolean): WireEditCommand {
  return { cmd: "setVisibility", target: { sketch: sketchId }, visible };
}

/** `RenameBody` — the body's user-authored name (durable across regen + save). */
export function renameBodyCommand(bodyId: string, name: string): WireEditCommand {
  return { cmd: "renameBody", body: bareBodyId(bodyId), name };
}

/** `RenameSketch` — the sketch's user-authored name. */
export function renameSketchCommand(sketchId: string, name: string): WireEditCommand {
  return { cmd: "renameSketch", sketch: sketchId, name };
}

// ── Datum planes (DATUM W1) ──────────────────────────────────────────────────

/**
 * A placeholder resolved frame for a freshly authored datum.
 *
 * The core REQUIRES `resolvedPlane`/`resolvedValid` (the struct has no serde
 * defaults for them) but OVERWRITES both in `add_datum` — it re-derives the frame
 * from the parametric definition and never trusts a client-supplied basis. So
 * this is deliberately the identity XY frame with `resolvedValid: false`: the
 * least-surprising thing to see if it ever DID survive, and identical to the Rust
 * `DatumPlane::offset_from_plane` constructor's own starting point. Read the real
 * frame back off `projection.datums[id].plane`.
 */
const UNRESOLVED_PLANE: WireDatumPlane["resolvedPlane"] = {
  origin: [0, 0, 0],
  xAxis: [0, 1, 0],
  yAxis: [-1, 0, 0],
  normal: [0, 0, 1],
};

/**
 * `AddDatumPlane` — an `OffsetFromPlane` datum offset from `basePlaneId` along
 * that base's NORMAL.
 *
 * **The bases are non-standard** (`Sketch.h`, ported verbatim): "XZ" has world
 * normal **+X** and "YZ" has **+Y**, so "XZ offset 10" moves along world +X, not
 * along −Y. Callers must not "correct" the sign for a base plane.
 *
 * `basePlaneId` may also be another datum's id — the backend chains off its
 * resolved frame. An unresolvable base is accepted leniently and comes back with
 * `resolvedValid: false` (it cannot host a sketch).
 */
export function buildAddDatumPlane(
  id: string,
  name: string,
  basePlaneId: string,
  offset: number,
): WireEditCommand {
  return {
    cmd: "addDatumPlane",
    datum: {
      id,
      name,
      kind: "OffsetFromPlane",
      basePlaneId,
      offset,
      angleDeg: 0,
      resolvedPlane: UNRESOLVED_PLANE,
      resolvedValid: false,
    },
  };
}

/**
 * `DeleteDatum` — remove a datum plane. The backend REJECTS this while any sketch
 * is attached to the datum (the error names the blocking sketches), so a caller
 * must surface the rejection rather than assume success.
 */
export function deleteDatumCommand(datumId: string): WireEditCommand {
  return { cmd: "deleteDatum", datum: datumId };
}

/** A short human label for a raw EditCommand (status-bar hint). */
export function editCommandLabel(cmd: WireEditCommand): string {
  switch (cmd.cmd) {
    case "editOperationInput":
    case "updateOperationParams":
      return "Repair reference";
    case "removeOperation":
      return "Delete feature";
    case "setRollback":
      return "Rollback";
    case "setOperationSuppression":
      return cmd.suppressed ? "Suppress" : "Unsuppress";
    case "setVisibility":
      return cmd.visible ? "Show" : "Hide";
    case "renameBody":
    case "renameSketch":
      return "Rename";
    case "addDatumPlane":
      return "Create datum plane";
    case "deleteDatum":
      return "Delete datum plane";
    default:
      return "Edit";
  }
}

/**
 * Parse a repair `refId` (`"<opId>.input<k>"`; SCHEMA §9) into its op id + input
 * slot index. Returns `null` when the shape does not match (the caller then treats
 * `k` as 0 / skips slot-targeting) so a backend format change fails soft.
 */
export function parseRefId(refId: string): { opId: string; index: number } | null {
  const m = /^(.*)\.input(\d+)$/.exec(refId);
  if (!m) return null;
  return { opId: m[1], index: Number(m[2]) };
}
