/*
 * previewOps — the ONE mapper from a live preview session to the canonical
 * `OperationOp` that both `PreviewOp` and the commit consume.
 *
 * SCHEMA §7.6 is explicit: "Preview callers MUST NOT maintain a second ad-hoc
 * mapper". The builders below ARE that single mapper. Each one mirrors its tool's
 * COMMIT call-site construction in `ModelToolController` field-for-field, so a
 * previewed candidate and the op that materializes it are the same op:
 *
 *   Extrude   ← the lane's own `buildOpFromSession` (moved here verbatim)
 *   Revolve   ← `confirmRevolve`
 *   Fillet    ← `commitFillet` (edgeOpKind "Fillet")
 *   Chamfer   ← `commitFillet` (edgeOpKind "Chamfer")
 *   Shell     ← `commitShell`
 *   Boolean   ← `commitBoolean`
 *   OffsetFace ← `commitOffsetFace`
 *
 * The two identity fields a preview adds on top of the commit-site shape are
 * `opId` (the stable RecordId reused by exact preview AND commit, so the backend
 * correlates them) and `featureId` (the re-edit target). Everything the WIRE sees
 * — `opType`, `inputs`, `params` — is byte-identical to the commit-site op.
 *
 * ── Why builders THROW instead of returning a partial op ─────────────────────
 * An invalid draft must not reach the kernel as a silently-degraded op. The lane
 * catches the throw and publishes a STRUCTURAL `PreviewFailure`, which blocks
 * commit (SCHEMA §7.6: "structural binding failures must be surfaced and must
 * disable commit"). Every guard below is either a mirror of the commit call-site's
 * own guard or a strictly-safer superset of it.
 *
 * ── Typed `inputs` matter for Fillet/Chamfer ONLY (read before "fixing" Shell) ─
 * `wireOperation` (tauriCommandMap.ts) DROPS an op's top-level `inputs`. The one
 * place it reads them is `filletParams(p, op.inputs)`, which synthesizes the typed
 * `params.edges` from them — and it does so ONLY when every edge input carries a
 * bodyId and the count matches `edgeIds` exactly. Shell and Boolean bodies ride
 * `params.targetBodyId` / `params.toolBodyId` instead; Rust's
 * `record.rs::derive_inputs` reads those to mint the op's semantic inputs on the
 * backend. So adding `inputs` to a Shell or Boolean preview changes NOTHING on the
 * wire — if one of those previews binds the wrong body, the bug is in the params,
 * never in a missing `inputs` array.
 */
import { REVOLVE_MIN_ANGLE } from "@/tools/modelTools/modelToolMachine";
import type {
  AxisRef,
  BooleanOperation,
  BooleanParams,
  ComponentParamValue,
  ExtrudeParams,
  FeatureBooleanMode,
  FilletParams,
  GearParams,
  GearFrame,
  HoleParams,
  HoleThread,
  HoleThreadDetail,
  HoleThreadStandard,
  HoleType,
  OffsetDistanceType,
  OffsetFaceParams,
  OperationOp,
  OpType,
  PlaceComponentParams,
  PlaceComponentSource,
  PreviewParams,
  RevolveParams,
  SemanticRef,
  ShellParams,
  TransformRotationParams,
} from "./types";

/**
 * The lane-session fields an op builder is allowed to read. The localSolver lane's
 * own session state extends this with its transport bookkeeping (pacing, in-flight
 * epochs, the mock lane's cached plane/profile) — none of which an op builder may
 * see, so this module stays PURE and unit-testable with a plain object literal.
 */
export interface PreviewSessionState {
  /** Stable RecordId shared by every exact preview of this session AND its commit. */
  opId: string;
  opType: OpType;
  /** Set ⇒ this session re-edits an existing feature (L1-only; never previewed). */
  editFeatureId?: string;
  sketchId?: string;
  regionId?: string;
  regionIdentityVersion?: number;
  /** Verbatim from `PreviewDraft.inputs` — see the Fillet/Chamfer note above. */
  inputs?: SemanticRef[];
  /** Newest complete immutable params snapshot the caller pushed. */
  latestParams: PreviewParams;
}

export type PreviewOpBuilder = (s: PreviewSessionState) => OperationOp;

// ── shared param coercion ────────────────────────────────────────────────────

/** A non-empty string, or undefined — an empty body id must never bind. */
function nonEmptyString(v: unknown): string | undefined {
  return typeof v === "string" && v.length > 0 ? v : undefined;
}

/** A fresh array of the string entries in `v` (rejects a non-array outright). */
function stringList(v: unknown, what: string): string[] {
  if (!Array.isArray(v)) throw new Error(`${what} must be an array of ids`);
  return v.map((entry) => {
    if (typeof entry !== "string" || entry.length === 0) {
      throw new Error(`${what} contains a non-string id`);
    }
    return entry;
  });
}

const FEATURE_BOOLEAN_MODES = ["NewBody", "Add", "Cut", "Intersect"] as const;

/** Validate the feature-fused boolean mode shared by Extrude + Revolve. */
function featureBooleanMode(op: string, raw: unknown): FeatureBooleanMode {
  if (raw === undefined) return "NewBody";
  if (!FEATURE_BOOLEAN_MODES.includes(raw as FeatureBooleanMode)) {
    throw new Error(`Unsupported ${op} booleanMode ${String(raw)}`);
  }
  return raw as FeatureBooleanMode;
}

// ── Extrude ──────────────────────────────────────────────────────────────────

/**
 * Build the concrete Extrude op a committed preview session materializes.
 *
 * Moved VERBATIM out of `localSolver.buildOpFromSession` (EXTRUDE-REGION-PARITY):
 * the extrude wire form is golden-pinned by `preview_and_commit_share_profile_and_body_lowering`
 * plus the whole extrude e2e set, so this body must not drift.
 */
function extrudeOp(s: PreviewSessionState): OperationOp {
  if (!s.sketchId) throw new Error("Extrude operation requires sketchId");
  if (!s.regionId) throw new Error("Extrude operation requires regionId");
  const distance = Number(s.latestParams.distance);
  if (!Number.isFinite(distance)) throw new Error("Extrude distance must be finite");
  const rawMode = s.latestParams.booleanMode;
  if (
    rawMode !== undefined &&
    rawMode !== "NewBody" &&
    rawMode !== "Add" &&
    rawMode !== "Cut" &&
    rawMode !== "Intersect"
  ) {
    throw new Error(`Unsupported Extrude booleanMode ${String(rawMode)}`);
  }
  const booleanMode: ExtrudeParams["booleanMode"] = rawMode ?? "NewBody";
  const targetBodyId =
    typeof s.latestParams.targetBodyId === "string" && s.latestParams.targetBodyId
      ? s.latestParams.targetBodyId
      : undefined;
  if (booleanMode !== "NewBody" && !targetBodyId) {
    throw new Error(`${booleanMode} Extrude requires targetBodyId`);
  }

  const END_MODES = ["Blind", "ThroughAll", "Symmetric", "ToNext", "ToFace"] as const;
  const rawEnd = s.latestParams.extrudeMode ?? "Blind";
  if (!END_MODES.includes(rawEnd as (typeof END_MODES)[number])) {
    throw new Error(`Unsupported Extrude mode ${String(rawEnd)}`);
  }
  const extrudeMode = rawEnd as (typeof END_MODES)[number];
  const rawEnd2 = s.latestParams.extrudeMode2 ?? "Blind";
  if (!END_MODES.includes(rawEnd2 as (typeof END_MODES)[number])) {
    throw new Error(`Unsupported second Extrude mode ${String(rawEnd2)}`);
  }
  const extrudeMode2 = rawEnd2 as (typeof END_MODES)[number];
  const twoDirections = s.latestParams.twoDirections === true;
  const draftAngleDeg = Number(s.latestParams.draftAngleDeg ?? 0);
  if (!Number.isFinite(draftAngleDeg)) throw new Error("Extrude draft angle must be finite");
  const distance2 = Number(s.latestParams.distance2 ?? 0);
  if (twoDirections && !Number.isFinite(distance2)) {
    throw new Error("Second Extrude distance must be finite");
  }
  if (extrudeMode === "Symmetric" && twoDirections) {
    throw new Error("Symmetric Extrude cannot use two directions");
  }
  if (extrudeMode === "ToFace" && !s.latestParams.targetFace) {
    throw new Error("ToFace Extrude requires targetFace");
  }
  if (twoDirections && extrudeMode2 === "ToFace" && !s.latestParams.targetFace2) {
    throw new Error("Second ToFace Extrude requires targetFace2");
  }

  const params: ExtrudeParams = {
    distance,
    extrudeMode,
    booleanMode,
    draftAngleDeg,
    twoDirections,
    extrudeMode2,
    distance2,
  };
  if (targetBodyId) params.targetBodyId = targetBodyId;
  if (twoDirections && extrudeMode2 === "ToFace") {
    params.targetFace2 = s.latestParams.targetFace2 as ExtrudeParams["targetFace2"];
  }
  if (extrudeMode === "ToFace") {
    params.targetFace = s.latestParams.targetFace as ExtrudeParams["targetFace"];
  }
  return {
    opType: "Extrude",
    opId: s.opId,
    sketchId: s.sketchId,
    regionId: s.regionId,
    regionIdentityVersion: s.regionIdentityVersion,
    featureId: s.editFeatureId,
    inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
    params,
  };
}

// ── Revolve ──────────────────────────────────────────────────────────────────

/**
 * Mirrors `ModelToolController.confirmRevolve`'s op: the same placeholder profile
 * input, the same `{angleDeg, axis, booleanMode, targetBodyId?}` params.
 *
 * The degenerate-angle guard mirrors `revolveStep`'s own refusal to confirm below
 * `REVOLVE_MIN_ANGLE` — a zero-thickness revolve is not a preview worth computing.
 * The boolean-target guard is the Extrude builder's, applied here too: an
 * Add/Cut/Intersect with no target would reach the worker as an unbound op.
 */
function revolveOp(s: PreviewSessionState): OperationOp {
  if (!s.sketchId) throw new Error("Revolve operation requires sketchId");
  if (!s.regionId) throw new Error("Revolve operation requires regionId");
  const angleDeg = Number(s.latestParams.angleDeg);
  if (!Number.isFinite(angleDeg)) throw new Error("Revolve angle must be finite");
  if (Math.abs(angleDeg) < REVOLVE_MIN_ANGLE) {
    throw new Error(`Revolve angle must be at least ${REVOLVE_MIN_ANGLE}°`);
  }
  const rawAxis = s.latestParams.axis as AxisRef | undefined;
  if (rawAxis !== undefined && rawAxis.kind !== "sketchLine" && rawAxis.kind !== "edge") {
    throw new Error(
      `Unsupported Revolve axis ${String((rawAxis as { kind?: unknown }).kind)}`,
    );
  }
  const axis: RevolveParams["axis"] = rawAxis;
  const booleanMode = featureBooleanMode("Revolve", s.latestParams.booleanMode);
  const targetBodyId = nonEmptyString(s.latestParams.targetBodyId);
  if (booleanMode !== "NewBody" && !targetBodyId) {
    throw new Error(`${booleanMode} Revolve requires targetBodyId`);
  }
  return {
    opType: "Revolve",
    opId: s.opId,
    sketchId: s.sketchId,
    regionId: s.regionId,
    regionIdentityVersion: s.regionIdentityVersion,
    featureId: s.editFeatureId,
    inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
    params: { angleDeg, axis, booleanMode, ...(targetBodyId ? { targetBodyId } : {}) },
  };
}

// ── Fillet / Chamfer (one gesture, one params struct, two opTypes) ───────────

/**
 * Mirrors `ModelToolController.commitFillet` for either `edgeOpKind`.
 *
 * DUAL-FIELD LOCKSTEP (the trap this guard exists for): `commitFillet` derives
 * `params.edgeIds` and `op.inputs` from the SAME `EntityRef[]` in the SAME order,
 * and `filletParams` silently DROPS the typed `params.edges` unless
 * `edgeInputs.length === edgeIds.length` and every input carries a bodyId. A
 * bare-`edgeIds` fillet derives no body and the worker fails with "Fillet requires
 * body input". The lane never sees the `EntityRef[]`, so the invariant is enforced
 * here instead: a draft whose `inputs` are not in lockstep with `params.edgeIds` is
 * REFUSED loudly rather than previewed as an op that will fail at the kernel.
 */
function edgeOpBuilder(kind: "Fillet" | "Chamfer"): PreviewOpBuilder {
  return (s) => {
    const radius = Number(s.latestParams.radius);
    if (!Number.isFinite(radius) || radius <= 0) {
      throw new Error(`${kind} radius must be greater than zero`);
    }
    const edgeIds = stringList(s.latestParams.edgeIds, `${kind} edgeIds`);
    if (edgeIds.length === 0) throw new Error(`${kind} requires at least one edge`);
    const inputs = [...(s.inputs ?? [])];
    if (inputs.length !== edgeIds.length) {
      throw new Error(
        `${kind} requires one edge input per edgeId (${inputs.length} inputs for ${edgeIds.length} edgeIds)`,
      );
    }
    if (inputs.some((r) => r.primary.kind !== "edge" || !r.primary.bodyId)) {
      throw new Error(`${kind} edge inputs must each carry an edge bodyId`);
    }
    const params: FilletParams = {
      mode: kind,
      radius,
      edgeIds,
      chainTangentEdges: s.latestParams.chainTangentEdges ?? true,
    };
    // SCHEMA §7.3 (2026-08-03) two-distance chamfer. STRUCTURAL guards, mirroring
    // core: it is Chamfer-only, and it must be a positive finite length — a draft
    // that says otherwise must fail the preview loudly rather than reach the
    // kernel as a silently-degraded equal-leg cut.
    const d2 = s.latestParams.distance2;
    if (d2 !== undefined && d2 !== null) {
      if (kind !== "Chamfer") throw new Error("distance2 is Chamfer-only (SCHEMA §7.3)");
      const n = Number(d2);
      if (!Number.isFinite(n) || n <= 0) {
        throw new Error("Chamfer distance2 must be greater than zero");
      }
      params.distance2 = n;
    }
    // The distance-angle mode (SCHEMA §7.3). Same structural guards, plus the
    // EXCLUSION core enforces by name: a draft carrying both modes is refused here
    // rather than sent as an op the backend will reject.
    const angle = s.latestParams.angleDeg;
    if (angle !== undefined && angle !== null) {
      if (kind !== "Chamfer") throw new Error("angleDeg is Chamfer-only (SCHEMA §7.3)");
      if (params.distance2 !== undefined) {
        throw new Error("Chamfer may carry distance2 or angleDeg, not both (SCHEMA §7.3)");
      }
      const a = Number(angle);
      if (!Number.isFinite(a) || a <= 0 || a >= 180) {
        throw new Error("Chamfer angleDeg must be strictly between 0 and 180 degrees");
      }
      params.angleDeg = a;
    }
    return { opType: kind, opId: s.opId, featureId: s.editFeatureId, inputs, params };
  };
}

// ── Shell ────────────────────────────────────────────────────────────────────

/**
 * Mirrors `ModelToolController.commitShell`. `targetBodyId` is the shelled body
 * (the call site takes it from the first picked face); the open faces are the
 * removed ones. `inputs` carries their semantic evidence so `wireOperation`
 * persists typed `faces` in lockstep with `openFaces`.
 */
function shellOp(s: PreviewSessionState): OperationOp {
  const thickness = Number(s.latestParams.thickness);
  if (!Number.isFinite(thickness) || thickness <= 0) {
    throw new Error("Shell thickness must be greater than zero");
  }
  const openFaces = stringList(s.latestParams.openFaces, "Shell openFaces");
  if (openFaces.length === 0) throw new Error("Shell requires at least one open face");
  const targetBodyId = nonEmptyString(s.latestParams.targetBodyId);
  if (!targetBodyId) throw new Error("Shell requires targetBodyId");
  const params: ShellParams = { thickness, openFaces, targetBodyId };
  return {
    opType: "Shell",
    opId: s.opId,
    featureId: s.editFeatureId,
    inputs: [...(s.inputs ?? [])],
    params,
  };
}

// ── Hole ─────────────────────────────────────────────────────────────────────

const HOLE_TYPES = ["simple", "counterbore", "countersink"] as const;
/** SCHEMA §7.3: the countersink included angles the backend admits. */
const HOLE_CS_ANGLES = [82, 90, 100, 120];
/** SCHEMA §7.3 `Hole.thread.detail` (WP-T1) — the closed vocabulary. */
const HOLE_THREAD_DETAILS: readonly HoleThreadDetail[] = ["cosmetic", "simplified", "modeled"];

/** A positive finite dimension, or a throw naming the field (the house rule). */
function positiveDim(v: unknown, what: string): number {
  const n = Number(v);
  if (!Number.isFinite(n) || n <= 0) throw new Error(`${what} must be greater than zero`);
  return n;
}

/**
 * Mirrors `gearParamsOf` (`tools/modelTools/gearMachine.ts`) — SCHEMA §7.6's
 * "preview callers MUST NOT maintain a second ad-hoc mapper" applies, so this
 * restates the SAME rules the FSM builder applies rather than inventing any:
 * exactly one placement mode, the active recipe block only, every inactive
 * block spelled `null`, and bore dimensions gated on their own flag.
 *
 * A Gear has NO target body — it mints one — so unlike every other builder here
 * there is no `targetBodyId` to demand.
 */
function gearOp(s: PreviewSessionState): OperationOp {
  const recipe = s.latestParams.recipe;
  if (recipe !== "involuteExternal") {
    throw new Error(`Unsupported Gear recipe ${String(recipe)}`);
  }
  const placement = s.latestParams.placement as
    | { face?: unknown; frame?: unknown; point?: unknown }
    | undefined;
  if (!placement) throw new Error("Gear requires a placement");

  const point = placement.point;
  if (!Array.isArray(point) || point.length !== 3 || point.some((c) => !Number.isFinite(c))) {
    throw new Error("Gear requires a [x, y, z] world point");
  }
  const face = (placement.face ?? null) as SemanticRef | null;
  const frame = (placement.frame ?? null) as GearFrame | null;
  if (face === null && frame === null) {
    throw new Error("Gear requires a placement face or frame");
  }
  if (face !== null && frame !== null) {
    throw new Error("Gear placement cannot carry both a face and a frame");
  }
  if (face !== null && !face.primary?.bodyId) {
    throw new Error("Gear placement face requires a body");
  }

  const inv = s.latestParams.involuteExternal as Record<string, unknown> | undefined;
  if (!inv) throw new Error("Gear requires its involuteExternal block");

  const teeth = Number(inv.teeth);
  if (!Number.isFinite(teeth) || teeth < 3) throw new Error("Gear teeth must be at least 3");
  const num = (v: unknown, fallback: number): number =>
    typeof v === "number" && Number.isFinite(v) ? v : fallback;
  const flag = (v: unknown): boolean => v === true;
  const axleHole = flag(inv.axleHole);
  const offsetHole = flag(inv.offsetHole);

  const params: GearParams = {
    recipe: "involuteExternal",
    placement: {
      face,
      frame: frame === null ? null : { ...frame },
      point: [point[0], point[1], point[2]] as [number, number, number],
    },
    involuteExternal: {
      teeth: Math.round(teeth),
      module: positiveDim(inv.module, "Gear module"),
      height: positiveDim(inv.height, "Gear height"),
      pressureAngleDeg: positiveDim(inv.pressureAngleDeg, "Gear pressureAngleDeg"),
      shift: num(inv.shift, 0),
      // Refused by the session and the worker in this version; emitted as the
      // only values they accept so the payload shape is already correct.
      helixAngleDeg: 0,
      doubleHelix: false,
      propertiesFromTool: flag(inv.propertiesFromTool),
      undercut: flag(inv.undercut),
      backlash: num(inv.backlash, 0),
      clearance: num(inv.clearance, 0.25),
      head: num(inv.head, 0),
      sampleCount: Math.max(2, Math.round(num(inv.sampleCount, 20))),
      axleHole,
      axleHoleDiameter: axleHole ? positiveDim(inv.axleHoleDiameter, "Gear axleHoleDiameter") : null,
      offsetHole,
      offsetHoleDiameter: offsetHole
        ? positiveDim(inv.offsetHoleDiameter, "Gear offsetHoleDiameter")
        : null,
      offsetHoleOffset: offsetHole
        ? positiveDim(inv.offsetHoleOffset, "Gear offsetHoleOffset")
        : null,
    },
  };
  return { opType: "Gear", opId: s.opId, params, ...(s.inputs ? { inputs: s.inputs } : {}) };
}

/**
 * Mirrors `ModelToolController.commitHole`. The conditional `cb*`/`cs*` blocks are
 * emitted ONLY for their own `holeType` and spelled `null` otherwise — the same
 * rule `holeParamsOf` applies at the call site, restated here because this builder
 * is what the wire actually sees and the Rust session REJECTS a simple hole
 * carrying a counterbore.
 */
function holeOp(s: PreviewSessionState): OperationOp {
  const holeType = s.latestParams.holeType;
  if (!HOLE_TYPES.includes(holeType as HoleType)) {
    throw new Error(`Unsupported Hole holeType ${String(holeType)}`);
  }
  const targetBodyId = nonEmptyString(s.latestParams.targetBodyId);
  if (!targetBodyId) throw new Error("Hole requires targetBodyId");
  const face = s.latestParams.face as SemanticRef | undefined;
  if (!face?.primary?.bodyId) throw new Error("Hole requires a host face ref");
  const point = s.latestParams.point;
  if (!Array.isArray(point) || point.length !== 3 || point.some((c) => !Number.isFinite(c))) {
    throw new Error("Hole requires a [x, y, z] world point");
  }
  const diameter = positiveDim(s.latestParams.diameter, "Hole diameter");
  const resultPolicyVersion = s.latestParams.resultPolicyVersion;
  if (resultPolicyVersion !== undefined && resultPolicyVersion !== 2) {
    throw new Error(`Unsupported Hole resultPolicyVersion ${String(resultPolicyVersion)}`);
  }
  // `null`/absent depth is THROUGH-ALL — a real end condition, so it must not be
  // coerced through `positiveDim`.
  const rawDepth = s.latestParams.depth;
  const depth = rawDepth === null || rawDepth === undefined ? null : positiveDim(rawDepth, "Hole depth");

  const params: HoleParams = {
    targetBodyId,
    face,
    point: [point[0], point[1], point[2]] as [number, number, number],
    holeType: holeType as HoleType,
    diameter,
    depth,
    cbDiameter: null,
    cbDepth: null,
    csDiameter: null,
    csAngleDeg: null,
    ...(resultPolicyVersion === 2 ? { resultPolicyVersion: 2 as const } : {}),
  };

  if (params.holeType === "counterbore") {
    params.cbDiameter = positiveDim(s.latestParams.cbDiameter, "Hole cbDiameter");
    params.cbDepth = positiveDim(s.latestParams.cbDepth, "Hole cbDepth");
    if (params.cbDiameter <= diameter) {
      throw new Error("Hole cbDiameter must exceed diameter");
    }
  } else if (params.holeType === "countersink") {
    params.csDiameter = positiveDim(s.latestParams.csDiameter, "Hole csDiameter");
    const angle = Number(s.latestParams.csAngleDeg);
    if (!HOLE_CS_ANGLES.includes(angle)) {
      throw new Error(`Hole csAngleDeg must be one of ${HOLE_CS_ANGLES.join(", ")}`);
    }
    params.csAngleDeg = angle;
    if (params.csDiameter <= diameter) {
      throw new Error("Hole csDiameter must exceed diameter");
    }
  }

  // `thread` (WP-T1) is orthogonal to `holeType` — structural guards only
  // (Rust owns the authoring bounds; the worker range-checks nothing here
  // except `detail`, which it preflight-refuses rather than silently ignores).
  const thread = s.latestParams.thread as HoleThread | undefined;
  if (thread) {
    if (thread.standard !== "ISO261") {
      throw new Error(`Unsupported Hole thread.standard ${String(thread.standard)}`);
    }
    if (typeof thread.designation !== "string" || thread.designation.length === 0) {
      throw new Error("Hole thread.designation must be non-empty");
    }
    if (!HOLE_THREAD_DETAILS.includes(thread.detail)) {
      throw new Error(`Unsupported Hole thread.detail ${String(thread.detail)}`);
    }
    params.thread = {
      standard: thread.standard as HoleThreadStandard,
      designation: thread.designation,
      majorDiameterMm: positiveDim(thread.majorDiameterMm, "Hole thread.majorDiameterMm"),
      pitchMm: positiveDim(thread.pitchMm, "Hole thread.pitchMm"),
      depthMm:
        thread.depthMm === null || thread.depthMm === undefined
          ? null
          : positiveDim(thread.depthMm, "Hole thread.depthMm"),
      detail: thread.detail,
    };
  }

  return {
    opType: "Hole",
    opId: s.opId,
    featureId: s.editFeatureId,
    inputs: [...(s.inputs ?? [])],
    params,
  };
}

// ── OffsetFace ───────────────────────────────────────────────────────────────

const OFFSET_DISTANCE_TYPES = ["Offset", "Total", "Radius", "Diameter"] as const;

/**
 * Mirrors `ModelToolController.offsetFaceParams`. SCHEMA §7.3 `op.offsetFace`.
 *
 * Every guard below restates one of core `OffsetFaceParams::validate`'s, so a
 * draft the Rust session would reject dies HERE as a structural preview failure
 * (which blocks the ✓ and names the reason) instead of reaching the kernel:
 *
 *  • the operative set is non-empty and every entry is a FACE carrying a bodyId —
 *    `faceIds`/`faces` lockstep is built from these at the marshalling seam, and a
 *    ref with no body derives no target;
 *  • `targetBodyId` equals EVERY face's body. Cross-body dies structurally rather
 *    than as an OCCT "foreign face silently ignored" (probe-confirmed) no-op;
 *  • a non-`Offset` type operates on exactly ONE face;
 *  • `Total` carries its opposite face and has the tangent chain OFF, and no other
 *    type carries one — the conditional is checked BOTH ways (the HoleParams
 *    doctrine) so a stale opposite left by a type switch cannot ride along;
 *  • an absolute distance is positive. Core only demands this for Radius/Diameter;
 *    a `Total` thickness ≤ 0 has no geometry either, so this is a deliberate,
 *    strictly-safer superset.
 *
 * NOTHING IS CLAMPED. A value outside the domain is refused, never corrected.
 *
 * `resultPolicyVersion` + `primaryFaceIds` are FORWARDED, not dropped. They are
 * not decoration: at V3 the version selects the blend-aware kernel path, so a
 * preview built without them would render the V2 sweep of a body the commit
 * rebuilds through suppress→offset→reblend. That is a preview the user approves
 * and does not get — the one failure this whole lane exists to prevent.
 */
function offsetFaceOp(s: PreviewSessionState): OperationOp {
  const distance = Number(s.latestParams.distance);
  if (!Number.isFinite(distance)) throw new Error("Offset face distance must be finite");
  const rawType = s.latestParams.distanceType ?? "Offset";
  if (!OFFSET_DISTANCE_TYPES.includes(rawType as OffsetDistanceType)) {
    throw new Error(`Unsupported OffsetFace distanceType ${String(rawType)}`);
  }
  const distanceType = rawType as OffsetDistanceType;
  const faces = s.latestParams.faces;
  if (!Array.isArray(faces) || faces.length === 0) {
    throw new Error("Offset face requires at least one operative face");
  }
  const refs = faces as SemanticRef[];
  if (refs.some((r) => r?.primary?.kind !== "face" || !r.primary.bodyId)) {
    throw new Error("Offset face operative faces must each carry a face bodyId");
  }
  const targetBodyId = nonEmptyString(s.latestParams.targetBodyId);
  if (!targetBodyId) throw new Error("Offset face requires targetBodyId");
  if (refs.some((r) => r.primary.bodyId !== targetBodyId)) {
    throw new Error("Offset face operates on ONE body — every face must belong to targetBodyId");
  }
  if (distanceType !== "Offset" && refs.length !== 1) {
    throw new Error(`Offset face distanceType ${distanceType} operates on exactly one face`);
  }
  const chainTangentFaces = s.latestParams.chainTangentFaces !== false;
  const opposite = s.latestParams.oppositeFace as SemanticRef | undefined;
  if (distanceType === "Total") {
    if (!opposite?.primary?.bodyId) {
      throw new Error("Offset face distanceType Total requires an opposite face");
    }
    if (chainTangentFaces) {
      throw new Error("Offset face distanceType Total requires chainTangentFaces = false");
    }
  } else if (opposite) {
    throw new Error(`Offset face oppositeFace is Total-only (got a ${distanceType} offset)`);
  }
  if (distanceType !== "Offset" && distance <= 0) {
    throw new Error(`Offset face distanceType ${distanceType} needs a positive distance`);
  }
  const resultPolicyVersion = s.latestParams.resultPolicyVersion;
  if (
    resultPolicyVersion !== undefined &&
    resultPolicyVersion !== 2 &&
    resultPolicyVersion !== 3
  ) {
    throw new Error(`Unsupported OffsetFace resultPolicyVersion ${String(resultPolicyVersion)}`);
  }
  const primaryFaceIds = s.latestParams.primaryFaceIds;
  const params: OffsetFaceParams = {
    faces: refs,
    distance,
    distanceType,
    chainTangentFaces,
    targetBodyId,
  };
  if (resultPolicyVersion !== undefined) params.resultPolicyVersion = resultPolicyVersion;
  if (primaryFaceIds !== undefined) params.primaryFaceIds = [...primaryFaceIds];
  if (distanceType === "Total" && opposite) params.oppositeFace = opposite;
  return {
    opType: "OffsetFace",
    opId: s.opId,
    featureId: s.editFeatureId,
    inputs: [...(s.inputs ?? [])],
    params,
  };
}

// ── Boolean ──────────────────────────────────────────────────────────────────

const BOOLEAN_OPERATIONS = ["Union", "Cut", "Intersect"] as const;

/**
 * Mirrors `ModelToolController.commitBoolean` — including the two body inputs it
 * synthesizes from the target/tool ids rather than from a pick list. `target ===
 * tool` is refused: OCCT would either no-op or self-intersect, and a preview that
 * shows a body consuming itself is worse than none.
 */
function booleanOp(s: PreviewSessionState): OperationOp {
  const operation = s.latestParams.operation;
  if (!BOOLEAN_OPERATIONS.includes(operation as BooleanOperation)) {
    throw new Error(`Unsupported Boolean operation ${String(operation)}`);
  }
  const targetBodyId = nonEmptyString(s.latestParams.targetBodyId);
  const toolBodyId = nonEmptyString(s.latestParams.toolBodyId);
  if (!targetBodyId) throw new Error("Boolean requires targetBodyId");
  if (!toolBodyId) throw new Error("Boolean requires toolBodyId");
  if (targetBodyId === toolBodyId) {
    throw new Error("Boolean target and tool must be different bodies");
  }
  const params: BooleanParams = {
    operation: operation as BooleanOperation,
    targetBodyId,
    toolBodyId,
  };
  return {
    opType: "Boolean",
    opId: s.opId,
    featureId: s.editFeatureId,
    inputs: [
      { primary: { bodyId: targetBodyId, kind: "body" } },
      { primary: { bodyId: toolBodyId, kind: "body" } },
    ],
    params,
  };
}

// ── PlaceComponent (ghost preview only — see PlaceComponentParams) ──────────

/**
 * Mirrors the candidate transform `placementController.ts` computes
 * (`placementSolver.ts`) — NOT a commit-site builder (there is no commit
 * call-site for this op-type; see {@link OperationOp}'s `PlaceComponent` doc).
 *
 * `source` is the backend's own resolution (`CadClient.resolveComponentSource`),
 * carried through verbatim. WP-3.2 replaced a hardcoded generator source here:
 * a blob-backed component previewed as the generator stub's screw and committed
 * something else, and a ghost that disagrees with its own commit is worse than
 * no ghost.
 */
function placeComponentOp(s: PreviewSessionState): OperationOp {
  const componentId = nonEmptyString(s.latestParams.componentId);
  if (!componentId) throw new Error("PlaceComponent requires componentId");
  const componentVersion = nonEmptyString(s.latestParams.componentVersion);
  if (!componentVersion) throw new Error("PlaceComponent requires componentVersion");
  const componentRevision = nonEmptyString(s.latestParams.componentRevision);
  if (!componentRevision) throw new Error("PlaceComponent requires componentRevision");
  const source = placeComponentSource(s.latestParams.source);
  const translate = s.latestParams.translate;
  if (
    !Array.isArray(translate) ||
    translate.length !== 3 ||
    translate.some((c) => typeof c !== "number" || !Number.isFinite(c))
  ) {
    throw new Error("PlaceComponent requires a [x,y,z] translate");
  }
  const rawRotate = s.latestParams.rotate as TransformRotationParams | undefined;
  const rotate: TransformRotationParams = rawRotate ?? {
    center: [0, 0, 0],
    axis: [0, 0, 1],
    angleDeg: 0,
  };
  const params: PlaceComponentParams = {
    componentId,
    componentVersion,
    componentRevision,
    source,
    translate: translate as [number, number, number],
    rotate,
  };
  return { opType: "PlaceComponent", opId: s.opId, params };
}

/**
 * A generator source's free-parameter overrides (WP-A3), or `undefined` when
 * the draft carries none. Values are the wire's own scalar union
 * (`ComponentParamValue`); anything else is a caller bug and throws rather
 * than being dropped, because a silently-dropped size override previews one
 * screw and commits another.
 */
function componentParams(raw: unknown): Record<string, ComponentParamValue> | undefined {
  if (raw === undefined || raw === null) return undefined;
  if (typeof raw !== "object" || Array.isArray(raw)) {
    throw new Error("PlaceComponent source.params must be an object");
  }
  const out: Record<string, ComponentParamValue> = {};
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof value !== "number" && typeof value !== "string" && typeof value !== "boolean") {
      throw new Error(`PlaceComponent source.params.${key} must be a number, string or boolean`);
    }
    if (typeof value === "number" && !Number.isFinite(value)) {
      throw new Error(`PlaceComponent source.params.${key} must be finite`);
    }
    out[key] = value;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * Validates the draft's `source` without reinterpreting it: every field the
 * worker reads must be present and of the right shape, but nothing is defaulted
 * or substituted — an under-specified source must fail at the arm, where the
 * component is named, rather than silently previewing something else.
 */
function placeComponentSource(raw: unknown): PlaceComponentSource {
  if (typeof raw !== "object" || raw === null) {
    throw new Error("PlaceComponent requires a source");
  }
  const s = raw as Record<string, unknown>;
  const kind = nonEmptyString(s.kind);
  if (kind === "generator") {
    const generatorId = nonEmptyString(s.generatorId);
    if (!generatorId) throw new Error("PlaceComponent generator source requires generatorId");
    const generatorVersion = Number(s.generatorVersion);
    if (!Number.isFinite(generatorVersion)) {
      throw new Error("PlaceComponent generatorVersion must be finite");
    }
    const params = componentParams(s.params);
    return params
      ? { kind: "generator", generatorId, generatorVersion, params }
      : { kind: "generator", generatorId, generatorVersion };
  }
  if (kind === "embedded" || kind === "document") {
    const sha256 = nonEmptyString(s.sha256);
    if (!sha256) throw new Error(`PlaceComponent ${kind} source requires sha256`);
    const codec = nonEmptyString(s.codec);
    if (!codec) throw new Error(`PlaceComponent ${kind} source requires codec`);
    const brepFormat = s.brepFormat === undefined ? undefined : Number(s.brepFormat);
    if (brepFormat !== undefined && !Number.isFinite(brepFormat)) {
      throw new Error("PlaceComponent brepFormat must be finite");
    }
    if (kind === "embedded") return { kind, sha256, codec, brepFormat };
    const documentSha256 = nonEmptyString(s.documentSha256);
    if (!documentSha256) {
      throw new Error("PlaceComponent document source requires documentSha256");
    }
    return { kind, documentSha256, sha256, codec, brepFormat };
  }
  throw new Error(`PlaceComponent source.kind ${String(s.kind)} is unknown`);
}

/**
 * Every opType a preview session can be opened for. Membership IS the support
 * check — a tool becomes previewable by adding a builder here, never by a branch
 * in the lane. Absent entries (LinearPattern / CircularPattern / MirrorBody) are
 * chip-driven ghost previews with no kernel candidate.
 */
export const OP_BUILDERS: Partial<Record<OpType, PreviewOpBuilder>> = {
  Extrude: extrudeOp,
  Revolve: revolveOp,
  Fillet: edgeOpBuilder("Fillet"),
  Chamfer: edgeOpBuilder("Chamfer"),
  Shell: shellOp,
  Boolean: booleanOp,
  Hole: holeOp,
  Gear: gearOp,
  OffsetFace: offsetFaceOp,
  PlaceComponent: placeComponentOp,
};

/** True when a preview session can be opened for `opType` (OP_BUILDERS membership). */
export function supportsPreview(opType: OpType): boolean {
  return OP_BUILDERS[opType] !== undefined;
}

/** The canonical op for a session's newest params. THROWS on an invalid draft. */
export function buildPreviewOp(s: PreviewSessionState): OperationOp {
  const build = OP_BUILDERS[s.opType];
  if (!build) throw new Error(`Preview session does not support ${s.opType}`);
  return build(s);
}
