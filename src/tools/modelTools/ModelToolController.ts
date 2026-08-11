/*
 * ModelToolController — imperative glue for the three model tools (F-WP7),
 * mirroring the sketch-mode SketchController. Lives inside ViewportRoot (created
 * after the engine initializes, so it never runs in jsdom). It:
 *   - arms/cancels model tools from explicit selection; finishing a sketch asks
 *     the user to select one closed region before Extrude,
 *   - drives the two-level extrude preview: L1 unit prism + drag handle track the
 *     pointer at refresh rate (depth = pointer-ray ∩ plane normal), while the
 *     previewThrottle paces exact L2 meshes that swap in underneath,
 *   - reconciles epochs on commit: L1 is dropped only once the exact body for the
 *     final epoch is present in the scene (via MeshIngest.onBodyLoaded),
 *   - runs the fillet radius drag + chip and the boolean tool-body pick + ghost.
 *
 * The pure transition logic lives in modelToolMachine; the pure math in
 * tools/preview/*. This file is the imperative wiring only.
 */
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  AxisRef,
  BooleanOperation,
  FeatureRecord,
  FilletParams,
  HoleParams,
  OffsetCurrentDims,
  OffsetDistanceType,
  OffsetFaceEvidence,
  OffsetFaceParams,
  OperationOp,
  PrepareEdgeOpResult,
  PrepareOffsetFaceResult,
  PreviewDraft,
  PreviewFailure,
  PreviewParams,
  PreviewResult,
  PreviewSession,
  SemanticRef,
  ShellParams,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickHit } from "@/viewport/engine/Picker";
import { getDatumVisuals } from "@/modules/modeling/datumViewport";
import { bareBodyId, buildAddDatumPlane, updateScalarParamsCommand } from "@/ipc/tauriCommandMap";
import { mintUuid } from "@/ipc/sketchWireMap";
import { promoteOne } from "@/ipc/promote";
import { toFeatureMeta } from "@/ipc/projectionHydration";
import { setSketchVisible } from "@/features/tree/treeActions";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { datumGhostPlane } from "@/viewport/engine/DatumLayer";
import { geometricLabel, type PickablePlane } from "@/viewport/engine/PlanePicker";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { buildBodyObjects, getEntry, remove as removeMesh, swap as swapMesh } from "@/viewport/mesh/meshRegistry";
import type { MeshEntry } from "@/viewport/mesh/meshRegistry";
import { toolStore } from "@/stores/toolStore";
import { trace, traceWarn } from "@/debug/trace";
import { logDebug } from "@/debug/log";
import { viewportStore, type ViewportState } from "@/stores/viewportStore";
import { documentStore, nextAppliedOps, nextDatumName, type SketchMeta } from "@/stores/documentStore";
import { selectionStore, topoRefId, type EntityRef } from "@/stores/selectionStore";
import { toolChipStore, MODEL_TOOL_CHIP_ID, DEFAULT_CHIP_OFFSET_PX } from "@/stores/toolChipStore";
import {
  autoModeFor,
  hasMaterial,
  NO_MATERIAL,
  type MaterialSides,
  type SideProbe,
} from "./materialProbe";
import { profileFromRegion, profileBounds, type PrismProfile } from "@/tools/preview/prismPreview";
import { regionAtPoint } from "@/tools/preview/regionPick";
import { axisDepthFromRay, normalize, type Vec3 } from "@/tools/preview/depthProjection";
import {
  radiusFromDrag,
  radiusFromValueText,
  screenDragAxis,
  signedValueFromDrag,
  SCREEN_UP_AXIS,
  type ScreenAxis,
} from "@/tools/preview/filletRadius";
import { averageOutward, edgeOutward } from "@/tools/preview/edgeDirection";
import { axisSplitsRegion, type LatheAxis } from "@/tools/preview/lathePreview";
import { angleFromDrag, snapRevolveAngle, clampAngle, angleFromValueText } from "@/tools/preview/revolveAngle";
import { thicknessFromValueText } from "@/tools/preview/shellThickness";
import {
  distanceFromValueText,
  ghostOffsets,
  offsetAnchorFor,
  offsetAxisFor,
  DEFAULT_OFFSET_DISTANCE,
} from "@/tools/preview/faceOffset";
import { faceDrawRange } from "@/viewport/engine/HighlightLayer";
import type { GhostInstances } from "@/viewport/engine/GhostLayer";
import {
  holeFsmFromParams,
  holeInit,
  holeParamsOf,
  holeStep,
  type HoleEvent,
  type HoleFsm,
} from "./holeMachine";
import { holeStandardPatch } from "./holeStandards";
import { getToolApplicability, resolveTargetSketchId } from "./toolApplicability";
import type { HoleChipOpts } from "@/stores/toolChipStore";
import {
  measureAdd,
  measureInit,
  measureSummary,
  pickFromElementInfo,
  type MeasureState,
} from "./measureTool";
import { measureStore } from "@/stores/measureStore";
import {
  WORLD_AXIS,
  WORLD_PLANE_NORMAL,
  linearGhostTransforms,
  circularGhostTransforms,
  mirrorGhostTransforms,
  clampPatternCount,
  countFromValueText,
  placementMatrix,
  applyPlacementToPoint,
  type GhostTransform,
  type Mat4Rows,
} from "@/tools/preview/patternPreview";
import {
  alignPlacement,
  composePlacements,
  faceFrame,
  inversePlacement,
  transformFrame,
  type PlanarFaceFrame,
} from "@/tools/preview/alignSolve";
import {
  accumulateAngleDeg,
  axisDragDelta,
  planeDragDelta,
  ringAngleDeg,
  snapRotateDeg,
  snapTranslate,
  type PointerRay,
} from "@/tools/preview/transformDrag";
import { PreviewThrottle } from "@/tools/preview/previewThrottle";
import {
  booleanInit,
  booleanStep as booleanStepRaw,
  extrudeInit,
  extrudeStep as extrudeStepRaw,
  type ExtrudeEndCondition,
  filletInit,
  filletStep as filletStepRaw,
  revolveInit,
  revolveStep as revolveStepRaw,
  shellInit,
  shellStep as shellStepRaw,
  offsetFaceInit,
  offsetFaceStep as offsetFaceStepRaw,
  offsetCanConfirm,
  OFFSET_TYPES_CURVED,
  OFFSET_TYPES_PLANAR,
  linearPatternInit,
  linearPatternStep as linearPatternStepRaw,
  circularPatternInit,
  circularPatternStep as circularPatternStepRaw,
  mirrorInit,
  mirrorStep as mirrorStepRaw,
  transformInit,
  transformStep as transformStepRaw,
  transformParamsOf,
  transformValue,
  axisIndex,
  dominantAxis,
  isIdentityPlacement,
  regionSelectInit,
  regionSelectStep,
  DEFAULT_EXTRUDE_DEPTH,
  DEFAULT_CHAMFER_DISTANCE,
  DEFAULT_FILLET_RADIUS,
  DEFAULT_REVOLVE_ANGLE,
  DEFAULT_SHELL_THICKNESS,
  type BooleanFsm,
  type BooleanMode,
  type BooleanSeed,
  type EdgeOpKind,
  type ExtrudeFsm,
  type FilletFsm,
  type RevolveFsm,
  type ShellFsm,
  type OffsetFaceFsm,
  type LinearPatternFsm,
  type CircularPatternFsm,
  type MirrorFsm,
  type PatternAxis,
  type MirrorPlane,
  type AlignPhase,
  type TransformFsm,
  type TransformGrab,
  type TransformMode,
  type TransformSeed,
  type RegionSelectState,
} from "./modelToolMachine";
import { withPhaseLog } from "./fsmLog";
import type { FeatureBooleanMode, TransformBodyParams } from "@/ipc/types";

// ── Observability wrapping (DEV-OBSERVABILITY Wave F) ────────────────────────
// The eight phase-bearing reducers are wrapped ONCE here, so every call site
// below keeps its original name and every phase transition lands one `fsm`
// debug event. `regionSelectStep` has no `phase` and stays unwrapped.
const extrudeStep = withPhaseLog("extrude", extrudeStepRaw);
const filletStep = withPhaseLog("edgeOp", filletStepRaw);
const revolveStep = withPhaseLog("revolve", revolveStepRaw);
const booleanStep = withPhaseLog("boolean", booleanStepRaw);
const shellStep = withPhaseLog("shell", shellStepRaw);
const offsetFaceStep = withPhaseLog("offsetFace", offsetFaceStepRaw);
const linearPatternStep = withPhaseLog("linearPattern", linearPatternStepRaw);
const circularPatternStep = withPhaseLog("circularPattern", circularPatternStepRaw);
const mirrorStep = withPhaseLog("mirror", mirrorStepRaw);
const transformStep = withPhaseLog("transform", transformStepRaw);

const DRAG_PX = 4;


/** Set-equality over body ids (order-insensitive; ids are unique per selection). */
function sameIdSet(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const set = new Set(b);
  return a.every((id) => set.has(id));
}

/** A stored Scalar (`{value}`) or bare number, as a number. Defaults to 0. */
function storedScalar(v: unknown): number {
  if (typeof v === "number") return v;
  if (v && typeof v === "object" && typeof (v as { value?: unknown }).value === "number") {
    return (v as { value: number }).value;
  }
  return 0;
}

/**
 * A stored OPTIONAL Scalar: the number, or `null` when the key is absent/unusable.
 * Distinct from {@link storedScalar} because a skip-none wire field (chamfer
 * `distance2`) needs "absent" and "0" to stay different answers.
 */
function storedOptionalScalar(v: unknown): number | null {
  if (v === undefined || v === null) return null;
  const n = storedScalar(v);
  return Number.isFinite(n) && n > 0 ? n : null;
}

/** A stored `[x,y,z]`, or null when it is not three finite numbers. */
function storedVec3(v: unknown): Vec3 | null {
  if (!Array.isArray(v) || v.length !== 3) return null;
  const out = v.map(storedScalar);
  return out.every(Number.isFinite) ? [out[0], out[1], out[2]] : null;
}

/**
 * Rebuild the full placement authoring state from a stored `TransformBodyParams`
 * (the wire serde shape `get_operation_params` returns).
 *
 * Returns null — refusing to arm — when the record has no targets, or a rotation
 * axis that is neither a world axis nor a usable vector at all. The original
 * refusal was blanket ("not one of the three the picker offers"), because arming
 * anyway would have let a ✓ REWRITE the axis to a world one the user never
 * chose. WP-B W2.5 makes an off-axis rotation a NORMAL frontend product — an
 * align flushes two arbitrary faces about their normals' cross product — so a
 * free axis is now carried VERBATIM in `rotAxisVec` instead. That closes the
 * data-loss hole properly: the exact vector goes straight back out on the ✓, and
 * a blanket refusal here would instead break the fold rule (a second placement
 * on an aligned body would stack a second record rather than re-edit the one).
 */
function transformSeedFromParams(
  stored: Record<string, unknown> | undefined,
): { targets: string[]; seed: TransformSeed } | null {
  if (!stored) return null;
  const targets = Array.isArray(stored.targets) ? stored.targets.filter((t) => typeof t === "string") : [];
  if (targets.length === 0) return null;
  const translate = storedVec3(stored.translate) ?? [0, 0, 0];
  const rotate = (stored.rotate ?? {}) as Record<string, unknown>;
  const center = storedVec3(rotate.center) ?? [0, 0, 0];
  const angleDeg = storedScalar(rotate.angleDeg);
  const axisVec = storedVec3(rotate.axis);
  const worldAxis = matchWorldAxis(axisVec ?? undefined);
  // A zero-length / malformed axis is still refused: nothing can be rebuilt from
  // it, and guessing one would be exactly the rewrite this guard exists to stop.
  if (!worldAxis && (!axisVec || Math.hypot(axisVec[0], axisVec[1], axisVec[2]) < 1e-9)) return null;
  const rotAxisVec = worldAxis ? null : axisVec;
  const rotAxis = worldAxis ?? dominantAxis(axisVec!);
  // Open on the component that actually carries the placement, so a re-edit shows
  // the number the row's value text shows rather than a bare 0 on Move X.
  const movedAxis = (["X", "Y", "Z"] as const).find((a) => translate[axisIndex(a)] !== 0);
  // A free axis has no chip that can show its angle, so a rotation-only aligned
  // record still opens on Move — the honest surface, not a mislabelled one.
  const mode: TransformMode = !movedAxis && angleDeg !== 0 && !rotAxisVec ? "rotate" : "move";
  return {
    targets,
    seed: {
      mode,
      axis: mode === "rotate" ? rotAxis : (movedAxis ?? "X"),
      translate,
      angleDeg,
      rotAxis,
      rotAxisVec,
      center,
      copy: stored.copy === true,
    },
  };
}

/** Status hint for a committed placement — names what actually changed. */
function placementHint(p: TransformBodyParams): string {
  const moved = p.translate.some((c) => c !== 0);
  if (moved && p.rotate.angleDeg !== 0) return "Placed";
  if (p.rotate.angleDeg !== 0) return `Rotated ${p.rotate.angleDeg.toFixed(1)}°`;
  if (moved) return "Moved";
  return "Placement unchanged";
}

/**
 * Preview trailing floors, per OP COST (`PreviewThrottle.setTrailingMs`). One
 * throttle serves every tool, but a fillet rebuild and a shell hollowing are far
 * dearer than the prism sweep the 80ms default was tuned for, so each owner sets
 * its own floor at arm time. The edge-op DRAG floor drops back to the default
 * while the pointer owns the value (the user is watching it move), and is
 * restored on release.
 */
const EDGE_OP_TRAILING_MS = 160;
const EDGE_OP_DRAG_TRAILING_MS = 80;
/** Shell hollows the whole solid — the dearest of the three; one floor, no drag tier. */
const SHELL_TRAILING_MS = 200;
/**
 * OffsetFace rebuilds the solid through `BRepOffset_MakeOffset` — dearer than a
 * prism sweep, cheaper than a full hollow, so it takes the edge-op floor.
 */
const OFFSET_FACE_TRAILING_MS = 160;

/**
 * Status hint shown for as long as a hole is armed. It names the RE-CLICK
 * affordance, which has no other surface: nothing on screen suggests that
 * clicking the face again moves the hole rather than starting a second one.
 */
const HOLE_ARMED_HINT = "Hole: set the size, click again to move it · Enter or ✓ to apply";

/**
 * How many armed edges contribute to the drag-direction mean. The mean only has
 * to orient ONE screen axis, so a long tangent-chain selection must not walk
 * every mesh entry to compute it.
 */
const EDGE_OP_OUTWARD_SAMPLE = 8;

/** Seed offset for a fresh datum plane (mm) — matches DEFAULT_EXTRUDE_DEPTH's role. */
const DEFAULT_DATUM_OFFSET = 10;

/**
 * How long a commit waits for its new bodies to enter the scene (onBodyLoaded)
 * before finishing anyway. A committed body that never ingests — e.g. a hidden body
 * whose mesh is never fetched — must not hang the tool open forever (finding 8).
 */
let bodyLoadTimeoutMs = 4000;
/** Test seam: shrink the commit body-load wait so a never-loading body resolves fast. */
export function __setBodyLoadTimeoutForTests(ms: number): void {
  bodyLoadTimeoutMs = ms;
}

/**
 * Trailing-send floor for the Revolve lane. A revolve rebuild costs roughly what an
 * extrude prism does (one sweep on a throwaway head), so it keeps the throttle's own
 * 80 ms default — set EXPLICITLY at arm time because the throttle is shared and a
 * previous owner may have re-tuned it for a more expensive op.
 */
const REVOLVE_PREVIEW_TRAILING_MS = 80;

let exactPreviewTimeoutMs = 4000;
/** Test seam: shrink the final exact-preview barrier timeout. */
export function __setExactPreviewTimeoutForTests(ms: number): void {
  exactPreviewTimeoutMs = ms;
}

/** The presentation policy `viewportStore.setStatusHint` accepts (severity + stickiness). */
type StatusHintOpts = Parameters<ViewportState["setStatusHint"]>[1];

/**
 * `timedOut` distinguishes "the kernel confirmed this candidate" from "we gave up
 * waiting". The generic barrier treats BOTH as `ok` on purpose (the backend
 * re-validates a commit authoritatively, so the barrier must never wedge one
 * behind a preview that never answers) — but OffsetFace FAILS CLOSED on it: its
 * operative closure was frozen by a separate handshake, and committing one the
 * kernel never got to evaluate is exactly the "silently wrong" outcome the whole
 * authoring transaction exists to prevent.
 */
type ExactPreviewOutcome =
  | { ok: true; timedOut: boolean }
  | { ok: false; error: PreviewFailure };

/**
 * Result of the shared previewed-commit sequence (fillet/chamfer + shell).
 * `superseded` is NOT a failure: the tool moved on mid-await (cancel / tool
 * switch / dispose), so the caller must touch nothing.
 */
type PreviewCommitOutcome =
  | { kind: "ok"; res: ApplyOperationResult }
  | { kind: "failed"; reason: string }
  | { kind: "superseded" };

interface ExactPreviewWaiter {
  timer: ReturnType<typeof setTimeout>;
  resolve: (outcome: ExactPreviewOutcome) => void;
}

export interface ModelToolDeps {
  engine: ViewportEngine;
  client: CadClient;
  container: HTMLElement;
  /** MeshIngest.onBodyLoaded — fires when a committed body enters the scene. */
  onBodyLoaded: (cb: (bodyId: string) => void) => () => void;
  debug?: boolean;
}

type DragKind = "extrude" | "fillet" | "revolve" | "shell" | "offsetFace" | "transform" | null;

/**
 * One placement gizmo gesture (WP-B W2). Everything is captured AT GRAB and the
 * drag reports DIFFERENCES against it, which is what stops the body jumping to
 * the cursor on the first frame: the grab point is never the handle's origin.
 *
 * `raw*` hold the unsnapped values. Snapping the accumulated raw value each frame
 * (rather than accumulating snapped values) is what keeps a slow drag from
 * ratcheting — the alternative quantises the ERROR too and drifts.
 */
interface GizmoDragState {
  grab: TransformGrab;
  /** Gizmo origin at grab (the frozen pivot displaced by the placement so far). */
  origin: Vec3;
  /** The placement's translation at grab — the base every drag frame adds to. */
  startTranslate: Vec3;
  /** Axis-arrow drags: the scalar sampled at grab, subtracted out every frame. */
  grabScalar: number;
  /** Ring drags: unsnapped cumulative angle + the previous frame's wrapped angle. */
  rawAngle: number;
  prevAngle: number;
  /** Live unsnapped translation, so the snap never feeds back into itself. */
  rawTranslate: Vec3;
}

/** One pickable revolve axis candidate (a sketch line), plane (u,v) endpoints. */
interface AxisCandidate {
  id: string;
  a: [number, number];
  b: [number, number];
}

/**
 * In-flight multi-region pick for Revolve. Extrude bypasses this path and requires
 * exactly one persistent sketchRegion selection. The controller owns the pointer
 * (orbit suppressed)
 * until the user confirms a selection or Esc cancels. `session` is captured up-front
 * so the resolved region(s) flow straight into the armed state (revolve reuses its
 * entities for axis candidates) with no second `enterSketch`.
 *
 * Wave 2: this is now a MULTI-select — the user toggles N regions (each becomes a
 * separate op) and confirms via Enter / the region chip ✓ / a double-click on one
 * region (select-only + confirm accelerator). `select` is the pure reducer state.
 */
interface RegionPickState {
  /** Which tool consumes the confirmed regions. */
  kind: "extrude" | "revolve";
  sketchId: string;
  plane: SketchPlane;
  /** Pickable regions (those with an extrudable profile), rendered + hit-tested. */
  regions: SketchRegion[];
  session: SketchSession;
  editFeatureId?: string;
  /** The armed tool's seed value (extrude depth / revolve angle). */
  startValue: number;
  /** Multi-select reducer state (ordered, deduped region ids). */
  select: RegionSelectState;
  /** Sketch-centroid world anchor for the region-select chip. */
  chipWorld: Vec3;
}

/**
 * One open kernel-preview lane session, owned by whichever tool armed it
 * (`previewOwner`). Everything here is opType-BLIND: the session is keyed by its
 * lane `sessionId`, and the exact candidate bodies it publishes are ingested the
 * same way whatever op produced them.
 *
 * `region`/`profile` are the PROFILE-based ops' extras (Extrude/Revolve bind one
 * sketch region each, and the L1 prism/lathe is drawn from the profile). An op
 * that operates on existing solids — Fillet/Chamfer/Shell/Boolean — carries
 * neither, which is why both are optional.
 */
interface ToolPreviewSession {
  session: PreviewSession;
  draft: PreviewDraft;
  region?: SketchRegion;
  profile?: PrismProfile;
  /** Newest L2 epoch applied for THIS session (drops out-of-order per-session results). */
  lastAppliedEpoch: number;
  /** Mesh-registry ids for every exact candidate body in the newest result. */
  previewBodyIds: string[];
  /** Committed bodies hidden by this session's exact candidate. */
  replacedBodyIds: string[];
}

/** Which tool owns the currently open preview sessions (drives hints + params). */
type PreviewOwner = "extrude" | "revolve" | "edgeOp" | "shell" | "offsetFace" | "boolean" | "hole";

export class ModelToolController {
  private extrude: ExtrudeFsm = extrudeInit();
  private fillet: FilletFsm = filletInit();
  /**
   * Which edge op the shared lane is authoring. Fillet and Chamfer are the SAME
   * gesture over the same `FilletChamferParams` (SCHEMA §7.3), distinguished only
   * by `opType`/`mode` at the wire — so one lane with a discriminator, not two
   * copies of the arm/drag/commit path.
   *
   * The FSM OWNS it (FILLET-CHAMFER-UNIFY): the drag direction can re-type the op
   * mid-gesture, and a second copy of that decision here would be a second source
   * of truth for what the ✓ commits.
   */
  private get edgeOpKind(): EdgeOpKind {
    return this.fillet.edgeOp;
  }
  private boolean: BooleanFsm = booleanInit();
  private revolve: RevolveFsm = revolveInit();
  private shell: ShellFsm = shellInit();
  private linear: LinearPatternFsm = linearPatternInit();
  private circular: CircularPatternFsm = circularPatternInit();
  private mirror: MirrorFsm = mirrorInit();
  private transform: TransformFsm = transformInit();
  private readonly throttle = new PreviewThrottle<PreviewParams>({ trailingMs: 80 });

  // Kernel-preview context. One session per armed target (Extrude: one per region
  // — N==1 single-region, N in Wave 2 multi-select). The first session paces the
  // shared throttle. Only ONE tool owns the lane at a time.
  private previewSessions: ToolPreviewSession[] = [];
  /** The tool that armed the open sessions; null when none are open. */
  private previewOwner: PreviewOwner | null = null;
  /**
   * The owner's params snapshotter. `sendPreview` is opType-blind: it asks for the
   * CURRENT complete params here instead of taking a tool-specific argument, so a
   * new tool joins the lane by binding this at arm time and nothing else.
   */
  private previewParamsFn: (() => PreviewParams) | null = null;
  /**
   * True between an arm's first exact-preview request and its first answer
   * (result OR failure). Only set for owners with NO local L1 synthesis — an
   * extrude already shows its prism immediately, so a "Computing preview…" hint
   * there would contradict what the user is looking at.
   */
  private previewPending = false;
  /** True while OUR "Computing preview…" hint owns the status line (so we restore it). */
  private previewPendingHint = false;
  /**
   * The arm hint to restore when the "Computing preview…" state clears, for the
   * owners that have no `armHintFor` entry (edgeOp / shell). Set at arm time,
   * dropped with the sessions.
   */
  private previewArmHint: string | null = null;
  private plane: SketchPlane | null = null;
  private centroidWorld: Vec3 = [0, 0, 0];
  private normal: Vec3 = [0, 0, 1];
  /**
   * The extrude drag's grab basis: the depth the arm held at the press, and the
   * axis projection of the press itself. A frame reports
   * `extrudeStartDepth + (raw - extrudeGrabDepth)`, so the pointer's own travel
   * since the grab is the only thing that moves the value. Both stay 0 on the
   * `forceExtrudeGrab` path, which collapses that back to the absolute mapping.
   */
  private extrudeStartDepth = 0;
  private extrudeGrabDepth = 0;
  /**
   * Whether this arm has been grabbed yet. Until it has, the arrow draws BOTH
   * heads: the sign is genuinely undecided, and a single head pointing along
   * +normal would claim otherwise. Symmetric is deliberately NOT drawn two-way —
   * one glyph, one meaning.
   */
  private extrudeGrabbed = false;
  private lastArmedSketch: string | null = null;
  /** `lastArmedSketch`'s geometryToken at arm time — a change invalidates the arm. */
  private armedSketchToken: string | null = null;
  private previewMeshRev = 0;
  /** Failure for the newest exact candidate. Confirmation is blocked until cleared. */
  private previewFailure: PreviewFailure | null = null;
  /** Prevent a stale head from causing an unbounded automatic retry loop. */
  private stalePreviewRetryAttempted = false;
  /** Full wire params retained verbatim for scalar-only feature re-edit. */
  private extrudeStoredParams: Record<string, unknown> | undefined;
  /** Fresh commits wait for the matching exact candidate before materializing. */
  private readonly exactPreviewWaiters = new Map<string, ExactPreviewWaiter>();
  /** One-shot "choose Cut" hint fired on the first depth sign-flip while bodies exist. */
  /** Double-click accelerator bookkeeping (region multi-select: same region twice). */
  private lastRegionClickId: string | null = null;
  private lastRegionClickAt = 0;

  // Revolve multi-region pick. `armGen` is bumped on every arm request +
  // every tool/mode change, so a finishSketch/enterSketch result that returns AFTER
  // the user re-triggered (or cancelled) is dropped instead of clobbering state.
  private armGen = 0;
  private regionPick: RegionPickState | null = null;

  // Commit-generation token (MODEL-HARDEN Wave 1). Bumped at every confirm start,
  // cancel, tool switch, and mode change; every await in a confirm sequence
  // re-checks it so a superseded commit can never resurrect (finishExtrude a body
  // or re-arm a preview) after the tool moved on. Extends the armGen discipline
  // into commit time.
  private commitGen = 0;

  // Click-away commit (MODEL-HARDEN Wave 1). A window-level capture pointerdown/up
  // pair confirms an armed extrude/revolve on a true click outside the chip /
  // toolbar / inputs and off the depth/angle handle (an orbit drag never commits).
  private clickAwayArmed = false;
  private clickAwayDownX = 0;
  private clickAwayDownY = 0;

  // Edge-op (fillet / chamfer) context.
  private filletEdges: EntityRef[] = [];
  private filletDownX = 0;
  private filletDownY = 0;
  /**
   * SIGNED value at the grab (negated for a Chamfer). The gesture lives on ONE
   * number line through zero — positive is Fillet, negative is Chamfer — so a
   * drag that crosses back over zero re-types continuously instead of jumping.
   */
  private filletStartValue = DEFAULT_FILLET_RADIUS;
  /** Screen direction one world unit of "away from the body" points, resolved per grab. */
  private filletAxis: ScreenAxis = SCREEN_UP_AXIS;
  /** World outward direction for the armed edges (null = none honest enough to use). */
  private filletOutward: Vec3 | null = null;
  /** World point the drag axis is projected from (mean of the armed edge midpoints). */
  private filletAnchor: Vec3 = [0, 0, 0];
  /** Edge tangent, only for a SINGLE-edge arm (removed from the screen axis so a
   *  drag ALONG the edge is inert). Meaningless to average across edges. */
  private filletTangent: Vec3 | null = null;
  /**
   * Which tier produced {@link filletOutward}. `auto` type-flipping is armed ONLY
   * on "bisector": the bbox proxy points INTO material on a concave edge, so
   * trusting it there would silently author the wrong op.
   */
  private filletAxisSource: "bisector" | "bbox" | "screen" = "screen";
  /** No honest world axis to put a handle on (the offset-face rule): the tool
   *  falls back to claiming every viewport press as a screen-space value drag. */
  private get filletDegraded(): boolean {
    return this.filletAxisSource === "screen" || !this.filletOutward;
  }
  private filletEditFeatureId: string | undefined;
  /** Stored params of the fillet being re-edited (radius-only edit preserves edges). */
  private filletStoredParams: Record<string, unknown> | undefined;
  /** Document revision whose prepared tangent closure `filletEdges` represents. */
  private filletPreparedRevision: number | null = null;

  // Shell context (mirrors fillet: face selection + vertical thickness drag).
  private shellFaces: EntityRef[] = [];
  private shellDownY = 0;
  private shellStartThickness = DEFAULT_SHELL_THICKNESS;
  private shellEditFeatureId: string | undefined;
  /** Stored params of the shell being re-edited (thickness-only edit preserves faces). */
  private shellStoredParams: Record<string, unknown> | undefined;

  // ── OffsetFace context (SCHEMA §7.3 + §7.6) ────────────────────────────────
  //
  // Everything below the FSM is AUTHORING TRANSACTION state, not tool state: the
  // operative closure is computed once by `PrepareOffsetFace`, promoted to
  // Rust-minted ids, and then FROZEN. A commit that cannot prove it still holds
  // (see `offsetPreparedRevision`) refuses rather than writing a record whose
  // faces were resolved against a head that has since moved.
  private offsetFace: OffsetFaceFsm = offsetFaceInit();
  /** The USER's picks, retained so a type / tangent toggle can re-run prepare. */
  private offsetPicks: EntityRef[] = [];
  /** The FROZEN operative closure as typed refs — exactly what the record holds. */
  private offsetFaces: SemanticRef[] = [];
  /** Snapshot TopoKeys parallel to {@link offsetFaces} (local mesh lookups only). */
  private offsetTopoKeys: string[] = [];
  /** The `Total` opposite face's typed ref; undefined for every other type. */
  private offsetOppositeFace: SemanticRef | undefined;
  /** The one body the closure belongs to (bare id, as the selection carries it). */
  private offsetTargetBodyId = "";
  /** Planar frames of the closure, in `offsetFaces` order. Empty ⇒ degraded. */
  private offsetFrames: PlanarFaceFrame[] = [];
  /** Index slices for the L1 ghost, parallel to {@link offsetFrames}. */
  private offsetRanges: { start: number; count: number }[] = [];
  /** The mean-normal drag axis; null ⇒ no honest arrow (degraded gesture). */
  private offsetAxis: Vec3 | null = null;
  /** World anchor for the arrow + chip (mean of the operative face centres). */
  private offsetAnchor: Vec3 = [0, 0, 0];
  /** `currentDims` from the newest successful prepare (absolute-type seeds). */
  private offsetDims: OffsetCurrentDims = {};
  /**
   * The `documentStore.revision` the newest successful prepare answered against,
   * or null when there is no valid handshake. The COMMIT GATE: a closure frozen
   * against an older head names TopoKeys that may now designate different
   * topology, and no offset is better than the wrong one.
   */
  private offsetPreparedRevision: number | null = null;
  /** No frames / diverging normals ⇒ screen-space value drag, no arrow, no ghost. */
  private offsetDegraded = false;
  /** True once an exact candidate replaced the L1 ghost (re-shown on failure). */
  private offsetGhostHidden = false;
  /** Grab-delta bookkeeping: the value + axis depth captured AT the press. */
  private offsetStartDistance = DEFAULT_OFFSET_DISTANCE;
  private offsetGrabDepth = 0;
  private offsetDownX = 0;
  private offsetDownY = 0;
  private offsetEditFeatureId: string | undefined;
  /** Stored params of the offset being re-edited (distance-only edit keeps faces). */
  private offsetStoredParams: Record<string, unknown> | undefined;

  // Hole context (WP-C T3): the FSM holds every param; only identity lives here.
  private hole: HoleFsm = holeInit();
  private holeEditFeatureId: string | undefined;
  /**
   * The armed seat's snapshot TopoKey. Kept OUTSIDE the FSM (which stores only
   * the minted `SemanticRef`) purely so a re-click can recognise "same face" on a
   * body whose faces were never promoted — a pick carries a TopoKey, and the
   * minted ElementId is the only thing the record may hold.
   */
  private holeTopoKey: string | undefined;

  // Pattern / mirror context (chip-driven; ghost clones of the source body).
  private patternEditFeatureId: string | undefined;
  private patternResultPolicyVersion: 2 | undefined;
  private patternFuseResult = false;

  /**
   * The record a placement commit will REWRITE — either a fold target the backend
   * lineage query named, or the row a history re-edit opened. `undefined` appends
   * a fresh record. Kept here rather than in `TransformFsm` for the same reason
   * `patternEditFeatureId` is: the reducers are pure authoring state, and a record
   * id is identity, not authoring.
   */
  private transformEditFeatureId: string | undefined;
  /** Whether the placement ghost is currently hiding its source bodies. */
  private transformHidSources = false;
  /** In-flight gizmo drag (WP-B W2), null between gestures. */
  private gizmoDrag: GizmoDragState | null = null;
  /**
   * Whether THIS controller has put the gizmo on screen. Guards the teardown the
   * same way `transformHidSources` guards the ghost's: the tool-switch sweep runs
   * on every arm of every tool, and a placement that was never armed has nothing
   * to take down.
   */
  private transformGizmoShown = false;

  /**
   * The INVERSE of the placement the on-screen geometry already carries, frozen
   * at arm — identity on a fresh arm, the record's stored motion on a fold /
   * re-edit (WP-B W2.5). Face frames for the align solve are read off the mesh
   * registry, which shows the body as COMMITTED, while a record's `translate` /
   * `rotate` are relative to the geometry it CONSUMES. Pulling a frame back
   * through this is what keeps those two frames from being silently conflated.
   */
  private transformArmedInverse: Mat4Rows | null = null;
  /** The first align pick, in the record's INPUT frame; null between picks. */
  private alignMovingFrame: PlanarFaceFrame | null = null;
  /** Whether THIS controller currently owns the selection store's hover. */
  private alignHidHover = false;
  /** Same flag for the MODAL PICK phases' hover (see `setToolHover`). */
  private toolHoverSet = false;

  /**
   * Datum-plane tool context (DATUM W1), null when the tool is not armed.
   * `base === null` is the BASE-PICK phase (the plane picker owns the pointer);
   * a non-null base is the OFFSET phase (ghost + chip up, ✓ / Enter commits).
   */
  private datum: { base: PickablePlane | null; offset: number } | null = null;

  // Boolean re-edit context (operation swap only — the tool body is consumed).
  private booleanEditFeatureId: string | undefined;
  /** Stored params of the boolean being re-edited (target/tool ids survive the swap). */
  private booleanStoredParams: Record<string, unknown> | undefined;

  // Revolve context. `revolveProfile`/`revolveRegionId` are the PRIMARY region
  // (drives the L1 lathe preview); the arrays carry ALL selected regions for the
  // multi-region commit loop + all-regions axis validity (Wave 2). N==1 single-region.
  private revolveProfile: PrismProfile | null = null;
  private revolveProfiles: PrismProfile[] = [];
  private revolveRegionIds: string[] = [];
  private revolveRegionIdentityVersions: Array<number | undefined> = [];
  private revolveSketchId: string | null = null;
  private revolveRegionId: string | null = null;
  private revolveEditFeatureId: string | undefined;
  /** Stored params of the revolve being re-edited (angle-only edit preserves the axis). */
  private revolveStoredParams: Record<string, unknown> | undefined;
  private revolveAxisCandidates: AxisCandidate[] = [];
  private revolveAxis: LatheAxis | null = null;
  private revolveAxisLineId: string | null = null;
  private revolveArmedDown = false; // LMB pressed while armed (maybe an angle drag)
  private revolveDownX = 0;
  private revolveLastX = 0;
  private revolveStartAngle = DEFAULT_REVOLVE_ANGLE;
  private commitRevolveBodyUnsub: (() => void) | null = null;

  // Commit reconciliation.
  private commitBodyId: string | null = null;
  private commitFinalEpoch = 0;
  private committedEpoch = 0;
  private lastL2Epoch = 0;
  private commitBodyUnsub: (() => void) | null = null;
  /** Bounded-wait timers for the commit body-load reconcile (finding 8). */
  private commitBodyTimer: ReturnType<typeof setTimeout> | null = null;
  private commitRevolveBodyTimer: ReturnType<typeof setTimeout> | null = null;

  /** Last phase vector logged by {@link updateDebug} — dedup, see there. */
  private lastPhaseSig = "";

  // Pointer bookkeeping.
  private downX = 0;
  private downY = 0;
  private downButton = -1;
  private moved = false;
  private dragging: DragKind = null;
  private altHeld = false;

  private readonly unsubs: Array<() => void> = [];

  constructor(private readonly deps: ModelToolDeps) {
    const c = deps.container;
    c.addEventListener("pointerdown", this.onPointerDown);
    c.addEventListener("pointermove", this.onPointerMove);
    c.addEventListener("pointerup", this.onPointerUp);
    window.addEventListener("keydown", this.onKeyDown, true);
    window.addEventListener("keyup", this.onKeyUp, true);
    // Click-away commit — window capture so a click on ANY chrome (or empty space)
    // outside the container is seen; the handlers gate on the armed phase + target.
    window.addEventListener("pointerdown", this.onWindowPointerDown, true);
    window.addEventListener("pointerup", this.onWindowPointerUp, true);

    this.unsubs.push(deps.client.onPreviewResult((r) => this.onPreviewResult(r)));

    // React to model-mode tool changes + the finish-sketch handoff.
    let lastMode = toolStore.getState().mode;
    let lastTool = toolStore.getState().modelTool;
    this.unsubs.push(
      toolStore.subscribe((s) => {
        if (s.mode !== lastMode) {
          lastMode = s.mode;
          if (s.mode !== "model") this.cancelAll();
        }
        if (s.mode === "model" && s.modelTool !== lastTool) {
          lastTool = s.modelTool;
          this.onToolChange(s.modelTool);
        }
      }),
    );

    // An armed extrude is bound to the sketch geometry it was armed on; editing that
    // sketch changes its geometryToken and can invalidate the region the preview
    // sessions reference, so the arm is dropped instead of committing stale profiles.
    this.unsubs.push(documentStore.subscribe((s) => this.onSketchGeometryChanged(s.sketches)));

    // An armed EDGE OP / SHELL is bound to snapshot-scoped TopoKeys on the CURRENT
    // head. Any document change re-snapshots the model, so those keys may silently
    // designate different topology — fail closed and drop the arm (mirrors the
    // geometryToken guard above, which covers only the sketch-bound tools).
    let lastRevision = documentStore.getState().revision;
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.revision === lastRevision) return;
        lastRevision = s.revision;
        this.onDocumentRevisionChanged();
      }),
    );

    let lastPending = viewportStore.getState().pendingExtrudeSketch;
    this.unsubs.push(
      viewportStore.subscribe((s) => {
        if (s.pendingExtrudeSketch !== lastPending) {
          lastPending = s.pendingExtrudeSketch;
          if (s.pendingExtrudeSketch && toolStore.getState().mode === "model") {
            viewportStore.getState().setPendingExtrude(null);
            this.resetToSelect("Select one closed sketch region, then choose Extrude", {
              sticky: true,
            });
          }
        }
      }),
    );
  }

  /**
   * Drop an armed/dragging extrude — or an armed/dragging/axis-picking revolve —
   * whose source sketch was edited underneath it. A COMMITTING op is left alone:
   * it is already generation-gated and the backend re-validates it authoritatively.
   */
  private onSketchGeometryChanged(sketches: Record<string, SketchMeta>): void {
    const id = this.lastArmedSketch;
    if (!id || this.armedSketchToken === null) return;
    const token = sketches[id]?.geometryToken;
    if (token === undefined || token === this.armedSketchToken) return;
    this.armedSketchToken = token;
    // Revolve binds the same sketch geometry TWICE (the profile ring and the axis
    // line entity), so an edit underneath it invalidates both — drop the arm instead
    // of committing a stale profile / vanished axis (extrude parity).
    const revolvePhase = this.revolve.phase;
    if (revolvePhase === "armed" || revolvePhase === "dragging" || revolvePhase === "axisPick") {
      this.cancelRevolve();
      // Esc-ladder tail (the reset also bumps commitGen).
      this.resetToSelect("Sketch changed — revolve preview cancelled", {
        severity: "info",
        sticky: true,
      });
      return;
    }
    if (this.previewSessions.length === 0) return;
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "dragging") return;
    this.cancelPreview();
    // Esc-ladder tail (the reset also bumps commitGen).
    this.resetToSelect("Sketch changed — extrude preview cancelled", {
      severity: "info",
      sticky: true,
    });
  }

  /**
   * Drop an armed edge op / shell when the DOCUMENT moved underneath it.
   *
   * Both tools bind their selection as snapshot-scoped `TopoKey`s (`"e:5"` /
   * `"f:22"`) captured off the head that was on screen when the user picked. Any
   * committed edit, undo or redo re-snapshots the model and those indices can
   * designate entirely different topology — the exact silent-mis-bind the whole
   * migration exists to eliminate. So this fails CLOSED: cancel the arm and ask
   * for a fresh pick rather than preview (and then commit) against keys that may
   * no longer mean what the user clicked.
   *
   * A COMMITTING op is excluded: its own commit is what bumped the revision, and
   * it is already generation-gated. A parametric RE-EDIT is excluded too — it
   * carries no TopoKeys of its own (the stored params hold the original refs) and
   * transitions to `committing` before applying, for exactly this reason.
   */
  private onDocumentRevisionChanged(): void {
    const filletArmed = this.fillet.phase === "armed" || this.fillet.phase === "dragging";
    const shellArmed = this.shell.phase === "armed" || this.shell.phase === "dragging";
    if (filletArmed && !this.filletEditFeatureId) {
      this.cancelFillet();
      this.resetToSelect("Model changed — re-select edges", { severity: "info", sticky: true }); // Esc-ladder tail
      this.updateDebug();
      return;
    }
    if (shellArmed && !this.shellEditFeatureId) {
      this.cancelShell();
      this.resetToSelect("Model changed — re-select faces", { severity: "info", sticky: true });
      this.updateDebug();
      return;
    }
    // OffsetFace fails closed for the same reason and one more: its operative
    // closure was FROZEN by a handshake against the previous head, so a moved
    // document invalidates the whole authoring transaction, not just the picks.
    const offsetArmed =
      this.offsetFace.phase === "armed" || this.offsetFace.phase === "dragging";
    if (offsetArmed && !this.offsetEditFeatureId) {
      this.cancelOffsetFace();
      this.resetToSelect("Model changed — re-select faces", { severity: "info", sticky: true });
      this.updateDebug();
    }
  }

  // ── tool arming ────────────────────────────────────────────────────────────

  /**
   * Leave the active tool for `select` and publish the message the user must read —
   * in THAT order, always.
   *
   * `setTool` drives `onToolChange` SYNCHRONOUSLY, and that sweep ends on
   * `setStatusHint(null)` for the `select` tool. So a flow that publishes its hint
   * first and resets the tool afterwards loses the message entirely — the defect
   * class this helper exists to make unrepresentable. The contract it encodes:
   * the cancel sweep is idempotent and must never emit a hint of its own, so the
   * LAST word wins and the caller's message is what survives. Call with no `hint`
   * to reset and clear.
   */
  private resetToSelect(hint?: string, opts?: StatusHintOpts): void {
    toolStore.getState().setTool("select");
    viewportStore.getState().setStatusHint(hint ?? null, opts);
  }

  private onToolChange(tool: string): void {
    // Any tool switch invalidates a pending arm (drop a late finishSketch result) and
    // a pending commit (so it can't resurrect on the new tool), and cleans up the
    // previous tool's transient state first.
    this.invalidateArm();
    this.commitGen++;
    // Before the sweep: the per-tool cancels below reset the FSMs this hover was
    // keyed to, and `selectionStore` outlives every one of them.
    this.clearToolHover();
    this.cancelRegionPick();
    this.cancelPreview();
    this.cancelFillet();
    this.cancelBoolean();
    this.cancelRevolve();
    this.cancelShell();
    this.cancelOffsetFace();
    this.cancelHole();
    this.cancelPattern();
    this.cancelTransform();
    this.endDatumPick();
    this.cancelMeasure();
    toolChipStore.getState().clear();
    // W3: arming a real tool leaves isolation. An op previews against bodies the
    // isolate mask is hiding, and the preview's own hide/restore snapshot would
    // fight that mask. Deliberately NOT on the switch back to `select`, which is
    // a disarm — Esc's own ladder owns exiting isolation there. Placed AFTER the
    // cancel sweep (so no preview still holds a saved visibility snapshot) and
    // BEFORE the arms (so their status hints have the last word).
    if (tool !== "select") viewportStore.getState().exitIsolate();
    if (tool === "datum") this.startDatum();
    else if (tool === "extrude") this.armExtrudeFromSelection();
    else if (tool === "revolve") this.armRevolveFromSelection();
    // The unified edge tool seeds Fillet and lets the DRAG DIRECTION re-type it
    // (where the direction is honest — `armEdgeOpFromSelection` downgrades `auto`
    // off the bisector tier). The `chamfer` tool id is dead (FILLET-CHAMFER-UNIFY
    // W2) — Chamfer is reached only via drag-into or the chip segment now.
    else if (tool === "fillet") void this.armEdgeOpFromSelection("Fillet", { auto: true });
    else if (tool === "boolean") this.startBooleanFromSelection();
    else if (tool === "shell") void this.armShellFromSelection();
    else if (tool === "offsetFace") void this.armOffsetFaceFromSelection();
    else if (tool === "hole") this.startHole();
    else if (tool === "linearPattern") this.armLinearFromSelection();
    else if (tool === "circularPattern") this.armCircularFromSelection();
    else if (tool === "mirror") this.armMirrorFromSelection();
    else if (tool === "transform") this.armTransformFromSelection();
    else if (tool === "measure") this.armMeasure();
    else viewportStore.getState().setStatusHint(null);
  }

  // ── measure (W2-B) — READ ONLY ──────────────────────────────────────────────
  //
  // Measure is the only tool here that writes nothing: no preview session, no
  // FSM in `modelToolMachine`, no commit path, no history row. It owns a pure
  // reducer (`measureTool`) plus a display store, and everything below is
  // self-contained so the rest of this (heavily churned) controller is unaffected.
  //
  // Picks arrive from ViewportRoot rather than this file's own pointer handlers:
  // the engine's Picker already does the face/edge raycast for `select`, and
  // duplicating that raycast here would be a second source of truth for what is
  // under the cursor.

  private measure: MeasureState = measureInit();
  /** Bumped on every arm/cancel so a late `elementInfo` for a dead arm is dropped. */
  private measureGen = 0;

  private armMeasure(): void {
    this.measure = measureInit();
    this.measureGen++;
    measureStore.getState().clear();
    viewportStore
      .getState()
      .setStatusHint("Select a face or edge to measure", { sticky: true });
  }

  private cancelMeasure(): void {
    this.measure = measureInit();
    this.measureGen++;
    measureStore.getState().clear();
  }

  /**
   * Handle one face/edge pick while Measure is armed (called by ViewportRoot).
   *
   * Promote-if-missing first, mirroring `SketchController.tryEnterOnSelectedFace`:
   * ViewportRoot promotes every pick, but that is fire-and-forget and may not have
   * landed yet, so we await our own promotion rather than racing it. Both handles
   * are then sent — the backend needs the TopoKey for a fresh pick (the worker
   * mints partition entries only for op-referenced elements) and the ElementId for
   * one an operation already consumed.
   *
   * A `null` reply means the element is not in the current snapshot. That is an
   * ordinary outcome for a stale pick and the pick is DROPPED with a hint —
   * never recorded as a zero-magnitude element at the origin.
   */
  async measurePick(ref: EntityRef): Promise<void> {
    if (ref.kind !== "face" && ref.kind !== "edge") return;
    const bodyId = ref.bodyId;
    if (!bodyId) return;
    const gen = this.measureGen;

    let elementId = ref.elementId;
    if (!elementId && ref.topoKey) {
      // A refused promotion (stale snapshot) hints and leaves `elementId` unset —
      // the topoKey rung below still answers for a fresh pick (see `promoteOne`).
      const promoted = await promoteOne(this.client, bodyId, {
        topoKey: ref.topoKey,
        anchor: ref.anchor,
      });
      if (gen !== this.measureGen) return; // disarmed / re-armed mid-await
      elementId = promoted?.elementId;
    }
    if (!elementId && !ref.topoKey) return;

    const info = await this.client
      .elementInfo(bodyId, elementId ?? "", ref.topoKey)
      .catch((e: unknown) => {
        traceWarn("measure", "elementInfo failed", errMessage(e));
        return null;
      });
    if (gen !== this.measureGen) return;
    if (!info) {
      viewportStore.getState().setStatusHint("That element is no longer in the model", {
        severity: "info",
        sticky: true,
      });
      return;
    }

    this.measure = measureAdd(this.measure, pickFromElementInfo(bodyId, info));
    const summary = measureSummary(this.measure);
    measureStore.getState().set(this.measure.picks, summary);
    viewportStore
      .getState()
      .setStatusHint(
        summary
          ? "Pick another element to re-measure, or Esc to clear"
          : "Select a second face or edge to measure between them",
        { sticky: true },
      );
  }

  /** Whether Measure currently owns picks (ViewportRoot routes clicks on this). */
  isMeasureArmed(): boolean {
    return toolStore.getState().modelTool === "measure";
  }

  private armExtrudeFromSelection(): void {
    const selected = selectionStore.getState().selected;
    const verdict = getToolApplicability("extrude", selected, this.applicabilityCtx());
    const picked = selected.filter((ref) => ref.kind === "sketchRegion");
    if (picked.length > 0 && verdict.enabled) {
      void this.armExtrudeRegions(picked[0].sketchId, picked.map((ref) => ref.regionId));
      return;
    }
    // Nothing region-typed selected: tool-first UX. A selected sketch (or the
    // document's sole visible sketch) opens the region PICK.
    if (picked.length === 0) {
      const sketchId = this.pickTargetSketchId();
      if (sketchId) {
        void this.armExtrudePick(sketchId);
        return;
      }
    }
    viewportStore
      .getState()
      .setStatusHint(verdict.reason ?? null, { severity: verdict.severity, sticky: true });
  }

  /**
   * Arm extrude on the EXACT regions the selection names. Region acquisition is a
   * pure `getSketchRegions` read (no `finishSketch` side effect), so this works on
   * a reopened sketch with no live backend session and authors no timeline record
   * at arm time.
   */
  private async armExtrudeRegions(sketchId: string, regionIds: string[]): Promise<void> {
    const gen = ++this.armGen;
    let read: { regions: SketchRegion[] };
    try {
      read = await this.deps.client.getSketchRegions(sketchId);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Cannot read sketch regions: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) return;
    const bound: SketchRegion[] = [];
    const profiles: PrismProfile[] = [];
    for (const id of regionIds) {
      const region = read.regions.find((candidate) => candidate.regionId === id);
      const profile = region ? profileFromRegion(region) : null;
      if (!region || !profile) {
        const available = read.regions.map((candidate) => candidate.regionId).join(", ") || "none";
        this.resetToSelect(`Extrude region ${id} is stale or invalid; available: ${available}`, {
          severity: "error",
          sticky: true,
        });
        return;
      }
      bound.push(region);
      profiles.push(profile);
    }
    const session = await this.deps.client.getSketch(sketchId);
    if (gen !== this.armGen) return;
    await this.beginExtrudeArmed(sketchId, bound, profiles, session.plane);
  }

  /** The sketch a region pick targets when no region is selected yet: an explicitly
   *  selected sketch wins; else the document's SOLE visible sketch; else null. */
  private pickTargetSketchId(): string | null {
    return resolveTargetSketchId(selectionStore.getState().selected, this.applicabilityCtx());
  }

  /** Sketch-existence slice `toolApplicability.ts` needs for its extrude/revolve
   *  document-level fallback — kept as a private helper so every applicability
   *  call site derives it identically. */
  private applicabilityCtx(): { sketches: Record<string, { id: string; visible: boolean }> } {
    return { sketches: documentStore.getState().sketches };
  }

  /** Tool-first entry: fetch the sketch's regions and open the extrude region pick. */
  private async armExtrudePick(sketchId: string): Promise<void> {
    const gen = ++this.armGen;
    let finish: { regions: SketchRegion[] };
    try {
      finish = await this.deps.client.getSketchRegions(sketchId);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Cannot read sketch regions: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) return;
    await this.enterRegionPick("extrude", sketchId, finish.regions, undefined, DEFAULT_EXTRUDE_DEPTH, gen);
  }


  private async armExtrude(
    sketchId: string,
    regionId: string,
    editFeatureId?: string,
    startDepth = DEFAULT_EXTRUDE_DEPTH,
  ): Promise<void> {
    const gen = ++this.armGen;
    let finish: { regions: SketchRegion[] };
    try {
      finish = await this.deps.client.getSketchRegions(sketchId);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Cannot read sketch regions: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) return; // superseded while finishSketch was in flight
    const region = finish.regions.find((candidate) => candidate.regionId === regionId);
    const profile = region ? profileFromRegion(region) : null;
    if (!region || !profile) {
      const available = finish.regions.map((candidate) => candidate.regionId).join(", ") || "none";
      viewportStore
        .getState()
        .setStatusHint(`Selected region ${regionId} is stale or invalid; available: ${available}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    // Pure read (no backend sketch session opened) — a model-mode arm must not
    // leave a stray watermark that a later stray finish could squash a model op's
    // undo entry into (MODEL-HARDEN W0.5). getSketch returns the same DTO shape.
    const session = await this.deps.client.getSketch(sketchId); // plane only
    if (gen !== this.armGen) return; // superseded while getSketch was in flight
    await this.beginExtrudeArmed(sketchId, [region], [profile], session.plane, editFeatureId, startDepth);
  }

  /**
   * Arm Extrude for the exact selected region(s). Accepts one or more regions;
   * the preview/commit machinery already opens one lane session per region.
   */
  private async beginExtrudeArmed(
    sketchId: string,
    regions: SketchRegion[],
    profiles: PrismProfile[],
    plane: SketchPlane,
    editFeatureId?: string,
    startDepth = DEFAULT_EXTRUDE_DEPTH,
  ): Promise<void> {
    if (regions.length === 0 || profiles.length === 0) {
      viewportStore
        .getState()
        .setStatusHint("Extrude requires at least one closed sketch region", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const gen = this.armGen;
    this.plane = plane;
    this.lastArmedSketch = sketchId;
    this.armedSketchToken = documentStore.getState().sketches[sketchId]?.geometryToken ?? null;
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    // WP-C3: a RE-EDIT opens on the record's own draft angle (the commit writes
    // the armed value back, so a 0 seed would silently drop a stored draft). A
    // fresh arm has no record to read and opens at 0.
    const storedDraft = editFeatureId
      ? storedScalar(this.extrudeStoredParams?.draftAngleDeg)
      : 0;

    const centroid = combinedCentroidWorld(plane, profiles);
    this.centroidWorld = centroid;
    this.normal = normalize(this.plane.normal as Vec3);

    // The boolean seed is resolved AFTER the plane basis, because the probe half
    // of it raycasts along that normal from inside each profile. A re-edit is
    // excluded — its commit deep-merges the STORED params, so seeding a default
    // here would be a lie about what the ✓ re-targets.
    const seed = editFeatureId ? null : this.materialSeed(sketchId, true, profiles, startDepth);
    this.extrude = extrudeStep(extrudeInit(), {
      kind: "arm",
      depth: startDepth,
      draft: storedDraft,
      ...(seed ? { boolean: seed } : {}),
    }).state;

    this.engine.showExtrudePreviews(this.plane, profiles, this.centroidWorld, this.normal);
    this.engine.setExtrudeDepth(startDepth, false);
    // A fresh arm has not chosen a direction yet — the arrow opens two-way.
    this.extrudeGrabbed = false;
    this.syncExtrudeHandle();

    // Open one preview session per region. A superseded arm (gen bumped mid-await)
    // tears its own sessions down so no lane session leaks.
    const sessions: ToolPreviewSession[] = [];
    for (let i = 0; i < regions.length; i++) {
      const params = this.extrudePreviewParams(startDepth);
      if (editFeatureId) params.featureId = editFeatureId;
      const draft: PreviewDraft = {
        opType: "Extrude",
        sketchId,
        regionId: regions[i].regionId,
        regionIdentityVersion: regions[i].regionIdentityVersion,
        params,
      };
      let session: PreviewSession;
      try {
        session = await this.deps.client.beginPreview(draft);
      } catch (error) {
        for (const opened of sessions) {
          void this.deps.client.endPreview(opened.session.sessionId, false);
        }
        if (gen === this.armGen) {
          this.engine.hideExtrudePreview();
          viewportStore
            .getState()
            .setStatusHint(`Extrude preview failed: ${errMessage(error)}`, {
              severity: "error",
              sticky: true,
            });
        }
        return;
      }
      if (gen !== this.armGen) {
        void this.deps.client.endPreview(session.sessionId, false);
        for (const s of sessions) void this.deps.client.endPreview(s.session.sessionId, false);
        return;
      }
      sessions.push({
        session,
        draft,
        region: regions[i],
        profile: profiles[i],
        lastAppliedEpoch: 0,
        previewBodyIds: [],
        replacedBodyIds: [],
      });
    }
    this.previewSessions = sessions;
    // Claim the shared lane: from here `sendPreview()` reads the extrude params
    // through this closure and needs no extrude-specific argument.
    this.previewOwner = "extrude";
    this.previewParamsFn = () => this.extrudePreviewParams(this.extrude.depth);
    this.previewPending = false;

    this.throttle.reset();
    this.engine.setPreviewTint("normal");
    toolStore.setState({ phase: "armed" });
    const n = regions.length;
    viewportStore.getState().setStatusHint(
      editFeatureId
        ? "Approximate feature-edit preview; final geometry updates on Apply"
        : this.armHintFor("extrude", n),
      { sticky: true },
    );

    const { canBoolean } = this.resolveBooleanTarget();
    const chipWorld = this.chipWorld();
    toolChipStore.getState().showExtrude(
      startDepth,
      chipWorld,
      {
        onValue: (v) => this.onExtrudeChip(v),
        onSymmetric: (sym) => this.setExtrudeSymmetric(sym),
        onConfirm: () => void this.confirmExtrude(),
        onCancel: () => toolStore.getState().setTool("select"),
        onBooleanMode: (mode) => this.onExtrudeBooleanMode(mode),
        onEndCondition: (end) => void this.onExtrudeEndCondition(end),
        onDraftAngle: (deg) => this.onExtrudeDraftAngle(deg),
      },
      // Re-edit is param-only (depth) → value + ✓/✕ only, no symmetric toggle, no
      // boolean/end-condition segments (fresh arms only).
      {
        symmetric: false,
        showSymmetric: !editFeatureId,
        showBooleanSegments: !editFeatureId,
        showEndConditions: !editFeatureId,
        // ThroughAll/ToNext/ToFace all need something to reach — the same visible
        // bodies the boolean target resolution counts.
        canUseBodyEnds: canBoolean,
        endCondition: "Blind",
        canBoolean,
        // The RESOLVED mode, not a literal — a host-seeded arm opens on Add.
        booleanMode: this.extrude.booleanMode,
        regionCount: n,
        // Draft is a plain scalar parameter like the distance, so BOTH the fresh
        // arm and the param-only re-edit offer it (WP-C3). The seed is the armed
        // state, which a re-edit has already loaded from the record.
        showDraft: true,
        draftAngleDeg: this.extrude.draftAngleDeg,
        // Sit BESIDE the extrude axis, not on it: the chip's anchor is the
        // arrowhead, and the prism grows along exactly that line.
        anchorAxisFrom: this.centroidWorld,
        anchorOffsetPx: DEFAULT_CHIP_OFFSET_PX,
      },
    );

    this.sendPreview(); // initial exact L2 (all sessions)
    this.updateDebug();
  }

  private chipWorld(): Vec3 {
    return this.extrudeHeadWorld(this.extrude.depth);
  }

  /**
   * Where the arrow's TAIL sits for a given depth — the far face of the prism, so
   * the arrow extends past the geometry it is driving instead of being buried in
   * it. Symmetric grows both ways from the centroid (`PreviewMesh.setDepth`), so
   * the head marks the +|depth| face.
   */
  private extrudeHeadWorld(depth: number): Vec3 {
    const d = this.extrude.symmetric ? Math.abs(depth) : depth;
    return [
      this.centroidWorld[0] + this.normal[0] * d,
      this.centroidWorld[1] + this.normal[1] * d,
      this.centroidWorld[2] + this.normal[2] * d,
    ];
  }

  /**
   * Re-anchor the arrow onto the CURRENT depth. Called from every place the depth,
   * the symmetry or the resolved boolean mode can change — the arrow is the
   * primary readout of all three now that the chip carries only a dimension.
   *
   * A zero depth passes a zero direction on purpose: `DragHandle.setAxis` holds
   * its previous orientation there rather than snapping to an arbitrary flip.
   */
  private syncExtrudeHandle(): void {
    if (!this.plane) return;
    const depth = this.extrude.depth;
    const sign = this.extrude.symmetric ? 1 : Math.sign(depth);
    const dir: Vec3 = [this.normal[0] * sign, this.normal[1] * sign, this.normal[2] * sign];
    const head = this.extrudeHeadWorld(depth);
    this.engine.setExtrudeHandle(
      head,
      dir,
      this.extrudeGrabbed ? "forward" : "twoWay",
      this.extrude.booleanMode === "Cut",
    );
    // The chip rides the arrowhead. Straight to the engine, NEVER through
    // `toolChipStore.worldPos`: that field is the mount effect's key, so writing
    // it per frame would remount the chip and drop the focus out of its input.
    this.engine.moveChip(MODEL_TOOL_CHIP_ID, head, this.centroidWorld);
  }

  private onExtrudeChip(v: number): void {
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "dragging") return;
    this.extrude = extrudeStep(this.extrude, { kind: "setDepth", depth: v }).state;
    this.engine.setExtrudeDepth(v, this.extrude.symmetric);
    this.syncExtrudeHandle(); // a typed depth moves the arrow exactly like a drag
    toolChipStore.getState().setValue(v);
    this.sendPreview();
  }

  /** ⇔ toggle on the armed cluster: mirror Alt-during-drag onto the live preview. */
  private setExtrudeSymmetric(symmetric: boolean): void {
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "dragging") return;
    this.extrude = extrudeStep(this.extrude, { kind: "setSymmetric", symmetric }).state;
    this.engine.setExtrudeDepth(this.extrude.depth, symmetric);
    this.syncExtrudeHandle();
    toolChipStore.getState().setSymmetric(symmetric);
    this.sendPreview();
  }

  /**
   * Draft angle authored in the [Draft] segment (WP-C3). The reducer CLAMPS to
   * the legacy ±89° range, so the chip is refreshed from the resulting state
   * rather than from the typed value — the readout must never claim a value the
   * op does not carry. The preview re-send is what makes the drafted prism
   * appear: `extrudePreviewParams` already carried `draftAngleDeg`, and the
   * kernel PreviewOp applies it exactly as the commit does.
   */
  private onExtrudeDraftAngle(deg: number): void {
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "dragging") return;
    this.extrude = extrudeStep(this.extrude, { kind: "setDraftAngle", deg }).state;
    toolChipStore.getState().setDraftAngle(this.extrude.draftAngleDeg);
    this.sendPreview();
    this.updateDebug();
  }

  // ── boolean modes (extrude/revolve New Body / Add / Cut — Wave 2) ─────────────

  /**
   * Resolve the boolean target from the current VISIBLE bodies (documentStore,
   * `visible !== false`, preview ids excluded): 0 → no boolean; 1 → auto-target
   * that body; >1 → a target must be picked. Drives the chip's canBoolean + the
   * Add/Cut → armed vs targetPick decision.
   */
  private resolveBooleanTarget(): {
    canBoolean: boolean;
    count: number;
    autoTargetId: string | null;
    visibleIds: string[];
  } {
    const bodies = documentStore.getState().bodies;
    const visible = Object.values(bodies)
      .filter((b) => b.visible !== false && !b.id.startsWith("preview:"))
      .map((b) => b.id);
    return {
      canBoolean: visible.length > 0,
      count: visible.length,
      autoTargetId: visible.length === 1 ? visible[0] : null,
      visibleIds: visible,
    };
  }

  /**
   * The boolean default a FRESH arm off `sketchId` opens with, and the material
   * the DRAG DIRECTION will read.
   *
   * Two sources, one answer:
   *  - A sketch hosted on a model FACE belongs to that body (`SketchDto.hostFace`,
   *    read synchronously from the same store row the arm already reads its
   *    `geometryToken` from, so this adds no round-trip). It opens on Add against
   *    the host, and its host counts as material flush against the −normal side —
   *    which is exactly what makes `depth < 0` read as Cut.
   *  - Any other sketch gets a RAY PROBE of the real scene. This is the
   *    generalization: a sketch on a datum plane slicing through a body used to
   *    open on NewBody with nothing but a text hint.
   *
   * A host body that is gone or HIDDEN falls back to NewBody: an Add/Cut against
   * a body the user cannot see would be a silent surprise, which is the same
   * reason `resolveBooleanTarget` counts only visible bodies.
   */
  private materialSeed(
    sketchId: string,
    directionAware: boolean,
    profiles: PrismProfile[],
    depth: number,
  ): BooleanSeed | null {
    const doc = documentStore.getState();
    const host = doc.sketches[sketchId]?.hostFace;
    if (host) {
      const body = doc.bodies[host.bodyId];
      if (body && body.visible !== false) {
        // The host is flush against the profile on the side the face looks away
        // from — a synthetic probe, because the mesh of a coplanar face is not a
        // reliable thing to raycast against.
        const flush: SideProbe = { bodyId: host.bodyId, gap: 0, inside: true };
        return {
          mode: "Add",
          targetBodyId: host.bodyId,
          auto: directionAware,
          sides: { pos: null, neg: flush },
        };
      }
    }
    if (!directionAware) return null;
    const sides = this.probeMaterialSides(profiles);
    if (!hasMaterial(sides)) return null;
    // Opening mode: whatever the ARMED depth already resolves to, so the chip and
    // the first exact preview agree with the arrow before the user moves anything.
    const opened = autoModeFor(sides, depth, { mode: "NewBody", targetBodyId: null });
    if (!opened.targetBodyId) {
      return { mode: "NewBody", targetBodyId: "", auto: true, sides };
    }
    return { mode: opened.mode, targetBodyId: opened.targetBodyId, auto: true, sides };
  }

  /**
   * Ray-probe the committed bodies on both sides of the sketch plane.
   *
   * ONE RAY PER REGION, from a point KNOWN to be inside that region (the centroid
   * of its largest cap triangle) — the combined bbox centroid the arrow uses can
   * sit in empty space between two regions, or in an annulus' hole.
   *
   * Run at ARM time, synchronously, before the first `sendPreview()`: the exact
   * preview hides the very bodies being probed for
   * (`setPreviewReplacedBodyIds`), and three's raycaster skips invisible objects.
   */
  private probeMaterialSides(profiles: PrismProfile[]): MaterialSides {
    if (!this.plane || typeof this.engine.probeMaterial !== "function") return NO_MATERIAL;
    const n = this.normal;
    const back: Vec3 = [-n[0], -n[1], -n[2]];
    let pos: SideProbe | null = null;
    let neg: SideProbe | null = null;
    for (const origin of this.probeOrigins(profiles)) {
      pos = strongerProbe(pos, this.probeVisible(origin, n));
      neg = strongerProbe(neg, this.probeVisible(origin, back));
    }
    return { pos, neg };
  }

  /** One probe, dropped when it lands on a body the user cannot see. Visibility is
   *  re-checked against the DOCUMENT rather than inherited from the scene graph:
   *  a body hidden by isolation is not a legal boolean target either. */
  private probeVisible(origin: Vec3, dir: Vec3): SideProbe | null {
    const hit = this.engine.probeMaterial(origin, dir);
    if (!hit) return null;
    const body = documentStore.getState().bodies[hit.bodyId];
    if (!body || body.visible === false) return null;
    return hit;
  }

  /** Interior sample point per armed profile, in world space. */
  private probeOrigins(profiles: PrismProfile[]): Vec3[] {
    const plane = this.plane;
    if (!plane) return [];
    const origins = profiles
      .map((p) => profileSampleWorld(plane, p))
      .filter((o): o is Vec3 => o !== null);
    return origins.length > 0 ? origins : [this.centroidWorld];
  }

  /**
   * The still-valid body an Add/Cut is ALREADY bound to, if any. A host-seeded arm
   * carries one from the moment it opens, and switching Add↔Cut must keep it —
   * demanding a body pick for a target the tool already knows would be a worse
   * override than no override. Null when nothing is bound (the pre-HOST-BOOLEAN
   * state of every fresh arm) or the bound body is gone / hidden.
   */
  private boundBooleanTarget(current: string | null, visibleIds: string[]): string | null {
    return current && visibleIds.includes(current) ? current : null;
  }

  /** Boolean segment picked on the armed EXTRUDE cluster. */
  private onExtrudeBooleanMode(mode: BooleanMode): void {
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "targetPick") return;
    const { canBoolean, count, autoTargetId, visibleIds } = this.resolveBooleanTarget();
    if (mode !== "NewBody" && !canBoolean) return; // segment disabled — no existing body
    const bound = this.boundBooleanTarget(this.extrude.targetBodyId, visibleIds);
    if (mode === "NewBody") {
      this.extrude = extrudeStep(this.extrude, { kind: "setBooleanMode", mode }).state;
    } else if (bound) {
      this.extrude = extrudeStep(this.extrude, { kind: "setBooleanMode", mode, targetBodyId: bound }).state;
    } else if (count === 1) {
      this.extrude = extrudeStep(this.extrude, { kind: "setBooleanMode", mode, targetBodyId: autoTargetId }).state;
    } else {
      this.extrude = extrudeStep(this.extrude, { kind: "setBooleanMode", mode, needsPick: true }).state;
    }
    this.applyBooleanState(this.extrude.booleanMode, this.extrude.phase === "targetPick", "extrude");
    if (this.extrude.phase === "armed") this.sendPreview();
  }

  /**
   * End-condition segment picked on the armed EXTRUDE cluster (MODEL-OPS W1).
   * `ToFace` hands pointer ownership to a face pick; everything else applies
   * immediately. The chip already disables the body-reaching conditions when no
   * body exists, so this only re-checks as a guard.
   */
  private async onExtrudeEndCondition(end: ExtrudeEndCondition): Promise<void> {
    if (this.extrude.phase !== "armed" && this.extrude.phase !== "facePick") return;
    if (end !== "Blind" && !this.resolveBooleanTarget().canBoolean) return;
    this.extrude = extrudeStep(this.extrude, { kind: "setEndCondition", end }).state;
    toolChipStore.setState({ endCondition: end });
    if (this.extrude.phase === "facePick") {
      this.engine.setOrbitSuppressed(true);
      viewportStore
        .getState()
        .setStatusHint("Click the face to extrude up to · Esc cancels", { sticky: true });
    } else {
      this.engine.setOrbitSuppressed(false);
      viewportStore.getState().setStatusHint(this.armHintFor("extrude"), { sticky: true });
      this.sendPreview();
    }
    this.updateDebug();
  }

  /**
   * A face pick while the extrude is choosing its `ToFace` target. The topoKey is
   * PROMOTED to a Rust-minted ElementId first (the same
   * probe → promoteSelection → semanticRef chain Shell uses for its open faces):
   * a ToFace target must survive a parametric edit, and only a minted id carries
   * the identity the resolution ladder rebinds (SCHEMA §7.3).
   */
  private async tryPickExtrudeFace(clientX: number, clientY: number): Promise<void> {
    if (this.extrude.phase !== "facePick") return;
    const hit = this.engine.probePick(clientX, clientY);
    if (!hit || hit.kind !== "face" || hit.bodyId.startsWith("preview:")) return;
    const gen = this.armGen;
    const worldTriple: [number, number, number] = [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z];
    let elementId = hit.elementId;
    if (!elementId && hit.topoKey) {
      const promoted = await promoteOne(this.client, hit.bodyId, {
        topoKey: hit.topoKey,
        anchor: { worldPoint: worldTriple },
      });
      if (gen !== this.armGen) return; // re-armed while awaiting — drop
      // A stale pick must remain a pick. An anchor-only ToFace ref would let a
      // later resolver reinterpret the stale face against different topology.
      if (!promoted?.elementId) return;
      elementId = promoted.elementId;
    }
    if (this.extrude.phase !== "facePick") return;
    const ref: SemanticRef = {
      primary: { bodyId: hit.bodyId, elementId, kind: "face" },
      anchor: { worldPoint: worldTriple },
    };
    this.extrude = extrudeStep(this.extrude, { kind: "pickFace", ref }).state;
    this.clearToolHover();
    this.engine.setOrbitSuppressed(false);
    viewportStore.getState().setStatusHint(this.armHintFor("extrude"), { sticky: true });
    this.sendPreview();
    this.updateDebug();
  }

  /** Esc during facePick: fall back to Blind rather than arming an unreachable ToFace. */
  private cancelFacePick(): void {
    if (this.extrude.phase !== "facePick") return;
    this.extrude = extrudeStep(this.extrude, { kind: "cancelFacePick" }).state;
    toolChipStore.setState({ endCondition: this.extrude.endCondition });
    this.clearToolHover();
    this.engine.setOrbitSuppressed(false);
    viewportStore.getState().setStatusHint(this.armHintFor("extrude"), { sticky: true });
    this.sendPreview();
    this.updateDebug();
  }

  /** Boolean segment picked on the armed REVOLVE cluster (mirrors the extrude path). */
  private onRevolveBooleanMode(mode: BooleanMode): void {
    if (this.revolve.phase !== "armed" && this.revolve.phase !== "targetPick") return;
    const { canBoolean, count, autoTargetId, visibleIds } = this.resolveBooleanTarget();
    if (mode !== "NewBody" && !canBoolean) return;
    const bound = this.boundBooleanTarget(this.revolve.targetBodyId, visibleIds);
    if (mode === "NewBody") {
      this.revolve = revolveStep(this.revolve, { kind: "setBooleanMode", mode }).state;
    } else if (bound) {
      this.revolve = revolveStep(this.revolve, { kind: "setBooleanMode", mode, targetBodyId: bound }).state;
    } else if (count === 1) {
      this.revolve = revolveStep(this.revolve, { kind: "setBooleanMode", mode, targetBodyId: autoTargetId }).state;
    } else {
      this.revolve = revolveStep(this.revolve, { kind: "setBooleanMode", mode, needsPick: true }).state;
    }
    this.applyBooleanState(this.revolve.booleanMode, this.revolve.phase === "targetPick", "revolve");
    // A mode change rewrites the candidate op (Cut/Add bind a target body), so the
    // exact preview must follow it — this is the frame where a Cut starts SUBTRACTING.
    if (this.revolve.phase === "armed") this.sendPreview();
  }

  /** Shared side effects of a boolean-mode change: chip + Cut tint + hint. */
  private applyBooleanState(mode: BooleanMode, targetPick: boolean, tool: "extrude" | "revolve"): void {
    toolChipStore.getState().setBooleanMode(mode);
    this.engine.setPreviewTint(mode === "Cut" ? "cut" : "normal");
    if (targetPick) {
      // Modal: the next click picks a body, so an LMB drag must not spin the
      // camera out from under it. Cleared at every EXIT (see the pick + cancel
      // paths below) rather than here — the `false` arm of this branch also runs
      // for an armed REVOLVE, whose own drag suppression must survive it.
      this.engine.setOrbitSuppressed(true);
      const verb = mode === "Cut" ? "Cut from" : "Add to";
      viewportStore.getState().setStatusHint(`Click a body to ${verb} · Esc for New Body`, { sticky: true });
    } else {
      viewportStore.getState().setStatusHint(this.armHintFor(tool), { sticky: true });
    }
    this.updateDebug();
  }

  /**
   * The standard "drag/confirm" arm hint for a tool (restored after a target pick).
   *
   * When the probe found material, the hint SAYS the direction rule out loud —
   * the boolean segments are behind the chip's `⋯` now, so this and the arrow's
   * own colour are what teach it. `regions` overrides the session count for the
   * arm itself, which sets the hint before the sessions exist.
   */
  private armHintFor(tool: "extrude" | "revolve", regions?: number): string {
    if (tool === "extrude") {
      const n = regions ?? this.previewSessions.length;
      const head =
        n > 1 ? `Drag the arrow to set depth for ${n} regions` : "Drag the arrow to set depth";
      const rule =
        this.extrude.booleanAuto && hasMaterial(this.extrude.sides)
          ? " · one way adds material, the other cuts"
          : "";
      return `${head}${rule} · Enter, ✓, or click away to confirm`;
    }
    return "Drag to set angle · Enter, ✓, or click away to revolve";
  }

  /** A body pick during targetPick (extrude): probe → adopt as the boolean target. */
  private tryPickExtrudeTarget(clientX: number, clientY: number): void {
    if (this.extrude.phase !== "targetPick") return;
    const hit = this.engine.probePick(clientX, clientY);
    if (!hit || hit.bodyId.startsWith("preview:")) return; // reject preview bodies
    this.extrude = extrudeStep(this.extrude, { kind: "pickTarget", bodyId: hit.bodyId }).state;
    this.clearToolHover();
    // Back to armed: the extrude handle is grabbed by hitting it, so orbit is free.
    this.engine.setOrbitSuppressed(false);
    this.applyBooleanState(this.extrude.booleanMode, false, "extrude");
    this.sendPreview();
  }

  /** A body pick during targetPick (revolve). */
  private tryPickRevolveTarget(clientX: number, clientY: number): void {
    if (this.revolve.phase !== "targetPick") return;
    const hit = this.engine.probePick(clientX, clientY);
    if (!hit || hit.bodyId.startsWith("preview:")) return;
    this.revolve = revolveStep(this.revolve, { kind: "pickTarget", bodyId: hit.bodyId }).state;
    this.clearToolHover();
    // Orbit STAYS suppressed: an armed revolve's drag is the angle (see `tryPickRevolveAxis`).
    this.applyBooleanState(this.revolve.booleanMode, false, "revolve");
    this.sendPreview();
  }

  /** Esc during targetPick: revert to armed(NewBody), clear the Cut tint. */
  private cancelTargetPick(): void {
    if (this.extrude.phase === "targetPick") {
      this.extrude = extrudeStep(this.extrude, { kind: "cancelTargetPick" }).state;
      this.clearToolHover();
      this.engine.setOrbitSuppressed(false); // back to armed — orbit is free again
      this.applyBooleanState("NewBody", false, "extrude");
      this.sendPreview();
    } else if (this.revolve.phase === "targetPick") {
      this.revolve = revolveStep(this.revolve, { kind: "cancelTargetPick" }).state;
      this.clearToolHover();
      // Orbit STAYS suppressed: an armed revolve's drag is the angle.
      this.applyBooleanState("NewBody", false, "revolve");
      this.sendPreview();
    }
  }

  // ── multi-region pick (Revolve only) ─────────────────────────────────────────
  //
  // Extrude consumes one persistent sketchRegion selection directly. Revolve still
  // offers an in-viewport picker because it starts from a whole-sketch selection.

  /** Bump the arm generation so any in-flight finishSketch/enterSketch result is dropped. */
  private invalidateArm(): void {
    this.armGen++;
  }

  private async enterRegionPick(
    kind: "extrude" | "revolve",
    sketchId: string,
    regions: SketchRegion[],
    editFeatureId: string | undefined,
    startValue: number,
    gen: number,
  ): Promise<void> {
    // Pure read (no session opened) — see armExtrude (MODEL-HARDEN W0.5).
    const session = await this.deps.client.getSketch(sketchId); // plane (+ entities for revolve)
    if (gen !== this.armGen) return; // superseded while getSketch was in flight
    const noun = kind;
    // Only regions with an extrudable profile are pickable (others can't be built).
    const pickable = regions.filter((r) => profileFromRegion(r) !== null);
    if (pickable.length === 0) {
      this.resetToSelect(`No closed region to ${noun}`, { severity: "error", sticky: true });
      return;
    }
    if (pickable.length === 1) {
      // Only one region is actually extrudable — arm it directly (no pointless pick).
      this.armPickedRegions(kind, sketchId, [pickable[0]], session, editFeatureId, startValue);
      return;
    }
    // Wave 2: >1 region → MULTI-select. Toggle membership; Enter / chip ✓ / a
    // double-click on one region confirms; Esc cancels.
    const chipWorld = regionsCentroidWorld(session.plane, pickable);
    this.regionPick = {
      kind,
      sketchId,
      plane: session.plane,
      regions: pickable,
      session,
      editFeatureId,
      startValue,
      select: regionSelectStep(regionSelectInit(), { kind: "enter" }).state,
      chipWorld,
    };
    this.lastRegionClickId = null;
    this.engine.showRegionPick(session.plane, pickable);
    this.engine.setRegionSelected([]);
    this.engine.setOrbitSuppressed(true); // modal: click picks a region, not orbit
    viewportStore.getState().setStatusHint(
      `Select regions to ${noun} — click to toggle · Enter to confirm`,
      { sticky: true },
    );
    toolChipStore.getState().showRegionSelect(0, chipWorld, {
      onConfirm: () => this.confirmRegionSelect(),
      onCancel: () => toolStore.getState().setTool("select"),
    });
    this.updateDebug();
  }

  /** Arm the picking tool on the chosen region(s), threading exact ids into each payload. */
  private armPickedRegions(
    kind: "extrude" | "revolve",
    sketchId: string,
    regions: SketchRegion[],
    session: SketchSession,
    editFeatureId: string | undefined,
    startValue: number,
  ): void {
    const profiles = regions.map((r) => profileFromRegion(r));
    const valid = regions.filter((_, i) => profiles[i] !== null);
    const validProfiles = profiles.filter((p): p is PrismProfile => p !== null);
    if (valid.length === 0) {
      this.resetToSelect(`No closed region to ${kind}`, { severity: "error", sticky: true });
      return;
    }
    if (kind === "extrude") {
      void this.beginExtrudeArmed(
        sketchId,
        valid,
        validProfiles,
        session.plane,
        editFeatureId,
        startValue,
      );
      return;
    }
    this.beginRevolveArmed(sketchId, valid, validProfiles, session, editFeatureId, startValue);
  }

  /** Pointer hover during region pick: tint the region under the pointer. */
  private updateRegionHover(clientX: number, clientY: number): void {
    const ctx = this.regionPick;
    if (!ctx) return;
    const p = this.deps.engine.screenToPlaneOn(ctx.plane, clientX, clientY);
    const id = p ? regionAtPoint(ctx.regions, p.x, p.y) : null;
    this.deps.engine.setRegionHover(id);
  }

  /** Pointer click during region pick: toggle the region under the pointer (multi-select). */
  private tryPickRegion(clientX: number, clientY: number): void {
    const ctx = this.regionPick;
    if (!ctx) return;
    const p = this.deps.engine.screenToPlaneOn(ctx.plane, clientX, clientY);
    if (!p) return;
    const id = regionAtPoint(ctx.regions, p.x, p.y);
    if (!id) return;
    // Double-click accelerator: a second click on the SAME region within the window
    // = select only it + confirm immediately.
    const now = performance.now();
    if (this.lastRegionClickId === id && now - this.lastRegionClickAt < 350) {
      this.lastRegionClickId = null;
      this.resolveRegionPick([id]);
      return;
    }
    this.lastRegionClickId = id;
    this.lastRegionClickAt = now;
    ctx.select = regionSelectStep(ctx.select, { kind: "toggle", id }).state;
    this.engine.setRegionSelected(ctx.select.selected);
    toolChipStore.getState().setCount(ctx.select.selected.length);
    this.updateDebug();
  }

  /** Enter / region chip ✓: confirm the current multi-selection (≥1 region). */
  private confirmRegionSelect(): void {
    const ctx = this.regionPick;
    if (!ctx) return;
    const step = regionSelectStep(ctx.select, { kind: "confirm" });
    if (step.effect !== "confirm") return; // empty selection — ignore
    this.resolveRegionPick(step.state.selected);
  }

  private resolveRegionPick(regionIds: string[]): void {
    const ctx = this.regionPick;
    if (!ctx) return;
    const regions = regionIds
      .map((id) => ctx.regions.find((r) => r.regionId === id))
      .filter((r): r is SketchRegion => r !== undefined);
    if (regions.length === 0) return;
    this.regionPick = null;
    this.deps.engine.hideRegionPick();
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    this.armPickedRegions(ctx.kind, ctx.sketchId, regions, ctx.session, ctx.editFeatureId, ctx.startValue);
  }

  /** Tear down an in-flight region pick (Esc / tool switch); restores orbit. */
  private cancelRegionPick(): void {
    if (!this.regionPick) return;
    this.regionPick = null;
    this.lastRegionClickId = null;
    this.deps.engine.hideRegionPick();
    this.deps.engine.setRegionSelected([]);
    this.deps.engine.setOrbitSuppressed(false);
  }

  // ── revolve ────────────────────────────────────────────────────────────────

  private armRevolveFromSelection(): void {
    // REVOLVE-REGION-PARITY (WP-C3): a typed `sketchRegion` selection binds those
    // EXACT regions, exactly as `armExtrudeFromSelection` does. The old ladder read
    // only a whole-SKETCH selection and re-derived a profile from it, so pressing
    // Revolve with a region already picked could revolve a DIFFERENT region than
    // the one on screen. Unlike extrude (single-profile by design), revolve keeps
    // its N-region commit loop, so every selected region is bound.
    const selected = selectionStore.getState().selected;
    const verdict = getToolApplicability("revolve", selected, this.applicabilityCtx());
    const picked = selected.filter((ref) => ref.kind === "sketchRegion");
    if (picked.length > 0 && verdict.enabled) {
      void this.armRevolveRegions(picked[0].sketchId, picked.map((ref) => ref.regionId));
      return;
    }
    // Nothing region-typed selected: the same tool-first ladder as extrude —
    // explicit sketch selection wins, else the document's sole visible sketch.
    if (picked.length === 0) {
      const sketchId = this.pickTargetSketchId();
      if (sketchId) {
        void this.armRevolve(sketchId);
        return;
      }
    }
    viewportStore.getState().setStatusHint(verdict.reason ?? null, { severity: verdict.severity, sticky: true });
  }

  /**
   * Arm revolve on the EXACT regions the selection names (WP-C3 region parity).
   *
   * Region acquisition is the PURE `getSketchRegions` read extrude uses — never
   * `finishSketch` — so this works on a REOPENED sketch that has no live backend
   * session (both `getSketchRegions` and `getSketch` resolve a never-entered
   * sketch by its store key, which IS the backend UUID after projection
   * hydration), and it authors no timeline record at arm time (MODEL-HARDEN
   * W0.5; `confirmRevolve` guarantees the record at the commit boundary).
   *
   * A `regionId` that no longer resolves fails LOUDLY with the available ids —
   * house rule: a stale id must never silently bind a different profile.
   */
  private async armRevolveRegions(sketchId: string, regionIds: string[]): Promise<void> {
    const gen = ++this.armGen;
    let read: { regions: SketchRegion[] };
    try {
      read = await this.deps.client.getSketchRegions(sketchId);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Cannot read sketch regions: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) return; // superseded while getSketchRegions was in flight
    const bound: SketchRegion[] = [];
    const profiles: PrismProfile[] = [];
    for (const id of regionIds) {
      const region = read.regions.find((candidate) => candidate.regionId === id);
      const profile = region ? profileFromRegion(region) : null;
      if (!region || !profile) {
        const available = read.regions.map((candidate) => candidate.regionId).join(", ") || "none";
        this.resetToSelect(`Revolve region ${id} is stale or invalid; available: ${available}`, {
          severity: "error",
          sticky: true,
        });
        return;
      }
      bound.push(region);
      profiles.push(profile);
    }
    // Pure read (no session opened) — the axis candidates come from
    // `session.entities`, which `getSketch` returns verbatim.
    const session = await this.deps.client.getSketch(sketchId);
    if (gen !== this.armGen) return; // superseded while getSketch was in flight
    this.beginRevolveArmed(sketchId, bound, profiles, session);
  }

  /**
   * Arm the revolve tool on a sketch. A single region goes straight to axis-pick; >1
   * region (fresh arm) first runs the region pick, THEN axis-pick on the chosen one.
   *
   * REVOLVE-REGION-PARITY: the region read is the PURE `getSketchRegions`, never
   * `finishSketch`. `finishSketch` opens/squashes a session AND mints the sketch's
   * timeline record as a side effect — arming must do neither (MODEL-HARDEN W0.5);
   * the record is guaranteed at the COMMIT boundary instead (`confirmRevolve`).
   *
   * `regionId` (re-edit) binds that EXACT stored region — a miss fails loudly with
   * the available ids and never silently falls back to `regions[0]`. An EMPTY stored
   * id is the wire's legitimate V1 first-region fallback, so it takes the same path
   * as a fresh single-region arm.
   */
  private async armRevolve(
    sketchId: string,
    editFeatureId?: string,
    startAngle = DEFAULT_REVOLVE_ANGLE,
    regionId?: string,
    storedParams?: Record<string, unknown>,
  ): Promise<void> {
    const gen = ++this.armGen;
    let read: { regions: SketchRegion[] };
    try {
      read = await this.deps.client.getSketchRegions(sketchId);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Cannot read sketch regions: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) return; // superseded while getSketchRegions was in flight

    if (regionId) {
      const bound = read.regions.find((candidate) => candidate.regionId === regionId);
      const boundProfile = bound ? profileFromRegion(bound) : null;
      if (!bound || !boundProfile) {
        const available = read.regions.map((candidate) => candidate.regionId).join(", ") || "none";
        this.resetToSelect(
          `Revolve region ${regionId} is stale or invalid; available: ${available}`,
          { severity: "error", sticky: true },
        );
        return;
      }
      // Pure read (no session opened) — see armExtrude (MODEL-HARDEN W0.5). The revolve
      // axis candidates read `session.entities`, which getSketch returns verbatim.
      const editSession = await this.deps.client.getSketch(sketchId); // plane + entities
      if (gen !== this.armGen) return; // superseded while getSketch was in flight
      this.beginRevolveArmed(
        sketchId,
        [bound],
        [boundProfile],
        editSession,
        editFeatureId,
        startAngle,
        storedParams,
      );
      return;
    }

    if (read.regions.length > 1 && !editFeatureId) {
      await this.enterRegionPick("revolve", sketchId, read.regions, editFeatureId, startAngle, gen);
      return;
    }
    const region = read.regions[0];
    const profile = region ? profileFromRegion(region) : null;
    if (!region || !profile) {
      this.resetToSelect("No closed region to revolve", { severity: "error", sticky: true });
      return;
    }
    const session = await this.deps.client.getSketch(sketchId); // plane + entities
    if (gen !== this.armGen) return; // superseded while getSketch was in flight
    this.beginRevolveArmed(sketchId, [region], [profile], session, editFeatureId, startAngle, storedParams);
  }

  /**
   * Arm the revolve tool on N chosen regions + session: resolve candidate axis lines,
   * then enter axis-pick (fresh) or go straight to armed (re-edit, `editFeatureId`).
   * Shared by the single-region + region multi-select paths. The PRIMARY region
   * (regions[0]) drives the L1 lathe preview; the arrays carry all regions for the
   * commit loop + all-regions axis validity (Wave 2).
   */
  private beginRevolveArmed(
    sketchId: string,
    regions: SketchRegion[],
    profiles: PrismProfile[],
    session: SketchSession,
    editFeatureId?: string,
    startAngle = DEFAULT_REVOLVE_ANGLE,
    storedParams?: Record<string, unknown>,
  ): void {
    const region = regions[0];
    const profile = profiles[0];
    const candidates: AxisCandidate[] = session.entities
      .filter((e) => e.type === "Line" && e.p0 && e.p1)
      .map((e) => ({ id: e.id, a: e.p0 as [number, number], b: e.p1 as [number, number] }));

    // Re-edit: seed the axis from the STORED lineId. A stored id that no longer
    // resolves is a REFUSAL, never a substitution — silently re-binding a different
    // line would make the preview lie about what the commit re-targets. Resolved
    // BEFORE any controller state is written so a refusal leaves nothing armed.
    const storedAxisLineId = editFeatureId ? axisLineIdFromParams(storedParams) : null;
    let seedAxis: AxisCandidate | null = null;
    if (editFeatureId) {
      if (storedAxisLineId) {
        seedAxis = candidates.find((c) => c.id === storedAxisLineId) ?? null;
        if (!seedAxis) {
          this.resetToSelect(
            `Revolve axis line ${storedAxisLineId} no longer exists in the sketch`,
            { severity: "error", sticky: true },
          );
          return;
        }
      } else {
        // No stored axis (legacy record): the first candidate only renders the L1
        // shell; the commit's deep-merge keeps whatever the record already holds.
        seedAxis = candidates[0] ?? null;
      }
    }

    this.plane = session.plane;
    this.lastArmedSketch = sketchId;
    this.armedSketchToken = documentStore.getState().sketches[sketchId]?.geometryToken ?? null;
    this.revolveProfile = profile;
    this.revolveProfiles = profiles;
    this.revolveRegionIds = regions.map((r) => r.regionId);
    this.revolveRegionIdentityVersions = regions.map((r) => r.regionIdentityVersion);
    this.revolveSketchId = sketchId;
    this.revolveRegionId = region.regionId;
    this.revolveEditFeatureId = editFeatureId;
    // Re-edit: the CALLER owns the stored-params fetch (it also owns the profile
    // binding read from them), so the angle-only commit deep-merges instead of
    // clobbering the user-picked axis (the projection does not expose it).
    this.revolveStoredParams = editFeatureId ? storedParams : undefined;
    this.revolveAxisCandidates = candidates;

    const b = profileBounds(profile);
    const c = planePointToWorld(this.plane, { x: b.centroidU, y: b.centroidV });
    this.centroidWorld = [c.x, c.y, c.z];
    this.normal = normalize(this.plane.normal as Vec3);
    this.revolveArmedDown = false;

    if (editFeatureId) {
      // Re-edit is param-only (angle) — skip axis-pick and render the L1 shell around
      // the STORED axis (resolved above); only a record with no stored axis at all
      // falls back to the first candidate. The commit re-targets by id.
      const cand = seedAxis;
      this.revolveAxis = cand ? { a: cand.a, b: cand.b } : fallbackAxis(profile.ring);
      this.revolveAxisLineId = cand?.id ?? null;
      this.revolve = revolveStep(revolveInit(), {
        kind: "arm",
        angle: startAngle,
        hasAxis: true,
        axisLineId: this.revolveAxisLineId,
      }).state;
      this.deps.engine.setOrbitSuppressed(true);
      this.deps.engine.showRevolvePreview(this.plane, profile.ring, this.revolveAxis, startAngle);
      toolStore.setState({ phase: "armed" });
      viewportStore.getState().setStatusHint("Drag or type an angle · Enter to apply", { sticky: true });
      toolChipStore.getState().showRevolve(
        startAngle,
        this.revolveChipWorld(),
        {
          onValue: (v) => this.onRevolveChip(v),
          onResetAxis: () => this.resetRevolveAxis(),
          onConfirm: () => void this.confirmRevolve(),
          onCancel: () => toolStore.getState().setTool("select"),
        },
        { anchorAxisFrom: this.revolveChipAxisFrom() },
      );
    } else {
      this.revolveAxis = null;
      this.revolveAxisLineId = null;
      // HOST-BOOLEAN, revolve half: a face-hosted sketch defaults to Add on its host.
      // NO direction logic — a revolve sweeps around an axis rather than pushing into
      // or away from the host — so the seeded mode holds until the chip changes it.
      const hostSeed = this.materialSeed(sketchId, false, [], 0);
      this.revolve = revolveStep(revolveInit(), {
        kind: "arm",
        angle: startAngle,
        ...(hostSeed ? { boolean: hostSeed } : {}),
      }).state; // → axisPick
      // Modal, like the region pick: a click takes the axis line, so an LMB drag
      // must not spin the camera instead. (MMB/RMB pan and the wheel are
      // untouched — `setOrbitSuppressed` gates the LMB orbit only.)
      this.deps.engine.setOrbitSuppressed(true);
      this.deps.engine.showRevolveAxisCandidates(
        this.plane,
        this.revolveAxisCandidates.map((k) => ({ a: k.a, b: k.b })),
      );
      toolStore.setState({ phase: "armed" });
      viewportStore.getState().setStatusHint(
        this.revolveAxisCandidates.length
          ? "Pick axis line"
          : "Draw a sketch line to use as the revolve axis",
        { sticky: true },
      );
      // In-viewport guidance for the axisPick gap the StatusBar alone was easy to
      // miss on (UNIFY-UX Phase 2) — no world axis exists yet, so it hangs off the
      // profile centroid rather than a leader-lined point.
      toolChipStore.getState().showRevolveAxisPick(this.centroidWorld, {
        onCancel: () => toolStore.getState().setTool("select"),
      });
    }
    this.updateDebug();
  }

  private revolveChipWorld(): Vec3 {
    return this.chipWorld();
  }

  /** The armed revolve chip's leader-line anchor: one endpoint of the picked
   *  axis line, projected to world — `undefined` once no axis/plane is known
   *  (a re-edit whose stored axis failed to resolve never reaches this). */
  private revolveChipAxisFrom(): Vec3 | undefined {
    if (!this.plane || !this.revolveAxis) return undefined;
    const w = planePointToWorld(this.plane, { x: this.revolveAxis.a[0], y: this.revolveAxis.a[1] });
    return [w.x, w.y, w.z];
  }

  private onRevolveChip(v: number): void {
    if (this.revolve.phase !== "armed" && this.revolve.phase !== "dragging") return;
    const angle = clampAngle(v);
    this.revolve = revolveStep(this.revolve, { kind: "setAngle", angle }).state;
    if (this.revolveAxis && this.revolveProfile) {
      this.deps.engine.setRevolveAngle(this.revolveProfile.ring, this.revolveAxis, angle);
    }
    toolChipStore.getState().setValue(angle);
    this.sendPreview();
  }

  /** Chip "Axis" affordance: drop the chosen axis and return to axis-pick. */
  private resetRevolveAxis(): void {
    if (this.revolve.phase !== "armed" && this.revolve.phase !== "dragging") return;
    this.revolve = revolveStep(this.revolve, { kind: "resetAxis" }).state;
    // No axis ⇒ no well-formed candidate: release the lane rather than leave sessions
    // open on a draft the builder would refuse (they re-open on the next axis pick).
    this.closePreviewSessions();
    this.revolveAxis = null;
    this.revolveAxisLineId = null;
    this.revolveArmedDown = false;
    if (this.dragging === "revolve") this.dragging = null;
    // STILL suppressed: this drops back into axis-pick, which is modal in exactly
    // the same way the first one was — not to a free-orbit armed state.
    this.deps.engine.setOrbitSuppressed(true);
    this.deps.engine.hideRevolvePreview();
    this.engine.setPreviewTint("normal");
    if (this.plane) {
      this.deps.engine.showRevolveAxisCandidates(
        this.plane,
        this.revolveAxisCandidates.map((k) => ({ a: k.a, b: k.b })),
      );
    }
    viewportStore.getState().setStatusHint("Pick axis line", { sticky: true });
    toolChipStore.getState().showRevolveAxisPick(this.centroidWorld, {
      onCancel: () => toolStore.getState().setTool("select"),
    });
    toolStore.setState({ phase: "armed" });
    this.updateDebug();
  }

  /** Axis-pick hover: highlight the nearest candidate line under the pointer. */
  private updateRevolveAxisHover(clientX: number, clientY: number): void {
    if (!this.plane) return;
    const p = this.deps.engine.screenToPlaneOn(this.plane, clientX, clientY);
    const idx = p ? this.nearestAxisCandidate(p.x, p.y) : -1;
    if (idx < 0) {
      this.deps.engine.setRevolveAxisHover(null);
      return;
    }
    const c = this.revolveAxisCandidates[idx];
    this.deps.engine.setRevolveAxisHover({ a: c.a, b: c.b });
  }

  /**
   * Axis-pick click: choose the nearest line, rejecting one that crosses ANY of the
   * selected regions' profiles (Wave 2 — the single axis must be valid for ALL).
   */
  private tryPickRevolveAxis(clientX: number, clientY: number): void {
    if (!this.plane || !this.revolveProfile) return;
    const p = this.deps.engine.screenToPlaneOn(this.plane, clientX, clientY);
    if (!p) return;
    const idx = this.nearestAxisCandidate(p.x, p.y);
    if (idx < 0) return;
    const cand = this.revolveAxisCandidates[idx];
    const profiles = this.revolveProfiles.length ? this.revolveProfiles : [this.revolveProfile];
    const failing = profiles.filter((pr) => axisSplitsRegion(cand.a, cand.b, pr.ring)).length;
    const valid = failing === 0;
    this.revolve = revolveStep(this.revolve, { kind: "pickAxis", lineId: cand.id, valid }).state;
    if (!valid) {
      this.deps.engine.setRevolveAxisHover(null);
      const msg =
        profiles.length > 1
          ? `Axis can't cross the profile — ${failing} of ${profiles.length} regions fail · pick another line`
          : "Axis can't cross the profile — pick another line";
      viewportStore.getState().setStatusHint(msg, { severity: "error", sticky: true });
      return;
    }
    this.revolveAxis = { a: cand.a, b: cand.b };
    this.revolveAxisLineId = cand.id;
    this.deps.engine.setOrbitSuppressed(true);
    this.deps.engine.showRevolvePreview(this.plane, this.revolveProfile.ring, this.revolveAxis, this.revolve.angle);
    this.engine.setPreviewTint("normal");
    const { canBoolean } = this.resolveBooleanTarget();
    toolChipStore.getState().showRevolve(
      this.revolve.angle,
      this.revolveChipWorld(),
      {
        onValue: (v) => this.onRevolveChip(v),
        onResetAxis: () => this.resetRevolveAxis(),
        onConfirm: () => void this.confirmRevolve(),
        onCancel: () => toolStore.getState().setTool("select"),
        onBooleanMode: (mode) => this.onRevolveBooleanMode(mode),
      },
      {
        // The RESOLVED mode, not a literal — a host-seeded arm opens on Add.
        showBooleanSegments: true,
        canBoolean,
        booleanMode: this.revolve.booleanMode,
        anchorAxisFrom: this.revolveChipAxisFrom(),
      },
    );
    viewportStore.getState().setStatusHint(this.armHintFor("revolve"), { sticky: true });
    toolStore.setState({ phase: "armed" });
    // The op is well-formed the moment an axis exists — open the kernel-preview
    // sessions now. Fire-and-forget: the arm itself is already complete + usable
    // (L1 lathe on screen), and the async half is armGen-guarded.
    void this.openRevolvePreviewSessions();
    this.updateDebug();
  }

  /**
   * The revolve's CURRENT complete params snapshot — the lane's `previewParamsFn`.
   *
   * Mirrors `confirmRevolve`'s op params FIELD-FOR-FIELD (which is also what the
   * `previewOps.ts` Revolve builder reconstructs), so the candidate the kernel
   * previews and the op the commit materializes are the same op. `axis` is
   * `undefined` until a line is picked — a state in which no session is open at
   * all, since axis-choice is exactly what makes the op well-formed.
   */
  private revolvePreviewParams(): PreviewParams {
    const sketchId = this.revolveSketchId;
    const axis: AxisRef | undefined =
      sketchId && this.revolveAxisLineId
        ? { kind: "sketchLine", sketchId, lineId: this.revolveAxisLineId }
        : undefined;
    const params: PreviewParams = {
      angleDeg: this.revolve.angle,
      axis,
      booleanMode: this.revolve.booleanMode as FeatureBooleanMode,
    };
    // `setBooleanMode NewBody` clears the target in the FSM, so this is set exactly
    // when the commit would set it (confirmRevolve reads the same two fields).
    const targetBodyId = this.revolve.targetBodyId ?? undefined;
    if (targetBodyId) params.targetBodyId = targetBodyId;
    return params;
  }

  /**
   * Open one kernel-preview session per armed region and claim the shared lane —
   * the Revolve half of `beginExtrudeArmed`'s session loop, run at the moment the
   * op becomes WELL-FORMED (an axis was chosen). Until then a Revolve draft has no
   * axis and the builder would refuse it.
   *
   * The L1 lathe stays underneath (extrude parity: L1 owns the drag handle + the
   * instant feedback); the exact L2 mesh swaps in beneath it, which is what makes a
   * Cut revolve finally SHOW the subtraction instead of a solid shell.
   *
   * A RE-EDIT opens none: `PreviewOp` runs against the current HEAD, so previewing
   * a feature's edit would double-apply that feature — the lane rejects an
   * `editFeatureId` draft structurally and the re-edit stays chip-driven L1.
   *
   * `armGen` guards every await exactly as the extrude loop does: a superseded arm
   * tears its OWN sessions down so no lane session leaks.
   */
  private async openRevolvePreviewSessions(): Promise<void> {
    if (this.revolveEditFeatureId) return; // re-edit: L1 only (see above)
    const sketchId = this.revolveSketchId;
    const regionIds = this.revolveRegionIds.length
      ? this.revolveRegionIds
      : this.revolveRegionId
        ? [this.revolveRegionId]
        : [];
    if (!sketchId || regionIds.length === 0) return;
    const gen = this.armGen;
    const sessions: ToolPreviewSession[] = [];
    for (let i = 0; i < regionIds.length; i++) {
      const draft: PreviewDraft = {
        opType: "Revolve",
        sketchId,
        regionId: regionIds[i],
        regionIdentityVersion: this.revolveRegionIdentityVersions[i],
        params: this.revolvePreviewParams(),
      };
      let session: PreviewSession;
      try {
        session = await this.deps.client.beginPreview(draft);
      } catch (error) {
        for (const opened of sessions) {
          void this.deps.client.endPreview(opened.session.sessionId, false);
        }
        if (gen === this.armGen) {
          // The L1 lathe stays: a preview that could not open is not a reason to
          // drop the user's armed revolve, only to say so.
          viewportStore
            .getState()
            .setStatusHint(`Revolve preview failed: ${errMessage(error)}`, {
              severity: "error",
              sticky: true,
            });
        }
        return;
      }
      if (gen !== this.armGen) {
        void this.deps.client.endPreview(session.sessionId, false);
        for (const s of sessions) void this.deps.client.endPreview(s.session.sessionId, false);
        return;
      }
      sessions.push({
        session,
        draft,
        profile: this.revolveProfiles[i],
        lastAppliedEpoch: 0,
        previewBodyIds: [],
        replacedBodyIds: [],
      });
    }
    this.previewSessions = sessions;
    // Claim the shared lane: from here `sendPreview()` reads the revolve params
    // through this closure and needs no revolve-specific argument.
    this.previewOwner = "revolve";
    this.previewParamsFn = () => this.revolvePreviewParams();
    this.previewPending = false;
    this.throttle.reset();
    this.throttle.setTrailingMs(REVOLVE_PREVIEW_TRAILING_MS);
    this.sendPreview(); // initial exact L2 (all sessions)
    this.updateDebug();
  }

  private nearestAxisCandidate(u: number, v: number): number {
    const tol = this.deps.engine.planePixelWorld() * 10;
    let best = -1;
    let bestD = tol;
    this.revolveAxisCandidates.forEach((c, i) => {
      const d = distPointSeg(u, v, c.a, c.b);
      if (d <= bestD) {
        bestD = d;
        best = i;
      }
    });
    return best;
  }

  /** Apply an in-progress angle drag from the current pointer x (Alt suppresses snap). */
  private applyRevolveDrag(clientX: number): void {
    if (!this.revolveAxis || !this.revolveProfile) return;
    this.revolveLastX = clientX;
    const raw = angleFromDrag(this.revolveStartAngle, clientX - this.revolveDownX);
    const angle = snapRevolveAngle(raw, this.altHeld);
    this.revolve = revolveStep(this.revolve, { kind: "drag", angle }).state;
    this.deps.engine.setRevolveAngle(this.revolveProfile.ring, this.revolveAxis, angle);
    toolChipStore.getState().setValue(angle);
    this.sendPreview(); // exact L2 under the L1 lathe (throttled)
    this.updateDebug(); // publish the live angle + throttle epoch (extrude parity)
  }

  /**
   * Commit the armed revolve (Enter / chip-✓ / click-away — MODEL-HARDEN Wave 1+2).
   * A re-edit is an angle-only deep-merge on the stored params; a fresh revolve loops
   * `applyOperation` once per selected region (N==1 single-region), each with the same
   * axis / angle / boolean mode + target. STOP ON FIRST FAILURE: committed regions
   * stay (real rows), the tool returns to armed (lathe preview kept) with a named
   * hint, and the committed regions are dropped so a re-confirm retries only the rest.
   * On full success the tool waits for ALL bodies, selects them, and auto-hides the
   * consumed sketch. A commitGen token guards every await.
   */
  private async confirmRevolve(): Promise<void> {
    if (this.revolve.phase !== "armed") return;
    // Degenerate-angle guard (finding 13): the machine refuses a confirm at ~0° — stay
    // armed and nudge the user instead of committing a zero-thickness revolve.
    const step = revolveStep(this.revolve, { kind: "confirm" });
    if (step.effect !== "commit") {
      viewportStore.getState().setStatusHint("Set a non-zero angle", { severity: "error", sticky: true });
      return;
    }
    // SCHEMA §7.6: a structural preview failure must DISABLE commit (extrude parity).
    // Scoped to a revolve-owned lane so a stale failure from another tool can't wedge it.
    if (this.previewOwner === "revolve" && this.previewFailure) {
      traceWarn("revolve", `confirm BLOCKED by preview failure: ${this.previewFailure.message}`);
      viewportStore
        .getState()
        .setStatusHint(`Cannot confirm invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const angle = this.revolve.angle;
    const sketchId = this.revolveSketchId;
    const regionIds = this.revolveRegionIds.length
      ? this.revolveRegionIds
      : this.revolveRegionId
        ? [this.revolveRegionId]
        : [];
    if (!sketchId || regionIds.length === 0 || !this.revolveProfile) {
      this.finishRevolve([]);
      return;
    }
    const gen = ++this.commitGen;
    this.revolve = step.state; // → committing
    toolStore.setState({ phase: "committing" });
    const axis: AxisRef | undefined = this.revolveAxisLineId
      ? { kind: "sketchLine", sketchId, lineId: this.revolveAxisLineId }
      : undefined;
    const booleanMode = this.revolve.booleanMode as FeatureBooleanMode;
    const targetBodyId = this.revolve.targetBodyId ?? undefined;

    // Profile-record guarantee (EXTRUDE-COMMIT-FIX, extended to Revolve): the regen
    // planner resolves the profile ONLY from the sketch's `Sketch` timeline record,
    // and arming is now a PURE read (`getSketchRegions`) that authors none — so the
    // record must be guaranteed HERE, at the exact boundary that needs it, above BOTH
    // the re-edit branch and the fresh loop. finishSketch is idempotent (unchanged
    // content upserts nothing). Failure returns to armed with a named hint and never
    // touches sessions, so a retry is possible.
    try {
      await this.client.finishSketch(sketchId);
    } catch (e) {
      if (gen !== this.commitGen) return;
      traceWarn("revolve", `commit: profile-record guarantee FAILED: ${errMessage(e)}`);
      this.revolve = revolveStep(this.revolve, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`Revolve failed: cannot record profile sketch: ${errMessage(e)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.commitGen) return;

    // Subscribe before committing so an early body (async doc-changed → mesh-ingest)
    // isn't missed while a later region is still committing (mirrors confirmExtrude).
    const loaded = new Set<string>();
    this.commitRevolveBodyUnsub?.();
    this.commitRevolveBodyUnsub = this.deps.onBodyLoaded((id) => loaded.add(id));

    // Re-edit path: angle-only deep-merge (single region, keeps the axis + target).
    if (this.revolveEditFeatureId && this.revolveStoredParams) {
      try {
        const res = await this.client.applyEditCommand(
          updateScalarParamsCommand(this.revolveEditFeatureId, "Revolve", this.revolveStoredParams, {
            angleDeg: { value: angle },
          }),
        );
        if (gen !== this.commitGen) return;
        this.applyResult(res);
        // Op-scoped bodies (finding 2): a re-edit's result is already this record's
        // own bodies — take ALL of them (a Cut re-edit yields split children).
        const bodyIds = res.changedBodies.map((b) => b.bodyId);
        if (bodyIds.length > 0) this.finishRevolveAll(bodyIds, gen, loaded);
        else {
          this.commitRevolveBodyUnsub?.();
          this.commitRevolveBodyUnsub = null;
          await this.rollbackFailedCommit();
          if (gen !== this.commitGen) return;
          this.onRevolveCommitFailed(0, 1, res.errorMessage ?? "Revolve failed", gen);
        }
      } catch (e) {
        if (gen !== this.commitGen) return;
        this.commitRevolveBodyUnsub?.();
        this.commitRevolveBodyUnsub = null;
        this.onRevolveCommitFailed(0, 1, errMessage(e), gen);
      }
      return;
    }

    const total = regionIds.length;
    const committedBodyIds: string[] = [];
    for (let k = 0; k < total; k++) {
      // The open lane session for THIS region. Sessions are opened in `regionIds`
      // order at axis pick and sliced in lockstep by `onRevolveCommitFailed`, so the
      // index correspondence holds across a partial failure. `undefined` only when
      // `beginPreview` itself failed — the commit then still materializes the SAME op
      // through `applyOperation` rather than refusing what the user armed.
      const es = this.previewSessions[k];
      let res: ApplyOperationResult | null;
      if (es) {
        // Push this session's FINAL params as the newest epoch, wait for the matching
        // exact candidate, then commit THAT candidate — preview and commit are one op
        // (same `opId`, same params), which is the whole point of the lane.
        const now = performance.now();
        const params = this.revolvePreviewParams();
        this.throttle.request(params, now);
        const send = this.throttle.flush(now);
        this.commitFinalEpoch = send ? send.epoch : this.throttle.epoch;
        this.client.updatePreview(
          es.session.sessionId,
          send?.params ?? params,
          this.commitFinalEpoch,
        );

        const exact = await this.waitForExactPreview(es.session.sessionId);
        if (gen !== this.commitGen) return;
        if (!exact.ok) {
          // Nothing consumed this lane session yet — release it so the re-arm below
          // does not leak it.
          void this.client.endPreview(es.session.sessionId, false);
          this.commitRevolveBodyUnsub?.();
          this.commitRevolveBodyUnsub = null;
          this.onRevolveCommitFailed(k, total, exact.error.message, gen);
          return;
        }
        try {
          res = await this.client.endPreview(es.session.sessionId, true);
        } catch (e) {
          if (gen !== this.commitGen) return;
          this.commitRevolveBodyUnsub?.();
          this.commitRevolveBodyUnsub = null;
          this.onRevolveCommitFailed(k, total, errMessage(e), gen);
          return;
        }
      } else {
        const op: OperationOp = {
          opType: "Revolve",
          sketchId,
          regionId: regionIds[k],
          regionIdentityVersion: this.revolveRegionIdentityVersions[k],
          inputs: [{ primary: { bodyId: "", kind: "face" }, anchor: {} }],
          params: { angleDeg: angle, axis, booleanMode, ...(targetBodyId ? { targetBodyId } : {}) },
        };
        try {
          res = await this.client.applyOperation(op);
        } catch (e) {
          if (gen !== this.commitGen) return;
          this.commitRevolveBodyUnsub?.();
          this.commitRevolveBodyUnsub = null;
          this.onRevolveCommitFailed(k, total, errMessage(e), gen);
          return;
        }
      }
      if (gen !== this.commitGen) return;
      // Extrude parity: a removal-ONLY result is a SUCCESS (a Cut revolve that
      // consumes its target entirely changes no body but removes one).
      if (!res || (res.changedBodies.length === 0 && res.removedBodies.length === 0)) {
        this.commitRevolveBodyUnsub?.();
        this.commitRevolveBodyUnsub = null;
        await this.rollbackFailedCommit();
        if (gen !== this.commitGen) return;
        this.onRevolveCommitFailed(k, total, res?.errorMessage ?? "Revolve failed", gen);
        return;
      }
      this.applyResult(res);
      // Op-scoped bodies (finding 2): each region's op result carries only its own
      // bodies (incl. split children) — collect them all across the loop.
      for (const b of res.changedBodies) committedBodyIds.push(b.bodyId);
    }
    this.finishRevolveAll(committedBodyIds, gen, loaded);
  }

  /** Wait for ALL committed revolve bodies to enter the scene, then teardown + select. */
  private finishRevolveAll(bodyIds: string[], gen: number, loaded: Set<string>): void {
    const pending = new Set(bodyIds.filter((id) => !loaded.has(id)));
    const done = (): void => {
      if (this.commitRevolveBodyTimer) {
        clearTimeout(this.commitRevolveBodyTimer);
        this.commitRevolveBodyTimer = null;
      }
      this.commitRevolveBodyUnsub?.();
      this.commitRevolveBodyUnsub = null;
      if (gen !== this.commitGen) return;
      this.finishRevolve(bodyIds);
    };
    if (pending.size === 0) {
      done();
      return;
    }
    this.commitRevolveBodyUnsub?.();
    this.commitRevolveBodyUnsub = this.deps.onBodyLoaded((id) => {
      if (!pending.has(id)) return;
      pending.delete(id);
      if (pending.size === 0) done();
    });
    // Bounded wait (finding 8): a committed body that never ingests (e.g. hidden ⇒ no
    // mesh) must not hang the tool — finish with whatever loaded after the timeout.
    this.commitRevolveBodyTimer = setTimeout(done, bodyLoadTimeoutMs);
  }

  /**
   * A failed revolve commit at region k of `total`: back to armed (lathe preview
   * kept), named error hint, committed regions dropped so a re-confirm retries only
   * the failed + remaining.
   */
  private onRevolveCommitFailed(k: number, total: number, reason: string, gen: number): void {
    traceWarn("revolve", `commit ${k + 1}/${total} FAILED → re-arming: ${reason}`);
    this.revolve = revolveStep(this.revolve, { kind: "commitFailed" }).state; // committing → armed
    toolStore.setState({ phase: "armed" });
    const msg =
      total > 1
        ? `Revolve ${k + 1} of ${total} failed: ${reason} — remaining kept`
        : `Revolve failed: ${reason}`;
    viewportStore.getState().setStatusHint(msg, { severity: "error", sticky: true });
    if (this.revolveRegionIds.length) {
      this.revolveRegionIds = this.revolveRegionIds.slice(k);
      this.revolveRegionIdentityVersions = this.revolveRegionIdentityVersions.slice(k);
      this.revolveProfiles = this.revolveProfiles.slice(k);
      this.revolveRegionId = this.revolveRegionIds[0] ?? this.revolveRegionId;
      this.revolveProfile = this.revolveProfiles[0] ?? this.revolveProfile;
    }
    // Rebuild the lathe preview for the NEW primary (first REMAINING) region — the
    // already-committed region's shell would otherwise linger (finding 12).
    if (this.plane && this.revolveProfile && this.revolveAxis) {
      this.deps.engine.showRevolvePreview(this.plane, this.revolveProfile.ring, this.revolveAxis, this.revolve.angle);
    }
    void this.rearmRemainingRevolve(k, gen);
    this.updateDebug();
  }

  /**
   * Re-open the failed region's lane session after a partial revolve commit, so the
   * user's work is never lost (`rearmRemainingExtrude`'s revolve twin). The already
   * COMMITTED sessions [0,k) were consumed by `endPreview(true)` and the failed one
   * was released, so only their stale L2 meshes need dropping; the survivors keep
   * their sessions and re-render from the `sendPreview()` below.
   */
  private async rearmRemainingRevolve(k: number, gen: number): Promise<void> {
    if (this.previewSessions.length === 0) return; // no lane in play (fallback commit)
    this.engine.clearPreviewBody();
    for (let i = 0; i < k; i++) this.removeExactPreviewMeshes(this.previewSessions[i]);
    const failed = this.previewSessions[k];
    const remaining = this.previewSessions.slice(k + 1);
    if (!failed) {
      this.previewSessions = remaining.map((s) => ({ ...s, lastAppliedEpoch: 0 }));
      this.throttle.reset();
      this.sendPreview();
      return;
    }
    this.removeExactPreviewMeshes(failed);
    let freshSession: PreviewSession;
    try {
      freshSession = await this.deps.client.beginPreview(failed.draft);
    } catch {
      // The lane could not be re-opened: the L1 lathe + the armed FSM still carry the
      // user's parameters, and a re-confirm falls back to `applyOperation`.
      this.previewSessions = remaining.map((s) => ({ ...s, lastAppliedEpoch: 0 }));
      this.previewOwner = this.previewSessions.length > 0 ? this.previewOwner : null;
      this.previewParamsFn = this.previewSessions.length > 0 ? this.previewParamsFn : null;
      return;
    }
    if (gen !== this.commitGen) {
      void this.deps.client.endPreview(freshSession.sessionId, false); // superseded — don't leak
      return;
    }
    // Reset every per-session lastAppliedEpoch: the throttle.reset() below restarts
    // epochs from a lower value, and a survivor holding a LARGE epoch would stale-drop
    // every new result and freeze its preview (extrude finding 9).
    this.previewSessions = [
      { ...failed, session: freshSession, lastAppliedEpoch: 0 },
      ...remaining.map((s) => ({ ...s, lastAppliedEpoch: 0 })),
    ];
    this.throttle.reset();
    this.throttle.setTrailingMs(REVOLVE_PREVIEW_TRAILING_MS);
    this.sendPreview();
    this.updateDebug();
  }

  private finishRevolve(bodyIds: string[]): void {
    this.deps.engine.hideRevolvePreview();
    this.deps.engine.setOrbitSuppressed(false);
    // The lane sessions were CONSUMED by the commit (endPreview(true)) — drop their
    // exact candidate meshes and release ownership so a later sendPreview() cannot
    // fire against a tool that is no longer armed (finishExtrude parity; no
    // endPreview here, which would be a second call on a consumed session).
    for (const es of this.previewSessions) this.removeExactPreviewMeshes(es);
    this.engine.clearPreviewBody();
    this.previewSessions = [];
    this.previewOwner = null;
    this.previewParamsFn = null;
    this.previewPending = false;
    this.previewPendingHint = false;
    this.previewFailure = null;
    this.clearExactPreviewWaiters();
    this.throttle.reset();
    this.engine.setPreviewTint("normal");
    toolChipStore.getState().clear();
    this.revolve = revolveStep(this.revolve, { kind: "settle" }).state;
    const consumedSketch = this.lastArmedSketch;
    const wasReedit = this.revolveEditFeatureId !== undefined;
    this.revolveProfile = null;
    this.revolveProfiles = [];
    this.revolveRegionIds = [];
    this.revolveRegionIdentityVersions = [];
    this.revolveAxis = null;
    this.revolveAxisLineId = null;
    this.revolveAxisCandidates = [];
    this.revolveEditFeatureId = undefined;
    this.revolveStoredParams = undefined;
    this.revolveArmedDown = false;
    if (this.dragging === "revolve") this.dragging = null;
    const unique = bodyIds.filter((id, i, a) => a.indexOf(id) === i);
    let completionHint: string | undefined;
    if (unique.length > 0) {
      selectionStore.getState().set(unique.map((id) => ({ kind: "body" as const, id })));
      completionHint = unique.length > 1 ? `Revolved ${unique.length} bodies` : "Revolved";
      // Consumed sketch auto-hides. Backend-backed (TRUST wave): a local flip pops
      // back visible on the next projection. Issued from `finishRevolve`, i.e. only
      // after the WHOLE commit loop terminated — `rollbackFailedCommit` never reaches
      // here, so an extra undo step is the only cost.
      if (consumedSketch && !wasReedit) void setSketchVisible(consumedSketch, false);
    }
    this.resetToSelect(completionHint);
    this.updateDebug();
  }

  private async armEdgeOpFromSelection(
    kind: EdgeOpKind,
    opts?: { auto?: boolean },
  ): Promise<void> {
    const selected = selectionStore.getState().selected;
    const edges = selected.filter((r) => r.kind === "edge");
    if (edges.length === 0) {
      const verdict = getToolApplicability("fillet", selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? `Select edges, then ${kind}`, { sticky: true });
      return;
    }
    const gen = ++this.armGen;
    if (!(await this.prepareEdgeClosure(gen, kind, edges))) return;
    if (gen !== this.armGen) return;
    this.filletEditFeatureId = undefined;
    this.computeEdgeOpOutward();
    // Direction-driven typing is armed ONLY on the bisector tier. The bbox proxy is
    // convex-only — on a pocket edge it points INTO material, so an automatic flip
    // there would author the op the user did not ask for. Off that tier the chip
    // segments are the type control and the drag only sizes.
    const auto = opts?.auto === true && this.filletAxisSource === "bisector";
    const size = kind === "Chamfer" ? DEFAULT_CHAMFER_DISTANCE : DEFAULT_FILLET_RADIUS;
    this.fillet = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: this.filletEdges.length,
      radius: size,
      edgeOp: kind,
      auto,
    }).state;
    toolStore.setState({ phase: "armed" });
    // A resolved world axis gets a real handle (the extrude/offset-face rule): only
    // a press ON it starts the drag, everything else stays free to orbit/select. The
    // "screen" tier has no honest axis to put a handle on, so it keeps claiming
    // every viewport press — orbit suppressed there, exactly as before this change.
    if (this.filletDegraded) {
      this.engine.hideValueHandle();
    } else {
      this.engine.showValueHandle(this.filletAnchor, this.filletOutward as Vec3);
    }
    this.deps.engine.setOrbitSuppressed(this.filletDegraded);
    const hint = this.edgeOpArmHint();
    this.previewArmHint = hint;
    viewportStore.getState().setStatusHint(hint, { sticky: true });
    this.showEdgeOpChip(size);
    this.updateDebug();
    await this.openEdgeOpPreview(gen);
  }

  private showEdgeOpChip(size: number): void {
    // Sit BESIDE the handle, leader-lined off its base, instead of centered ON the
    // picked edge — mirrors extrude's `worldPos` = arrowhead, `anchorAxisFrom` =
    // base. Only when a handle actually exists: a degraded arm has no axis, so its
    // chip stays centered on the picked point, exactly as before this change.
    const outward = this.filletOutward;
    const degraded = this.filletDegraded || !outward;
    const anchor: [number, number, number] = degraded
      ? this.filletEdges[0].anchor?.worldPoint ?? [0, 0, 0]
      : [
          this.filletAnchor[0] + outward[0],
          this.filletAnchor[1] + outward[1],
          this.filletAnchor[2] + outward[2],
        ];
    toolChipStore.getState().showFillet(
      size,
      anchor,
      (v) => this.onFilletChip(v),
      {
        onConfirm: () => void this.commitFillet(),
        onCancel: () => toolStore.getState().setTool("select"),
      },
      {
        edgeOp: this.fillet.edgeOp,
        showEdgeOpSegments: true,
        onEdgeOp: (k) => this.onEdgeOpChip(k),
        distance2: this.fillet.distance2,
        onDistance2: (v) => this.onEdgeOpDistance2(v),
        anchorAxisFrom: degraded ? undefined : this.filletAnchor,
      },
    );
  }

  private async prepareEdgeClosure(
    gen: number,
    kind: EdgeOpKind,
    picks: EntityRef[],
  ): Promise<boolean> {
    this.filletPreparedRevision = null;
    let res: PrepareEdgeOpResult;
    try {
      res = await this.client.prepareEdgeOp({
        mode: kind,
        pickedEdges: picks.map((pick) => ({
          bodyId: pick.bodyId,
          // The selection's TopoKey names the exact fenced snapshot and lets the
          // response return promotion evidence. ElementId is the fallback only.
          topoKey: pick.topoKey,
          elementId: pick.topoKey ? undefined : pick.elementId,
        })),
        chainTangentEdges: true,
      });
    } catch (error) {
      if (gen !== this.armGen) return false;
      this.publishEdgePrepareFailure(kind, errMessage(error));
      return false;
    }
    if (gen !== this.armGen) return false;
    if (res.refusal) {
      this.publishEdgePrepareFailure(kind, res.refusal.message);
      return false;
    }
    return this.adoptPreparedEdges(gen, kind, picks, res);
  }

  private adoptPreparedEdges(
    gen: number,
    kind: EdgeOpKind,
    picks: EntityRef[],
    res: PrepareEdgeOpResult,
  ): boolean {
    const bodyId = picks[0]?.bodyId ?? "";
    if (!bodyId || bareBodyId(res.targetBodyId) !== bareBodyId(bodyId) || res.edges.length === 0) {
      this.publishEdgePrepareFailure(kind, "the closure resolved to invalid topology");
      return false;
    }
    try {
      const topoKeys = new Set(res.edges.map((edge) => edge.topoKey));
      if (topoKeys.size !== res.edges.length) throw new Error("prepared closure contains duplicate edges");
      this.filletEdges = res.edges.map((edge) => {
        if (
          edge.kind !== "edge" ||
          !edge.elementId ||
          bareBodyId(edge.bodyId) !== bareBodyId(bodyId)
        ) {
          throw new Error(`edge ${edge.topoKey} was not promoted`);
        }
        const anchor = edge.anchor?.worldPoint
          ? { worldPoint: edge.anchor.worldPoint, surfaceUv: edge.anchor.surfaceUv }
          : picks.find((pick) => pick.topoKey === edge.topoKey)?.anchor;
        return {
          kind: "edge" as const,
          id: topoRefId(bodyId, edge.topoKey),
          bodyId,
          topoKey: edge.topoKey,
          elementId: edge.elementId,
          anchor,
        };
      });
      this.filletPreparedRevision = documentStore.getState().revision;
      return true;
    } catch (error) {
      if (gen === this.armGen) this.publishEdgePrepareFailure(kind, errMessage(error));
      return false;
    }
  }

  private publishEdgePrepareFailure(kind: EdgeOpKind, reason: string): void {
    viewportStore.getState().setStatusHint(`${kind} unavailable: ${reason}`, {
      severity: "error",
      sticky: true,
    });
  }

  /** The armed edge op's status line, naming the CURRENT type (a drag flip or a
   *  segment pick republishes it). */
  private edgeOpArmHint(): string {
    const kind = this.fillet.edgeOp;
    const noun = kind === "Chamfer" ? "distance" : "radius";
    // A RE-EDIT arms with NO picks (`filletEdges` is empty by construction — the
    // edges live in the stored params), so the pick-count wording would read
    // "0 edges". One hint source so a type flip republishes the right sentence.
    if (this.filletEditFeatureId) {
      return `Edit ${kind.toLowerCase()} ${noun} — drag or type, Enter to apply`;
    }
    const n = this.filletEdges.length;
    return `${kind} ${n} edge${n > 1 ? "s" : ""} — drag or type ${noun} · Enter or ✓ to apply`;
  }

  /**
   * Resolve the world "away from the body" direction for the armed edges — ONCE,
   * at arm time.
   *
   * Deliberately NOT lazy: `MeshEntry`s are double-buffered on every mesh swap
   * (`meshRegistry.swap`), so a mid-drag lookup could read an entry whose edge
   * ordinals no longer mean what the user picked. The arm is already dropped on
   * any document change (`onDocumentRevisionChanged`), which is exactly the event
   * that could invalidate this.
   *
   * At most {@link EDGE_OP_OUTWARD_SAMPLE} edges contribute: the mean only has to
   * be good enough to orient one drag axis, and a 200-edge chain selection must
   * not walk every mesh. A mean the tier math refuses (`averageOutward` null —
   * edges facing substantially different ways) leaves NO direction rather than a
   * fabricated one.
   */
  private computeEdgeOpOutward(): void {
    this.filletOutward = null;
    this.filletTangent = null;
    this.filletAnchor = this.filletEdges[0]?.anchor?.worldPoint ?? [0, 0, 0];
    this.filletAxisSource = "screen";

    const dirs: Vec3[] = [];
    const mids: Vec3[] = [];
    let tangent: Vec3 | null = null;
    // Every contributing edge must have resolved via the bisector for the arm to
    // count as bisector-tier — one bbox fallback in the mean poisons the sign.
    let allBisector = true;
    for (const ref of this.filletEdges.slice(0, EDGE_OP_OUTWARD_SAMPLE)) {
      const entry = ref.bodyId ? getEntry(ref.bodyId) : undefined;
      const index = entry?.edgeIndex;
      if (!entry || !index) {
        allBisector = false;
        continue;
      }
      const ord = index.ordinalForId(ref.topoKey ?? ref.id);
      if (ord < 0) {
        allBisector = false;
        continue;
      }
      const res = edgeOutward(entry.view, ord);
      if (!res) {
        allBisector = false;
        continue;
      }
      if (res.source !== "bisector") allBisector = false;
      dirs.push(res.outward);
      mids.push(res.mid);
      tangent = res.tangent;
    }
    if (dirs.length === 0) return;
    const mean = averageOutward(dirs);
    if (!mean) return;
    this.filletOutward = mean;
    this.filletAnchor = [
      mids.reduce((a, m) => a + m[0], 0) / mids.length,
      mids.reduce((a, m) => a + m[1], 0) / mids.length,
      mids.reduce((a, m) => a + m[2], 0) / mids.length,
    ];
    // A tangent is only meaningful for ONE edge; averaging two tangents produces a
    // direction neither edge has, and subtracting it would tilt the axis wrongly.
    this.filletTangent = dirs.length === 1 ? tangent : null;
    this.filletAxisSource = allBisector ? "bisector" : "bbox";
  }

  /**
   * The screen axis a value drag is measured along, resolved PER GRAB (the camera
   * may have orbited since the arm). Finite difference rather than a transformed
   * vector: one projection of the anchor and one of a point a few pixels' worth of
   * world along `outward` capture perspective foreshortening exactly.
   *
   * Falls back to the screen convention (up grows) whenever there is nothing
   * honest to project — no outward direction, an engine without `projectPoint`
   * (test mocks), a point behind the camera, or a collapsed projected difference
   * (a head-on edge, which `screenDragAxis` refuses rather than normalizing noise).
   */
  private edgeOpScreenAxis(): ScreenAxis {
    const outward = this.filletOutward;
    if (!outward) return SCREEN_UP_AXIS;
    const engine = this.engine as Partial<ViewportEngine>;
    if (typeof engine.projectPoint !== "function") return SCREEN_UP_AXIS;
    const project = engine.projectPoint.bind(this.engine);
    const step = 10 * this.engine.planePixelWorld();
    const a = this.filletAnchor;
    const along = (d: Vec3): Vec3 => [a[0] + step * d[0], a[1] + step * d[1], a[2] + step * d[2]];
    const pMid = project(a);
    const pOut = project(along(outward));
    if (!pMid || !pOut) return SCREEN_UP_AXIS;
    const pTan = this.filletTangent ? project(along(this.filletTangent)) : null;
    return screenDragAxis(pMid, pOut, pTan ?? undefined) ?? SCREEN_UP_AXIS;
  }

  /**
   * Commit a Fillet↔Chamfer change — the ONE place a type flip becomes real,
   * whether the drag direction or a chip segment caused it.
   *
   * `beginPreview` FREEZES the draft's `opType` (`localSolver.ts:495-508`; the
   * per-opType builders in `previewOps.ts` are chosen there), so a type change is
   * a session close + REOPEN, never a params patch. `armGen` is bumped BEFORE the
   * close: two hysteresis crossings inside one in-flight `beginPreview` would
   * otherwise both pass the gen fence, installing a stale-kind session and leaking
   * the other.
   */
  private applyEdgeOpKindChange(): void {
    toolChipStore.getState().setEdgeOp(this.fillet.edgeOp);
    const hint = this.edgeOpArmHint();
    this.previewArmHint = hint;
    viewportStore.getState().setStatusHint(hint, { sticky: true });
    const gen = ++this.armGen;
    const wasDragging = this.dragging === "fillet";
    this.closePreviewSessions();
    // A RE-EDIT never opens a preview: PreviewOp runs against the CURRENT head, so
    // previewing an existing feature double-applies it (the rule `armShell` states
    // for its own re-edit). The flip there is chip + record only.
    if (!this.filletEditFeatureId) {
      void this.openEdgeOpPreview(gen).then(() => {
        // The reopen re-raises the ARMED trailing floor; a flip DURING a drag has to
        // get the drag floor back or the value stops tracking the pointer.
        if (wasDragging && this.dragging === "fillet") {
          this.throttle.setTrailingMs(EDGE_OP_DRAG_TRAILING_MS);
        }
      });
    }
    this.updateDebug();
  }

  /** A [Fillet|Chamfer] segment was clicked: lock the type for the session. */
  private onEdgeOpChip(edgeOp: EdgeOpKind): void {
    // Idempotent — but only once the type is already LOCKED. Re-picking the active
    // segment while the drag still owns the type is the gesture that kills `auto`,
    // so it must fall through.
    if (edgeOp === this.fillet.edgeOp && !this.fillet.auto) return;
    this.fillet = filletStep(this.fillet, { kind: "setEdgeOp", edgeOp }).state;
    toolChipStore.getState().setValue(this.fillet.radius); // a pristine arm reseeds
    this.applyEdgeOpKindChange();
  }

  /**
   * Open the ONE kernel-preview session an armed edge op drives.
   *
   * DUAL-FIELD LOCKSTEP (`ipc/previewOps.ts edgeOpBuilder` REFUSES a mismatch, and
   * `tauriCommandMap.filletParams` silently drops the typed `params.edges` on one):
   * the draft's `inputs` and `params.edgeIds` are both derived from `this.filletEdges`,
   * in that array's order, by the two helpers below and nowhere else.
   */
  private async openEdgeOpPreview(gen: number): Promise<void> {
    const draft: PreviewDraft = {
      opType: this.edgeOpKind,
      inputs: this.edgeOpInputs(),
      params: this.edgeOpPreviewParams(),
    };
    let session: PreviewSession;
    try {
      session = await this.deps.client.beginPreview(draft);
    } catch (error) {
      if (gen !== this.armGen) return;
      // The chip stays armed and usable: commitEdgeOp falls back to a plain
      // applyOperation when no session exists, so a lane refusal costs the
      // preview, never the tool.
      traceWarn(
        this.edgeOpKind.toLowerCase(),
        `${this.edgeOpKind} preview session failed: ${errMessage(error)}`,
      );
      return;
    }
    if (gen !== this.armGen) {
      void this.deps.client.endPreview(session.sessionId, false);
      return;
    }
    this.previewSessions = [{ session, draft, lastAppliedEpoch: 0, previewBodyIds: [], replacedBodyIds: [] }];
    this.previewOwner = "edgeOp";
    this.previewParamsFn = () => this.edgeOpPreviewParams();
    this.previewPending = false;
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    this.throttle.reset();
    // A fillet/chamfer rebuild is dearer than a prism sweep — pace the armed lane
    // slower than the 80ms drag floor (restored on every release).
    this.throttle.setTrailingMs(EDGE_OP_TRAILING_MS);
    this.sendPreview();
    this.updateDebug();
  }

  /** The typed per-edge refs — SAME array, SAME order as {@link edgeOpPreviewParams}. */
  private edgeOpInputs(): SemanticRef[] {
    return this.filletEdges.map((e) => this.semanticRefFor(e));
  }

  /**
   * Complete canonical edge-op params for both exact preview and commit — ONE
   * derivation of `edgeIds`, from the same `filletEdges` array `edgeOpInputs`
   * reads, so the two can never drift out of lockstep.
   */
  private edgeOpParams(radius = this.fillet.radius): FilletParams {
    const params: FilletParams = {
      mode: this.edgeOpKind,
      radius,
      edgeIds: this.filletEdges.map((e) => e.elementId ?? ""),
      chainTangentEdges: true,
      tangentClosureVersion: 1,
    };
    // SCHEMA §7.3 (2026-08-03): the second leg is CHAMFER-ONLY, and absent means
    // equal-leg. The FSM KEEPS the user's number across a type flip (so flipping
    // back hands it straight back), which is exactly why the gate belongs here at
    // the emitting seam rather than in the reducer — a Fillet must never author
    // it, whatever the arm still remembers.
    if (this.edgeOpKind === "Chamfer" && this.fillet.distance2 !== null) {
      params.distance2 = this.fillet.distance2;
    }
    return params;
  }

  /** The same params as a lane {@link PreviewParams} (which carries an index signature). */
  private edgeOpPreviewParams(radius = this.fillet.radius): PreviewParams {
    return { ...this.edgeOpParams(radius) };
  }

  private onFilletChip(v: number): void {
    this.fillet = filletStep(this.fillet, { kind: "setRadius", radius: v }).state;
    toolChipStore.getState().setValue(v);
    this.sendPreview();
  }

  /**
   * The chamfer second-leg field was typed or cleared back to `=` (SCHEMA §7.3).
   *
   * Unlike a type flip this is a plain PARAMS change, so the live preview session
   * stays open and is simply re-sent — `beginPreview` freezes `opType`, not the
   * params. A re-edit has no session and `sendPreview` is a no-op there.
   */
  private onEdgeOpDistance2(distance2: number | null): void {
    this.fillet = filletStep(this.fillet, { kind: "setDistance2", distance2 }).state;
    // Read the value BACK off the FSM — it normalizes (clamp / non-positive ⇒
    // equal-leg), so echoing the raw input to the chip could show a number the
    // op will not carry.
    toolChipStore.getState().setDistance2(this.fillet.distance2);
    this.sendPreview();
  }

  private startBooleanFromSelection(): void {
    const selected = selectionStore.getState().selected;
    const body = selected.find((r) => r.kind === "body");
    if (!body) {
      const verdict = getToolApplicability("boolean", selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    this.boolean = booleanStep(booleanInit(), { kind: "start", targetBodyId: body.id }).state;
    toolStore.setState({ phase: "armed" });
    // Modal: the click picks the tool BODY, so an LMB drag must not orbit instead.
    this.deps.engine.setOrbitSuppressed(true);
    viewportStore.getState().setStatusHint("Pick the tool body to combine", { sticky: true });
    this.updateDebug();
  }

  // ── shell ──────────────────────────────────────────────────────────────────
  //
  // Shell mirrors fillet: it arms from a FACE selection (selected faces = removed
  // faces) and a vertical drag (or the mm chip) sets the wall thickness. Release
  // keeps it ARMED; Enter / chip-✓ commits. There is no cheap L1 mesh (hollowing
  // needs OCCT) — the visible preview is the kernel's exact candidate.

  private async armShellFromSelection(): Promise<void> {
    const selected = selectionStore.getState().selected;
    const faces = selected.filter((r) => r.kind === "face");
    if (faces.length === 0) {
      const verdict = getToolApplicability("shell", selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    await this.armShell(faces);
  }

  private async armShell(
    faces: EntityRef[],
    editFeatureId?: string,
    startThickness = DEFAULT_SHELL_THICKNESS,
  ): Promise<void> {
    const gen = ++this.armGen;
    this.shellFaces = faces;
    this.shellEditFeatureId = editFeatureId;
    this.shell = shellStep(shellInit(), {
      kind: "arm",
      // A re-edit has no fresh face picks yet — faceCount 1 keeps the FSM out of
      // its bail path (mirrors the fillet re-edit seed).
      faceCount: editFeatureId ? 1 : faces.length,
      thickness: startThickness,
    }).state;
    toolStore.setState({ phase: "armed" });
    this.deps.engine.setOrbitSuppressed(true); // modal: drag adjusts thickness, not orbit
    const n = faces.length;
    const hint = editFeatureId
      ? "Edit shell thickness — drag or type, Enter to apply"
      : `Shell ${n} face${n > 1 ? "s" : ""} — drag or type thickness · Enter or ✓ to apply`;
    this.previewArmHint = editFeatureId ? null : hint;
    viewportStore.getState().setStatusHint(hint, { sticky: true });
    const anchor = faces[0]?.anchor?.worldPoint ?? [0, 0, 0];
    if (editFeatureId) {
      toolChipStore.getState().showShell(startThickness, anchor, (v) => {
        this.onShellChip(v);
        void this.commitShell(); // chip Enter/blur commits the thickness-only re-edit
      });
    } else {
      toolChipStore.getState().showShell(startThickness, anchor, (v) => this.onShellChip(v), {
        onConfirm: () => void this.commitShell(),
        onCancel: () => toolStore.getState().setTool("select"),
      });
    }
    this.updateDebug();
    // A re-edit runs L1-only: PreviewOp executes against the CURRENT head, so
    // previewing an existing feature would double-apply it (the extrude re-edit
    // rule, applied here too).
    if (!editFeatureId) await this.openShellPreview(gen);
  }

  /**
   * Open the ONE kernel-preview session an armed shell drives. The open faces and
   * the shelled body both come from `this.shellFaces`. `wireOperation` uses the
   * typed `inputs` to persist `params.faces` beside `params.openFaces`; Rust then
   * derives worker inputs from that typed evidence. See `ipc/previewOps.ts`.
   */
  private async openShellPreview(gen: number): Promise<void> {
    const draft: PreviewDraft = {
      opType: "Shell",
      inputs: this.shellFaces.map((f) => this.semanticRefFor(f)),
      params: this.shellPreviewParams(),
    };
    let session: PreviewSession;
    try {
      session = await this.deps.client.beginPreview(draft);
    } catch (error) {
      if (gen !== this.armGen) return;
      traceWarn("shell", `Shell preview session failed: ${errMessage(error)}`);
      return;
    }
    if (gen !== this.armGen) {
      void this.deps.client.endPreview(session.sessionId, false);
      return;
    }
    this.previewSessions = [{ session, draft, lastAppliedEpoch: 0, previewBodyIds: [], replacedBodyIds: [] }];
    this.previewOwner = "shell";
    this.previewParamsFn = () => this.shellPreviewParams();
    this.previewPending = false;
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    this.throttle.reset();
    this.throttle.setTrailingMs(SHELL_TRAILING_MS);
    this.sendPreview();
    this.updateDebug();
  }

  /** Complete canonical Shell params for both exact preview and commit. */
  private shellParams(thickness = this.shell.thickness): ShellParams {
    return {
      thickness,
      openFaces: this.shellFaces.map((f) => f.elementId ?? f.topoKey ?? f.id),
      targetBodyId: this.shellFaces[0]?.bodyId ?? "",
    };
  }

  private shellPreviewParams(thickness = this.shell.thickness): PreviewParams {
    return { ...this.shellParams(thickness) };
  }

  private onShellChip(v: number): void {
    this.shell = shellStep(this.shell, { kind: "setThickness", thickness: v }).state;
    toolChipStore.getState().setValue(v);
    this.sendPreview();
  }

  /**
   * Apply the armed shell. The RE-EDIT path is unchanged (a thickness-only
   * scalar merge, no lane session); a FRESH shell runs the previewed-commit
   * sequence so a kernel refusal blocks the write instead of leaving an errored
   * row on the timeline.
   */
  private async commitShell(): Promise<void> {
    const editFeatureId = this.shellEditFeatureId;
    if (editFeatureId) {
      await this.commitShellEdit(editFeatureId);
      return;
    }
    if (this.shellFaces.length === 0) {
      this.cancelShell();
      toolStore.getState().setTool("select");
      return;
    }
    if (this.shell.phase !== "armed") return;
    if (this.previewFailure) {
      viewportStore
        .getState()
        .setStatusHint(`Cannot apply invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const step = shellStep(this.shell, { kind: "confirm" });
    if (step.effect !== "commit") return;
    this.shell = step.state; // → committing (also excludes us from the stale-arm guard)
    toolStore.setState({ phase: "committing" });
    const gen = ++this.commitGen;
    const params = this.shellParams();
    const outcome = await this.commitPreviewedOp(
      { opType: "Shell", inputs: this.shellFaces.map((f) => this.semanticRefFor(f)), params },
      gen,
    );
    if (outcome.kind === "superseded") return;
    if (outcome.kind === "failed") {
      this.shell = shellStep(this.shell, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`Shell failed: ${outcome.reason}`, { severity: "error", sticky: true });
      await this.openShellPreview(this.armGen); // re-arm the preview (work kept)
      this.updateDebug();
      return;
    }
    this.applyResult(outcome.res);
    this.teardownPreviewedTool();
    this.shell = shellInit();
    this.shellFaces = [];
    this.shellEditFeatureId = undefined;
    this.shellStoredParams = undefined;
    this.resetToSelect("Shelled");
    this.updateDebug();
  }

  /** Thickness-only parametric re-edit — deep-merges into the stored params. */
  private async commitShellEdit(editFeatureId: string): Promise<void> {
    const thickness = this.shell.thickness;
    // Move to `committing` so the document-revision bump this commit causes does
    // not read as an external model change and cancel our own arm.
    const step = shellStep(this.shell, { kind: "confirm" });
    if (step.effect === "commit") this.shell = step.state;
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    // The result message is captured, not published, until the tool has been reset —
    // see `resetToSelect` for why publishing first would lose it.
    let failure: string | null = null;
    try {
      // A re-edit changes ONLY the thickness: deep-merge into the stored params so the
      // shell's open faces + target survive (a whole-params replace would wipe them).
      if (!this.shellStoredParams) throw new Error("Stored Shell parameters are unavailable");
      const res = await this.client.applyEditCommand(
        updateScalarParamsCommand(editFeatureId, "Shell", this.shellStoredParams, {
          thickness: { value: thickness },
        }),
      );
      this.applyResult(res);
    } catch (e) {
      failure = errMessage(e);
    }
    this.shell = shellInit();
    this.shellFaces = [];
    this.shellEditFeatureId = undefined;
    this.shellStoredParams = undefined;
    if (failure !== null) {
      this.resetToSelect(`Shell failed: ${failure}`, { severity: "error", sticky: true });
    } else {
      this.resetToSelect("Shell thickness updated");
    }
    this.updateDebug();
  }

  /** Re-arm the shell tool on an existing shell feature (thickness re-edit seed). */
  async editShellFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    // Gate on `opType`, NEVER `kind`: `dto.rs feature_kind` folds Shell into the
    // `fillet` bucket, so a `kind === "shell"` guard is unsatisfiable on the real
    // Tauri lane and silently killed shell re-edit there.
    if (!feat || feat.opType !== "Shell") return;
    const thickness = thicknessFromValueText(feat.valueText);
    // Fetch the stored params so the thickness-only commit deep-merges instead of
    // wiping the shell's open faces + target (the projection does not expose them).
    const stored = await this.deps.client.getOperationParams(featureId).catch(() => undefined);
    toolStore.getState().setTool("shell"); // fires cancelShell (clears shellStoredParams)
    this.shellStoredParams = stored; // set AFTER the tool-change cancel
    await this.armShell([], featureId, thickness);
  }

  // ── offset face (SCHEMA §7.3 OffsetFace + §7.6 PrepareOffsetFace) ──────────
  //
  // Shell's ARMING (face selection → chip → kernel preview → explicit confirm)
  // with Extrude's MANIPULATOR (a real 3D arrow along the mean face normal,
  // `axisDepthFromRay`, orbit left free) and an L1 ghost of the moving faces.
  //
  // The part that is neither tool's is the AUTHORING TRANSACTION. Every other op
  // here builds its params from what the user picked; this one cannot, because the
  // operative set is not the picks — the kernel auto-propagates an offset across
  // G1-tangent junctions and cannot hold a tangent neighbour fixed, so the set that
  // will actually move has to be computed by the worker BEFORE anything is
  // authored. `PrepareOffsetFace` does that, returns EVIDENCE only (never ids —
  // minting is Rust's alone), and the arm path promotes every returned face before
  // it will arm at all. A refusal, a failed promotion, or a document that moves
  // afterwards all mean the same thing: no arm, or no commit. FAIL CLOSED
  // throughout — this op's whole point is that the frozen set is trustworthy.

  private async armOffsetFaceFromSelection(): Promise<void> {
    const selected = selectionStore.getState().selected;
    // The body cross-check comes from the FACE refs, not `selectedBodyIds()`
    // (which reads whole-body selections): an offset is defined against ONE
    // body, and a cross-body pick has no target to name. Refused here as well
    // as by the backend's own `crossBody` refusal — the local check costs no
    // round trip. Encoded in `toolApplicability.ts` (single source of truth
    // with the toolbar's gray-out check).
    const verdict = getToolApplicability("offsetFace", selected, this.applicabilityCtx());
    if (!verdict.enabled) {
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { severity: verdict.severity, sticky: true });
      return;
    }
    this.offsetPicks = selected.filter((r) => r.kind === "face");
    await this.armOffsetFace();
  }

  /**
   * Run the handshake and — only if it succeeded — arm. `distanceType`/`chain`
   * default to a fresh arm's; a chip toggle re-enters here with its new pair,
   * because both CHANGE THE CLOSURE and a params update would keep the old one.
   */
  private async armOffsetFace(
    distanceType: OffsetDistanceType = "Offset",
    chainTangentFaces = true,
    seedDistance?: number,
  ): Promise<void> {
    const gen = ++this.armGen;
    // A re-arm supersedes the previous closure's preview session outright: its
    // draft names faces that are about to be replaced.
    this.closePreviewSessions();
    this.engine.hideGhostPreview();
    this.offsetGhostHidden = false;
    const kept = this.offsetFace.phase === "idle" ? undefined : this.offsetFace;
    if (!(await this.prepareOffsetClosure(gen, distanceType, chainTangentFaces))) {
      // Refused: the hint is already published and NOTHING is armed. A partial arm
      // here would be a tool holding a closure the worker declined to compute.
      if (gen === this.armGen) {
        this.offsetFace = offsetFaceInit();
        this.engine.hideValueHandle();
        toolChipStore.getState().clear();
        this.updateDebug();
      }
      return;
    }
    if (gen !== this.armGen) return;
    const distance = seedDistance ?? this.seedOffsetDistance(distanceType, kept);
    const step = offsetFaceStep(offsetFaceInit(), {
      kind: "arm",
      faceCount: this.offsetFaces.length,
      distance,
      distanceType,
      chainTangentFaces,
      touched: kept?.touched === true,
    });
    if (step.effect !== "begin") {
      // The reducer refused the arm (a non-Offset type over a multi-face closure).
      // Say so rather than leaving a tool that looks active and does nothing.
      viewportStore.getState().setStatusHint(
        `Offset face: ${distanceType} operates on exactly one face (this selection resolved to ${this.offsetFaces.length})`,
        { severity: "error", sticky: true },
      );
      this.offsetFace = offsetFaceInit();
      this.updateDebug();
      return;
    }
    this.offsetFace = step.state;
    toolStore.setState({ phase: "armed" });
    // Orbit stays FREE while a real arrow exists (the extrude rule: the handle is
    // hit-tested, so a press that misses it is an orbit). The degraded gesture has
    // no handle to miss, so it claims every press and must suppress orbit.
    this.deps.engine.setOrbitSuppressed(this.offsetDegraded);
    const hint = this.offsetArmHint();
    this.previewArmHint = hint;
    viewportStore.getState().setStatusHint(hint, { sticky: true });
    this.showOffsetFaceChip();
    this.applyOffsetFaceState();
    this.updateDebug();
    await this.openOffsetFacePreview(gen);
  }

  /** The arm hint — it has to name the CLOSURE, which may exceed the picks. */
  private offsetArmHint(): string {
    const n = this.offsetFaces.length;
    const chained = n - this.offsetPicks.length;
    const face = `${n} face${n > 1 ? "s" : ""}`;
    const tail = chained > 0 ? ` (+${chained} tangent)` : "";
    return this.offsetDegraded
      ? `Offset ${face}${tail} — drag or type a distance · Enter or ✓ to apply`
      : `Offset ${face}${tail} — drag the arrow or type · Enter or ✓ to apply`;
  }

  /**
   * The distance a (re-)arm opens at. An ABSOLUTE type seeds from the kernel's own
   * measurement (`currentDims`) — "Ø8" must open at the hole's real diameter, not
   * at a made-up default — and only while the user has not authored one of their
   * own (the fillet pristine-reseed rule).
   */
  private seedOffsetDistance(type: OffsetDistanceType, kept?: OffsetFaceFsm): number {
    if (kept?.touched) return kept.distance;
    if (type === "Radius" && this.offsetDims.radius !== undefined) return this.offsetDims.radius;
    if (type === "Diameter" && this.offsetDims.radius !== undefined) {
      return this.offsetDims.radius * 2;
    }
    if (type === "Total" && this.offsetDims.thickness !== undefined) {
      return this.offsetDims.thickness;
    }
    return type === "Offset" ? (kept?.distance ?? DEFAULT_OFFSET_DISTANCE) : DEFAULT_OFFSET_DISTANCE;
  }

  /**
   * The authoring handshake: `PrepareOffsetFace` → promote every returned face →
   * freeze the typed refs. Returns false (with a published hint) on ANY of the
   * ways it can decline; the caller must not arm on a false.
   *
   * A REFUSAL is an ordinary answer here (SCHEMA §7.6 `ok:true`), and its message
   * is what the user reads — dropping it for a generic "could not offset" would
   * throw away the only explanation they get.
   */
  private async prepareOffsetClosure(
    gen: number,
    distanceType: OffsetDistanceType,
    chainTangentFaces: boolean,
  ): Promise<boolean> {
    const picks = this.offsetPicks;
    const bodyId = picks[0]?.bodyId ?? "";
    if (picks.length === 0 || !bodyId) return false;
    this.offsetPreparedRevision = null; // no valid handshake until this one lands
    let res: PrepareOffsetFaceResult;
    try {
      res = await this.deps.client.prepareOffsetFace({
        // The two address rungs are mutually exclusive on the wire; a promoted
        // pick's ElementId is the stronger one, so it wins where present.
        pickedFaces: picks.map((p) => ({
          bodyId: p.bodyId,
          elementId: p.elementId,
          topoKey: p.elementId ? undefined : p.topoKey,
        })),
        chainTangentFaces,
        distanceType,
      });
    } catch (e) {
      if (gen !== this.armGen) return false;
      viewportStore.getState().setStatusHint(`Offset face unavailable: ${errMessage(e)}`, {
        severity: "error",
        sticky: true,
      });
      return false;
    }
    if (gen !== this.armGen) return false;
    if (res.refusal) {
      viewportStore.getState().setStatusHint(res.refusal.message, {
        severity: "error",
        sticky: true,
      });
      return false;
    }
    if (res.faces.length === 0) {
      viewportStore.getState().setStatusHint("Offset face: that selection resolved to no faces", {
        severity: "error",
        sticky: true,
      });
      return false;
    }
    // The worker answers in its own `body_<uuid>` wire form; the selection holds a
    // bare id. Compared through `bareBodyId` so the two forms of the SAME body
    // agree — and a genuine disagreement still refuses rather than binding the
    // closure to a body the user did not pick.
    if (res.targetBodyId && bareBodyId(res.targetBodyId) !== bareBodyId(bodyId)) {
      viewportStore.getState().setStatusHint("Offset face: the closure resolved to another body", {
        severity: "error",
        sticky: true,
      });
      return false;
    }
    const faces: SemanticRef[] = [];
    const keys: string[] = [];
    for (const ev of res.faces) {
      const ref = await this.promoteOffsetEvidence(gen, bodyId, ev);
      if (gen !== this.armGen) return false;
      if (!ref) return false; // `promoteOne` published the stale-pick hint
      faces.push(ref);
      keys.push(ev.topoKey);
    }
    let opposite: SemanticRef | undefined;
    if (res.oppositeFace) {
      const ref = await this.promoteOffsetEvidence(gen, bodyId, res.oppositeFace);
      if (gen !== this.armGen) return false;
      if (!ref) return false;
      opposite = ref;
    }
    if (distanceType === "Total" && !opposite) {
      viewportStore
        .getState()
        .setStatusHint("Offset face: no unique opposite face for a total thickness", {
          severity: "error",
          sticky: true,
        });
      return false;
    }
    this.offsetFaces = faces;
    this.offsetTopoKeys = keys;
    this.offsetOppositeFace = opposite;
    this.offsetTargetBodyId = bodyId;
    this.offsetDims = res.currentDims;
    this.offsetPreparedRevision = documentStore.getState().revision;
    this.resolveOffsetFrames();
    return true;
  }

  /**
   * One evidence entry → the typed `SemanticRef` the record holds.
   *
   * `bodyId` is the SELECTION's (bare) form, never the promotion's: `promoteOne`
   * answers in the worker's `body_<uuid>` wire form, and the preview builder
   * compares each face's body against `targetBodyId` for equality — mixing the two
   * spellings would fail that guard for a perfectly good closure.
   *
   * An already-promoted pick short-circuits: a face the user picked may already
   * carry an ElementId, and re-promoting it is a round trip for the same answer.
   */
  private async promoteOffsetEvidence(
    gen: number,
    bodyId: string,
    ev: OffsetFaceEvidence,
  ): Promise<SemanticRef | null> {
    const worldPoint =
      (ev.anchor?.worldPoint as Vec3 | undefined) ??
      this.offsetPicks.find((p) => p.topoKey === ev.topoKey)?.anchor?.worldPoint;
    const anchor = worldPoint ? { worldPoint } : undefined;
    const known = this.offsetPicks.find((p) => p.topoKey === ev.topoKey && p.elementId);
    if (known?.elementId) {
      return { primary: { bodyId, elementId: known.elementId, kind: "face" }, anchor };
    }
    const promoted = await promoteOne(this.client, bodyId, {
      topoKey: ev.topoKey,
      anchor,
    });
    if (gen !== this.armGen) return null;
    // A REFUSED promotion is fatal here, unlike the hole seat's degraded path: an
    // OffsetFace record stores typed refs whose `primary.elementId` must equal its
    // own `faceIds[i]` (core validates the lockstep), so there is no anchor-only
    // form of this op to fall back to.
    if (!promoted?.elementId) return null;
    return { primary: { bodyId, elementId: promoted.elementId, kind: "face" }, anchor };
  }

  /**
   * Resolve the closure's planar frames off the LOCAL mesh — the drag axis, the
   * chip/arrow anchor and the L1 ghost slices, all from one pass.
   *
   * DEGRADES rather than guesses. A cylindrical face has no planar frame at all
   * (`faceFrame` refuses it), and a set whose normals diverge has no honest mean —
   * either way `offsetDegraded` goes true and the tool falls back to the
   * screen-space value drag with no arrow and no ghost. Planarity here is a HINT
   * for the interaction only; the kernel remains the authority on what the offset
   * actually does.
   */
  private resolveOffsetFrames(): void {
    this.offsetFrames = [];
    this.offsetRanges = [];
    this.offsetAxis = null;
    this.offsetDegraded = true;
    const entry = getEntry(this.offsetTargetBodyId);
    const frames: PlanarFaceFrame[] = [];
    const ranges: { start: number; count: number }[] = [];
    if (entry) {
      for (const topoKey of this.offsetTopoKeys) {
        const ordinal = entry.faceIndex.ordinalForId(topoKey);
        if (ordinal < 0) break;
        const frame = faceFrame(entry.view, ordinal);
        if (!frame) break; // curved (or degenerate) — no planar frame to offset along
        frames.push(frame);
        ranges.push(faceDrawRange(entry.view.faceRanges, ordinal));
      }
    }
    // Anchor first: it is worth having even when the axis is not (the chip has to
    // hang somewhere), so it falls back to the first pick's own anchor point.
    this.offsetAnchor =
      (frames.length === this.offsetTopoKeys.length ? offsetAnchorFor(frames) : null) ??
      (this.offsetPicks[0]?.anchor?.worldPoint as Vec3 | undefined) ??
      [0, 0, 0];
    if (frames.length !== this.offsetTopoKeys.length) return; // partial ⇒ degraded
    const axis = offsetAxisFor(frames);
    if (!axis) return;
    this.offsetFrames = frames;
    this.offsetRanges = ranges;
    this.offsetAxis = axis;
    this.offsetDegraded = false;
  }

  /** Which distance types this closure may offer (the chip's segment group). */
  private offsetAllowedTypes(): readonly OffsetDistanceType[] {
    // SCHEMA §7.3: only `Offset` admits a multi-face set that is not a coaxial
    // cylindrical closure, and the frontend never authors the coaxial case.
    if (this.offsetFaces.length !== 1) return ["Offset"];
    // Planarity drives the rest. A face with a planar frame can measure a Total
    // thickness against an opposite; a curved one is the Radius/Diameter case.
    // Both lists still travel through prepare, whose refusals are authoritative.
    return this.offsetFrames.length === 1 ? OFFSET_TYPES_PLANAR : OFFSET_TYPES_CURVED;
  }

  /** (Re)publish the armed offset cluster with the FSM's current numbers. */
  private showOffsetFaceChip(): void {
    const s = this.offsetFace;
    toolChipStore.getState().showOffsetFace(
      s.distance,
      this.offsetAnchor,
      {
        onValue: (v) => this.onOffsetFaceChip(v),
        onDistanceType: (t) => this.onOffsetDistanceType(t),
        onChainTangent: (c) => this.onOffsetChainTangent(c),
        onConfirm: () => void this.commitOffsetFace(),
        onCancel: () => toolStore.getState().setTool("select"),
      },
      {
        distanceType: s.distanceType,
        chainTangentFaces: s.chainTangentFaces,
        distanceTypes: this.offsetEditFeatureId ? [s.distanceType] : this.offsetAllowedTypes(),
        valueError: s.valueError,
      },
    );
  }

  /**
   * Redraw the arrow + the L1 ghost for the FSM's current distance.
   *
   * The ghost is one translucent clone of EACH operative face, translated along
   * THAT face's own normal — not one clone of the body along the mean, which for a
   * tangent chain would show a rigid translation instead of an offset. It is
   * dropped as soon as an exact kernel candidate lands (`offsetGhostHidden`), and
   * comes back if that candidate later fails.
   */
  private applyOffsetFaceState(): void {
    if (this.offsetDegraded || !this.offsetAxis) return;
    const d = this.offsetFace.distance;
    const axis = this.offsetAxis;
    const origin: Vec3 = [
      this.offsetAnchor[0] + axis[0] * d,
      this.offsetAnchor[1] + axis[1] * d,
      this.offsetAnchor[2] + axis[2] * d,
    ];
    this.engine.showValueHandle(origin, axis);
    if (this.offsetGhostHidden) return;
    const entry = getEntry(this.offsetTargetBodyId);
    if (!entry) return;
    const offsets = ghostOffsets(this.offsetFrames, d);
    if (offsets.length !== this.offsetRanges.length) return;
    const items: GhostInstances[] = offsets.map((offset, i) => ({
      entry,
      range: this.offsetRanges[i],
      transforms: [{ kind: "translate", offset } as GhostTransform],
    }));
    this.engine.showGhostPreviewMulti(items);
  }

  /** Open the ONE kernel-preview session an armed offset drives. */
  private async openOffsetFacePreview(gen: number): Promise<void> {
    let params: PreviewParams;
    try {
      params = this.offsetFacePreviewParams();
    } catch {
      return; // not yet a complete offset — nothing honest to preview
    }
    const draft: PreviewDraft = { opType: "OffsetFace", inputs: [...this.offsetFaces], params };
    let session: PreviewSession;
    try {
      session = await this.deps.client.beginPreview(draft);
    } catch (error) {
      if (gen !== this.armGen) return;
      traceWarn("offsetFace", `OffsetFace preview session failed: ${errMessage(error)}`);
      return;
    }
    if (gen !== this.armGen) {
      void this.deps.client.endPreview(session.sessionId, false);
      return;
    }
    this.previewSessions = [{ session, draft, lastAppliedEpoch: 0, previewBodyIds: [], replacedBodyIds: [] }];
    this.previewOwner = "offsetFace";
    this.previewParamsFn = () => this.offsetFacePreviewParams();
    this.previewPending = false;
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    this.throttle.reset();
    this.throttle.setTrailingMs(OFFSET_FACE_TRAILING_MS);
    this.sendPreview();
    this.updateDebug();
  }

  /** Complete canonical OffsetFace params for both exact preview and commit. */
  private offsetFaceParams(distance = this.offsetFace.distance): OffsetFaceParams {
    const s = this.offsetFace;
    const params: OffsetFaceParams = {
      faces: [...this.offsetFaces],
      distance,
      distanceType: s.distanceType,
      chainTangentFaces: s.chainTangentFaces,
      targetBodyId: this.offsetTargetBodyId,
    };
    // Total-ONLY, gated on the type rather than on presence: a stale opposite left
    // behind by a type switch must never ride the record (core rejects it).
    if (s.distanceType === "Total" && this.offsetOppositeFace) {
      params.oppositeFace = this.offsetOppositeFace;
    }
    return params;
  }

  private offsetFacePreviewParams(distance = this.offsetFace.distance): PreviewParams {
    return { ...this.offsetFaceParams(distance) };
  }

  private onOffsetFaceChip(v: number): void {
    const step = offsetFaceStep(this.offsetFace, { kind: "setDistance", distance: v });
    this.offsetFace = step.state;
    // The chip re-renders the STATE's value, not the typed one: a refused entry is
    // never written back, so the field must show the last value that was accepted.
    toolChipStore.getState().setValue(this.offsetFace.distance);
    toolChipStore.getState().setValueError(this.offsetFace.valueError);
    if (step.effect !== "update") return;
    this.applyOffsetFaceState();
    this.sendPreview();
    this.updateDebug();
  }

  /** A distance-type segment pick. Re-runs the handshake: the closure depends on it. */
  private onOffsetDistanceType(t: OffsetDistanceType): void {
    const step = offsetFaceStep(this.offsetFace, { kind: "setDistanceType", distanceType: t });
    if (step.effect !== "begin") return;
    this.offsetFace = step.state;
    toolChipStore.getState().setDistanceType(step.state.distanceType);
    toolChipStore.getState().setChainTangent(step.state.chainTangentFaces);
    void this.armOffsetFace(step.state.distanceType, step.state.chainTangentFaces);
  }

  /** The tangent toggle. Also a re-arm — a wider/narrower closure is a new op. */
  private onOffsetChainTangent(chain: boolean): void {
    const step = offsetFaceStep(this.offsetFace, {
      kind: "setChainTangent",
      chainTangentFaces: chain,
    });
    if (step.effect !== "begin") return;
    this.offsetFace = step.state;
    toolChipStore.getState().setChainTangent(step.state.chainTangentFaces);
    void this.armOffsetFace(step.state.distanceType, step.state.chainTangentFaces);
  }

  /**
   * Apply the armed offset. The RE-EDIT path is a distance-only scalar merge (no
   * lane session, no handshake — the stored record already owns its closure); a
   * FRESH offset runs the previewed-commit sequence under OP-SPECIFIC STRICTNESS.
   *
   * Three preconditions the generic barrier does not give us, and why each is here:
   *  1. a successful handshake for the CURRENT document revision — the frozen
   *     TopoKeys were resolved against that head, and a moved head can silently
   *     re-point them;
   *  2. no outstanding preview failure — the ✓ is blocked while the newest
   *     candidate is a refusal (the house rule);
   *  3. an exact candidate that ACTUALLY LANDED for the final params. The generic
   *     barrier proceeds on timeout by design; here that would commit a frozen
   *     closure the kernel never evaluated.
   */
  private async commitOffsetFace(): Promise<void> {
    const editFeatureId = this.offsetEditFeatureId;
    if (editFeatureId) {
      await this.commitOffsetFaceEdit(editFeatureId);
      return;
    }
    if (this.offsetFaces.length === 0) {
      this.cancelOffsetFace();
      toolStore.getState().setTool("select");
      return;
    }
    if (this.offsetFace.phase !== "armed") return;
    if (
      this.offsetPreparedRevision === null ||
      this.offsetPreparedRevision !== documentStore.getState().revision
    ) {
      viewportStore
        .getState()
        .setStatusHint("Offset face: the model changed — re-select the faces", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (this.previewFailure) {
      viewportStore
        .getState()
        .setStatusHint(`Cannot apply invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const step = offsetFaceStep(this.offsetFace, { kind: "confirm" });
    if (step.effect !== "commit") {
      // The reducer refused a degenerate magnitude. Nothing is clamped — say what
      // the domain is and leave the number exactly as the user left it.
      this.offsetFace = step.state;
      toolChipStore.getState().setValueError(true);
      viewportStore.getState().setStatusHint(
        this.offsetFace.distanceType === "Offset"
          ? "Offset face: that distance is too small to change anything"
          : `Offset face: a ${this.offsetFace.distanceType} must be greater than zero`,
        { severity: "error", sticky: true },
      );
      return;
    }
    this.offsetFace = step.state; // → committing (excludes us from the stale-arm guard)
    toolStore.setState({ phase: "committing" });
    const gen = ++this.commitGen;
    const params = this.offsetFaceParams();
    const outcome = await this.commitPreviewedOp(
      { opType: "OffsetFace", inputs: [...this.offsetFaces], params },
      gen,
      { requireExactPreview: true },
    );
    if (outcome.kind === "superseded") return;
    if (outcome.kind === "failed") {
      this.offsetFace = offsetFaceStep(this.offsetFace, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`Offset face failed: ${outcome.reason}`, { severity: "error", sticky: true });
      this.offsetGhostHidden = false;
      this.applyOffsetFaceState();
      await this.openOffsetFacePreview(this.armGen); // re-arm the preview (work kept)
      this.updateDebug();
      return;
    }
    this.applyResult(outcome.res);
    this.teardownPreviewedTool();
    this.engine.hideValueHandle();
    this.engine.hideGhostPreview();
    this.resetOffsetFaceState();
    this.resetToSelect("Face offset");
    this.updateDebug();
  }

  /** Distance-only parametric re-edit — deep-merges into the stored params. */
  private async commitOffsetFaceEdit(editFeatureId: string): Promise<void> {
    const distance = this.offsetFace.distance;
    if (!offsetCanConfirm(this.offsetFace)) {
      toolChipStore.getState().setValueError(true);
      viewportStore.getState().setStatusHint("Offset face: that distance is out of range", {
        severity: "error",
        sticky: true,
      });
      return;
    }
    // Move to `committing` so the revision bump this commit causes does not read
    // as an external model change and cancel our own arm.
    const step = offsetFaceStep(this.offsetFace, { kind: "confirm" });
    if (step.effect === "commit") this.offsetFace = step.state;
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    let failure: string | null = null;
    try {
      // A re-edit changes ONLY the distance: deep-merge into the stored params so
      // the frozen closure, the opposite face and the target survive verbatim (a
      // whole-params replace would wipe every one of them).
      if (!this.offsetStoredParams) {
        throw new Error("Stored Offset face parameters are unavailable");
      }
      const res = await this.client.applyEditCommand(
        updateScalarParamsCommand(editFeatureId, "OffsetFace", this.offsetStoredParams, {
          distance: { value: distance },
        }),
      );
      this.applyResult(res);
    } catch (e) {
      failure = errMessage(e);
    }
    this.resetOffsetFaceState();
    if (failure !== null) {
      this.resetToSelect(`Offset face failed: ${failure}`, { severity: "error", sticky: true });
    } else {
      this.resetToSelect("Offset distance updated");
    }
    this.updateDebug();
  }

  /**
   * Re-arm the offset tool on an existing OffsetFace feature (distance re-edit).
   *
   * L1-ONLY, like every other re-edit here: `PreviewOp` runs against the CURRENT
   * head, so previewing an existing feature would double-apply it. It also runs NO
   * handshake — the record's closure is already frozen and re-preparing would
   * compute a new one against today's geometry, quietly replacing the operative
   * set the user authored.
   */
  async editOffsetFaceFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    // Gate on `opType`, NEVER `kind`: `dto.rs feature_kind` folds OffsetFace into
    // the `fillet` bucket, so a `kind` guard would be unsatisfiable on the real lane.
    if (!feat || feat.opType !== "OffsetFace") return;
    const stored = await this.deps.client.getOperationParams(featureId).catch(() => undefined);
    const distance = feat.primaryValue ?? distanceFromValueText(feat.valueText);
    const storedType = stored?.distanceType;
    const distanceType: OffsetDistanceType =
      storedType === "Total" || storedType === "Radius" || storedType === "Diameter"
        ? storedType
        : "Offset";
    toolStore.getState().setTool("offsetFace"); // fires cancelOffsetFace
    this.offsetStoredParams = stored; // set AFTER the tool-change cancel
    this.offsetEditFeatureId = featureId;
    const gen = ++this.armGen;
    // `faceCount: 1` keeps the FSM out of its bail path — a re-edit has no fresh
    // picks (the fillet/shell re-edit seed rule).
    this.offsetFace = offsetFaceStep(offsetFaceInit(), {
      kind: "arm",
      faceCount: 1,
      distance,
      distanceType,
      chainTangentFaces: stored?.chainTangentFaces !== false,
      touched: true,
    }).state;
    if (gen !== this.armGen) return;
    toolStore.setState({ phase: "armed" });
    this.offsetAnchor = [0, 0, 0];
    this.showOffsetFaceChip();
    viewportStore
      .getState()
      .setStatusHint("Edit offset distance — type a value, Enter to apply", { sticky: true });
    this.updateDebug();
  }

  /** Every field the offset lane owns, back to its idle value. */
  private resetOffsetFaceState(): void {
    this.offsetFace = offsetFaceInit();
    this.offsetPicks = [];
    this.offsetFaces = [];
    this.offsetTopoKeys = [];
    this.offsetOppositeFace = undefined;
    this.offsetTargetBodyId = "";
    this.offsetFrames = [];
    this.offsetRanges = [];
    this.offsetAxis = null;
    this.offsetAnchor = [0, 0, 0];
    this.offsetDims = {};
    this.offsetPreparedRevision = null;
    this.offsetDegraded = false;
    this.offsetGhostHidden = false;
    this.offsetEditFeatureId = undefined;
    this.offsetStoredParams = undefined;
  }

  private cancelOffsetFace(): void {
    // Release any open offset lane session FIRST (see cancelFillet).
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.deps.engine.setOrbitSuppressed(false);
    // R3: the drag arrow is the ONE shared `DragHandle`, so a cancel that left it
    // visible would hand the next tool a floating arrow it never asked for.
    this.engine.hideValueHandle();
    this.engine.hideGhostPreview();
    this.resetOffsetFaceState();
    if (this.dragging === "offsetFace") this.dragging = null;
    toolChipStore.getState().clear();
    this.updateDebug(); // republish the now-idle phase (a tool switch has no other hook)
  }

  // ── hole (WP-C T3) ─────────────────────────────────────────────────────────
  //
  // Two-phase gesture, like the datum tool: activate → click a PLANAR face →
  // armed with a live kernel preview at the clicked point, chips for the profile
  // and its dimensions, ✓ / Enter commits. A further click on a face MOVES the
  // hole instead of re-arming (the seat is the expensive choice, the position is
  // cheap), so a mis-placed hole is one click from fixed rather than a cancel.
  //
  // Planarity is validated BEFORE the tool arms (the sketch-on-face precedent):
  // the backend would refuse a curved seat with `OP_FAILED`, and refusing it here
  // turns a failed history row into an inline hint. The check runs on the LOCAL
  // mesh (`faceFrame`) — no round-trip on a pointer click.

  private startHole(): void {
    this.hole = holeStep(this.hole, { kind: "start" }).state;
    // Modal for BOTH phases: the first click places the seat and every later one
    // MOVES it, so an LMB drag must never become an orbit that eats the click.
    this.deps.engine.setOrbitSuppressed(true);
    viewportStore
      .getState()
      .setStatusHint("Click a flat face to place the hole · Esc cancels", { sticky: true });
    this.updateDebug();
  }

  /** The picked face's planar frame, or `null` when the face is not flat. */
  private holeFaceFrame(bodyId: string, topoKey: string): PlanarFaceFrame | null {
    const entry = getEntry(bodyId);
    if (!entry) return null;
    const ordinal = entry.faceIndex.ordinalForId(topoKey);
    if (ordinal < 0) return null;
    return faceFrame(entry.view, ordinal);
  }

  /**
   * A face click while the hole tool is picking (or re-positioning). Promotes the
   * TopoKey to a Rust-minted ElementId first, exactly like `tryPickExtrudeFace`:
   * the seat must survive a parametric edit, and only a minted id carries the
   * identity the resolution ladder rebinds (SCHEMA §7.3 / §10).
   */
  private async tryPickHoleFace(clientX: number, clientY: number): Promise<void> {
    if (this.hole.phase !== "facePick" && this.hole.phase !== "armed") return;
    const hit = this.engine.probePick(clientX, clientY);
    if (!hit || hit.kind !== "face" || hit.bodyId.startsWith("preview:")) {
      viewportStore.getState().setStatusHint("Hole: click a flat FACE to place the hole", {
        severity: "error",
        sticky: true,
      });
      return;
    }
    if (!this.holeFaceFrame(hit.bodyId, hit.topoKey)) {
      // Refused HERE rather than by the kernel: a curved seat has no axis, and an
      // errored history row is a worse way to learn that than a hint.
      viewportStore.getState().setStatusHint("Hole: that face is not flat — a hole needs a flat seat", {
        severity: "error",
        sticky: true,
      });
      return;
    }
    const point: [number, number, number] = [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z];

    // Re-positioning an ALREADY armed hole on the SAME face: no promotion, no new
    // session — just newer params on the live one.
    const armedFace = this.hole.face as SemanticRef | null;
    if (this.hole.phase === "armed" && armedFace?.primary.bodyId === hit.bodyId) {
      const sameFace = hit.elementId
        ? armedFace.primary.elementId === hit.elementId
        : this.holeTopoKey === hit.topoKey;
      if (sameFace) {
        this.hole = holeStep(this.hole, { kind: "movePoint", point }).state;
        this.showHoleChip(); // republish at the new anchor
        this.sendPreview();
        this.updateDebug();
        return;
      }
    }

    const gen = ++this.armGen;
    let elementId = hit.elementId;
    if (!elementId && hit.topoKey) {
      const promoted = await promoteOne(this.client, hit.bodyId, {
        topoKey: hit.topoKey,
        anchor: { worldPoint: point },
      });
      if (gen !== this.armGen) return; // re-armed while awaiting — drop
      // A stale pick must stay in face-pick mode. The Hole seat cannot degrade
      // to an anchor-only ref without making a later rebind author a new face.
      if (!promoted?.elementId) return;
      elementId = promoted.elementId;
    }
    if (toolStore.getState().modelTool !== "hole") return;
    const face: SemanticRef = {
      primary: { bodyId: hit.bodyId, elementId, kind: "face" },
      anchor: { worldPoint: point },
    };
    this.holeTopoKey = hit.topoKey;
    this.hole = holeStep(this.hole, {
      kind: "pickFace",
      face,
      targetBodyId: hit.bodyId,
      point,
    }).state;
    this.showHoleChip();
    viewportStore.getState().setStatusHint(HOLE_ARMED_HINT, { sticky: true });
    this.previewArmHint = this.holeEditFeatureId ? null : HOLE_ARMED_HINT;
    this.updateDebug();
    // A re-edit runs L1-only: PreviewOp executes against the CURRENT head, so
    // previewing an existing feature would double-apply it (the extrude re-edit rule).
    if (!this.holeEditFeatureId) await this.openHolePreview(gen);
  }

  /** (Re)publish the armed hole cluster with the FSM's current numbers. */
  private showHoleChip(): void {
    const anchor = this.hole.point ?? [0, 0, 0];
    toolChipStore.getState().showHole(
      this.hole.diameter,
      anchor,
      {
        onValue: (v) => this.onHoleEvent({ kind: "setDiameter", diameter: v }),
        onHoleType: (holeType) => this.onHoleEvent({ kind: "setHoleType", holeType }),
        onDepth: (depth) => this.onHoleEvent({ kind: "setDepth", depth }),
        onCbDiameter: (v) => this.onHoleEvent({ kind: "setCbDiameter", value: v }),
        onCbDepth: (v) => this.onHoleEvent({ kind: "setCbDepth", value: v }),
        onCsDiameter: (v) => this.onHoleEvent({ kind: "setCsDiameter", value: v }),
        onCsAngle: (deg) => this.onHoleEvent({ kind: "setCsAngle", angleDeg: deg }),
        onStandard: (thread, fit) => {
          const patch = holeStandardPatch(thread, fit, this.hole.holeType);
          if (patch) this.onHoleEvent({ kind: "applyStandard", patch });
        },
        onConfirm: () => void this.commitHole(),
        onCancel: () => toolStore.getState().setTool("select"),
      },
      this.holeChipOpts(),
    );
  }

  private holeChipOpts(): HoleChipOpts {
    return {
      holeType: this.hole.holeType,
      depth: this.hole.depth,
      cbDiameter: this.hole.cbDiameter,
      cbDepth: this.hole.cbDepth,
      csDiameter: this.hole.csDiameter,
      csAngleDeg: this.hole.csAngleDeg,
    };
  }

  /**
   * ONE entry point for every chip edit: step the FSM, republish the cluster (a
   * profile flip changes WHICH fields render, so the whole cluster is reissued
   * rather than patched field by field), and push newer preview params.
   */
  private onHoleEvent(e: HoleEvent): void {
    const before = this.hole;
    this.hole = holeStep(this.hole, e).state;
    if (this.hole === before) return;
    this.showHoleChip();
    this.sendPreview();
    this.updateDebug();
  }

  private async openHolePreview(gen: number): Promise<void> {
    // Re-seating an already-armed hole opens a NEW session; the old one must be
    // released first or it leaks a scratch job in the worker (and its stale
    // candidate would keep publishing meshes at the previous seat).
    this.closePreviewSessions();
    let params: PreviewParams;
    try {
      params = this.holePreviewParams();
    } catch {
      return; // not yet a complete hole — nothing honest to preview
    }
    const draft: PreviewDraft = {
      opType: "Hole",
      inputs: this.holeInputs(),
      params,
    };
    let session: PreviewSession;
    try {
      session = await this.deps.client.beginPreview(draft);
    } catch (error) {
      if (gen !== this.armGen) return;
      traceWarn("hole", `Hole preview session failed: ${errMessage(error)}`);
      return;
    }
    if (gen !== this.armGen) {
      void this.deps.client.endPreview(session.sessionId, false);
      return;
    }
    this.previewSessions = [{ session, draft, lastAppliedEpoch: 0, previewBodyIds: [], replacedBodyIds: [] }];
    this.previewOwner = "hole";
    this.previewParamsFn = () => this.holePreviewParams();
    this.previewPending = false;
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    this.throttle.reset();
    this.throttle.setTrailingMs(SHELL_TRAILING_MS);
    this.sendPreview();
    this.updateDebug();
  }

  /**
   * The op's semantic refs — host body then host face, mirroring SCHEMA §7.3's
   * `inputs: [semanticRef(host body), semanticRef(host face)]`. (Rust rebuilds
   * these from `params` for the wire; this is the graph-visible echo the preview
   * draft and the commit op both carry, in lockstep.)
   */
  private holeInputs(): SemanticRef[] {
    const face = this.hole.face as SemanticRef | null;
    if (!face) return [];
    return [{ primary: { bodyId: this.hole.targetBodyId, kind: "body" } }, face];
  }

  /** Complete canonical Hole params for both exact preview and commit. */
  private holeParams(): HoleParams {
    return holeParamsOf(this.hole);
  }

  private holePreviewParams(): PreviewParams {
    return { ...this.holeParams() };
  }

  /**
   * Apply the armed hole. A RE-EDIT is a whole-params replacement (unlike Shell's
   * scalar merge): the conditional `cb*`/`cs*` blocks mean a patch cannot express
   * "this is no longer a counterbore" — `updateScalarParamsCommand` spreads, and a
   * spread can never DELETE a key, so a counterbore→simple edit would leave the
   * `cb*` behind and the Rust session would reject the record. The FSM already
   * holds every field, so replacing wholesale is both correct and simpler.
   */
  private async commitHole(): Promise<void> {
    if (this.hole.phase !== "armed") return;
    if (this.previewFailure) {
      viewportStore
        .getState()
        .setStatusHint(`Cannot apply invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const step = holeStep(this.hole, { kind: "confirm" });
    if (step.effect !== "commit") return;
    this.hole = step.state; // → committing
    toolStore.setState({ phase: "committing" });
    const editFeatureId = this.holeEditFeatureId;
    const gen = ++this.commitGen;
    const op: OperationOp = {
      opType: "Hole",
      featureId: editFeatureId,
      inputs: this.holeInputs(),
      params: this.holeParams(),
    };

    // A re-edit has no lane session (it would double-apply the existing feature),
    // so it takes the plain applyOperation path.
    if (editFeatureId) {
      let failure: string | null = null;
      try {
        this.applyResult(await this.client.applyOperation(op));
      } catch (e) {
        failure = errMessage(e);
      }
      this.finishHole(failure === null ? "Hole updated" : `Hole failed: ${failure}`, failure !== null);
      return;
    }

    const outcome = await this.commitPreviewedOp(op, gen);
    if (outcome.kind === "superseded") return;
    if (outcome.kind === "failed") {
      this.hole = holeStep(this.hole, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`Hole failed: ${outcome.reason}`, { severity: "error", sticky: true });
      await this.openHolePreview(this.armGen); // re-arm the preview (work kept)
      this.updateDebug();
      return;
    }
    this.applyResult(outcome.res);
    this.teardownPreviewedTool();
    this.finishHole("Hole", false);
  }

  private finishHole(hint: string, failed: boolean): void {
    this.hole = holeInit();
    this.holeEditFeatureId = undefined;
    this.holeTopoKey = undefined;
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    this.resetToSelect(hint, failed ? { severity: "error", sticky: true } : undefined);
    this.updateDebug();
  }

  /**
   * Re-arm the hole tool on an existing Hole feature. Unlike the value-only
   * re-edits, EVERY field is seeded from the stored params — including the frozen
   * seat (`face`/`point`), which the user never has to re-pick.
   */
  async editHoleFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    // Gate on `opType`, NEVER `kind`: `dto.rs feature_kind` buckets Hole under
    // `boolean`, so a `kind === "hole"` guard is unsatisfiable on the Tauri lane.
    if (!feat || feat.opType !== "Hole") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return; // superseded while in flight
    const seeded = holeFsmFromParams(stored);
    if (!seeded) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit this hole: its stored parameters are unavailable", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    toolStore.getState().setTool("hole"); // fires cancelHole (clears the seed fields)
    this.armGen++; // the async startHole path must not clobber the seed
    this.holeEditFeatureId = featureId; // set AFTER the tool-change cancel
    this.hole = seeded;
    // `setTool` above is a no-op when the hole tool is ALREADY active, so the
    // `startHole` suppression cannot be relied on here — re-assert it.
    this.deps.engine.setOrbitSuppressed(true);
    this.showHoleChip();
    viewportStore.getState().setStatusHint(HOLE_ARMED_HINT, { sticky: true });
    this.updateDebug();
  }

  private cancelHole(): void {
    // Release any open hole lane session FIRST (see cancelFillet).
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    this.hole = holeInit();
    this.holeEditFeatureId = undefined;
    this.holeTopoKey = undefined;
    toolChipStore.getState().clear();
    this.updateDebug(); // republish the now-idle phase (a tool switch has no other hook)
  }

  // ── linear pattern ───────────────────────────────────────────────────────
  //
  // Chip-driven: axis (X/Y/Z) + count (2–12) + spacing (mm) + Apply. A live ghost
  // of translated body clones renders as any chip changes. Orbit stays free so the
  // 3D ghost can be inspected; there is no drag-to-commit (Apply commits).

  private armLinearFromSelection(): void {
    const bodyId = this.firstSelectedBodyId();
    if (!bodyId) {
      const verdict = getToolApplicability("linearPattern", selectionStore.getState().selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    this.armLinear(bodyId);
  }

  private armLinear(
    bodyId: string,
    editFeatureId?: string,
    seedCount?: number,
    seedAxis?: PatternAxis,
    seedSpacing?: number,
    resultPolicyVersion?: 2,
    fuseResult?: boolean,
  ): void {
    this.patternEditFeatureId = editFeatureId;
    this.patternResultPolicyVersion = editFeatureId ? resultPolicyVersion : 2;
    this.patternFuseResult = editFeatureId ? fuseResult ?? true : false;
    this.linear = linearPatternStep(linearPatternInit(), {
      kind: "arm",
      bodyId,
      count: seedCount,
      axis: seedAxis,
      spacing: seedSpacing,
    }).state;
    toolStore.setState({ phase: "armed" });
    viewportStore.getState().setStatusHint("Pick axis + count + spacing, then Apply", { sticky: true });
    this.rebuildLinearGhost();
    toolChipStore.getState().showLinearPattern(this.linear.axis, this.linear.count, this.linear.spacing, this.bodyCenter(bodyId), {
      onAxis: (a) => this.onLinearAxis(a),
      onCount: (n) => this.onLinearCount(n),
      onSpacing: (v) => this.onLinearSpacing(v),
      onApply: () => void this.commitLinear(),
    });
    this.updateDebug();
  }

  private onLinearAxis(axis: PatternAxis): void {
    this.linear = linearPatternStep(this.linear, { kind: "setAxis", axis }).state;
    toolChipStore.getState().setAxis(this.linear.axis);
    this.rebuildLinearGhost();
  }
  private onLinearCount(count: number): void {
    this.linear = linearPatternStep(this.linear, { kind: "setCount", count: clampPatternCount(count) }).state;
    toolChipStore.getState().setCount(this.linear.count);
    this.rebuildLinearGhost();
  }
  private onLinearSpacing(spacing: number): void {
    this.linear = linearPatternStep(this.linear, { kind: "setSpacing", spacing }).state;
    toolChipStore.getState().setValue(this.linear.spacing);
    this.rebuildLinearGhost();
  }

  private rebuildLinearGhost(): void {
    if (!this.linear.bodyId) return;
    const entry = getEntry(this.linear.bodyId);
    if (!entry) return;
    this.deps.engine.showGhostPreview(
      entry,
      linearGhostTransforms(WORLD_AXIS[this.linear.axis], this.linear.spacing, this.linear.count),
    );
  }

  private async commitLinear(): Promise<void> {
    if (this.linear.phase !== "armed" || !this.linear.bodyId) return;
    const { bodyId, axis, spacing, count } = this.linear;
    const editFeatureId = this.patternEditFeatureId;
    this.linear = linearPatternStep(this.linear, { kind: "apply" }).state;
    const op: OperationOp = {
      opType: "LinearPattern",
      featureId: editFeatureId,
      inputs: [{ primary: { bodyId, kind: "body" } }],
      params: {
        sourceBodyId: bodyId,
        direction: WORLD_AXIS[axis],
        spacing,
        count,
        fuseResult: this.patternFuseResult,
        ...(this.patternResultPolicyVersion === undefined
          ? {}
          : { resultPolicyVersion: this.patternResultPolicyVersion }),
      },
    };
    await this.commitPattern(op, bodyId, `Linear pattern ×${count}`);
  }

  // ── circular pattern ─────────────────────────────────────────────────────

  private armCircularFromSelection(): void {
    const bodyId = this.firstSelectedBodyId();
    if (!bodyId) {
      const verdict = getToolApplicability("circularPattern", selectionStore.getState().selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    this.armCircular(bodyId);
  }

  private armCircular(
    bodyId: string,
    editFeatureId?: string,
    seedCount?: number,
    seedAxis?: PatternAxis,
    seedAngle?: number,
    resultPolicyVersion?: 2,
    fuseResult?: boolean,
  ): void {
    this.patternEditFeatureId = editFeatureId;
    this.patternResultPolicyVersion = editFeatureId ? resultPolicyVersion : 2;
    this.patternFuseResult = editFeatureId ? fuseResult ?? true : false;
    this.circular = circularPatternStep(circularPatternInit(), {
      kind: "arm",
      bodyId,
      count: seedCount,
      axis: seedAxis,
      angle: seedAngle,
    }).state;
    toolStore.setState({ phase: "armed" });
    viewportStore.getState().setStatusHint("Pick axis + count + angle, then Apply", { sticky: true });
    this.rebuildCircularGhost();
    toolChipStore.getState().showCircularPattern(this.circular.axis, this.circular.count, this.circular.angle, this.bodyCenter(bodyId), {
      onAxis: (a) => this.onCircularAxis(a),
      onCount: (n) => this.onCircularCount(n),
      onAngle: (v) => this.onCircularAngle(v),
      onApply: () => void this.commitCircular(),
    });
    this.updateDebug();
  }

  private onCircularAxis(axis: PatternAxis): void {
    this.circular = circularPatternStep(this.circular, { kind: "setAxis", axis }).state;
    toolChipStore.getState().setAxis(this.circular.axis);
    this.rebuildCircularGhost();
  }
  private onCircularCount(count: number): void {
    this.circular = circularPatternStep(this.circular, { kind: "setCount", count: clampPatternCount(count) }).state;
    toolChipStore.getState().setCount(this.circular.count);
    this.rebuildCircularGhost();
  }
  private onCircularAngle(angle: number): void {
    this.circular = circularPatternStep(this.circular, { kind: "setAngle", angle }).state;
    toolChipStore.getState().setValue(this.circular.angle);
    this.rebuildCircularGhost();
  }

  private rebuildCircularGhost(): void {
    if (!this.circular.bodyId) return;
    const entry = getEntry(this.circular.bodyId);
    if (!entry) return;
    this.deps.engine.showGhostPreview(
      entry,
      circularGhostTransforms([0, 0, 0], WORLD_AXIS[this.circular.axis], this.circular.angle, this.circular.count),
    );
  }

  private async commitCircular(): Promise<void> {
    if (this.circular.phase !== "armed" || !this.circular.bodyId) return;
    const { bodyId, axis, angle, count } = this.circular;
    const editFeatureId = this.patternEditFeatureId;
    this.circular = circularPatternStep(this.circular, { kind: "apply" }).state;
    const op: OperationOp = {
      opType: "CircularPattern",
      featureId: editFeatureId,
      inputs: [{ primary: { bodyId, kind: "body" } }],
      params: {
        sourceBodyId: bodyId,
        axisOrigin: [0, 0, 0],
        axisDirection: WORLD_AXIS[axis],
        angleDeg: angle,
        count,
        fuseResult: this.patternFuseResult,
        ...(this.patternResultPolicyVersion === undefined
          ? {}
          : { resultPolicyVersion: this.patternResultPolicyVersion }),
      },
    };
    await this.commitPattern(op, bodyId, `Circular pattern ×${count}`);
  }

  // ── mirror body ──────────────────────────────────────────────────────────

  private armMirrorFromSelection(): void {
    const bodyId = this.firstSelectedBodyId();
    if (!bodyId) {
      const verdict = getToolApplicability("mirror", selectionStore.getState().selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    this.armMirror(bodyId);
  }

  private armMirror(bodyId: string, editFeatureId?: string, seedPlane?: MirrorPlane): void {
    this.patternEditFeatureId = editFeatureId;
    this.mirror = mirrorStep(mirrorInit(), { kind: "arm", bodyId, plane: seedPlane }).state;
    toolStore.setState({ phase: "armed" });
    viewportStore.getState().setStatusHint("Pick a mirror plane, then Apply", { sticky: true });
    this.rebuildMirrorGhost();
    toolChipStore.getState().showMirror(this.mirror.plane, this.bodyCenter(bodyId), {
      onPlane: (p) => this.onMirrorPlane(p),
      onApply: () => void this.commitMirror(),
    });
    this.updateDebug();
  }

  private onMirrorPlane(plane: MirrorPlane): void {
    this.mirror = mirrorStep(this.mirror, { kind: "setPlane", plane }).state;
    toolChipStore.getState().setPlane(this.mirror.plane);
    this.rebuildMirrorGhost();
  }

  private rebuildMirrorGhost(): void {
    if (!this.mirror.bodyId) return;
    const entry = getEntry(this.mirror.bodyId);
    if (!entry) return;
    this.deps.engine.showGhostPreview(entry, mirrorGhostTransforms([0, 0, 0], WORLD_PLANE_NORMAL[this.mirror.plane]));
  }

  private async commitMirror(): Promise<void> {
    if (this.mirror.phase !== "armed" || !this.mirror.bodyId) return;
    const { bodyId, plane } = this.mirror;
    const editFeatureId = this.patternEditFeatureId;
    this.mirror = mirrorStep(this.mirror, { kind: "apply" }).state;
    const op: OperationOp = {
      opType: "MirrorBody",
      featureId: editFeatureId,
      inputs: [{ primary: { bodyId, kind: "body" } }],
      params: {
        sourceBodyId: bodyId,
        planePoint: [0, 0, 0],
        planeNormal: WORLD_PLANE_NORMAL[plane],
        fuseWithOriginal: false,
      },
    };
    await this.commitPattern(op, bodyId, "Mirrored");
  }

  /** Shared commit tail for the pattern/mirror ops (apply → select → teardown). */
  private async commitPattern(op: OperationOp, bodyId: string, doneHint: string): Promise<void> {
    this.deps.engine.hideGhostPreview();
    toolChipStore.getState().clear();
    // The result message is captured, not published, until the tool has been reset —
    // see `resetToSelect` for why publishing first would lose it.
    let failure: string | null = null;
    try {
      const res = await this.client.applyOperation(op);
      this.applyResult(res);
      selectionStore.getState().set([{ kind: "body", id: bodyId }]);
    } catch (e) {
      failure = errMessage(e);
    }
    this.linear = linearPatternInit();
    this.circular = circularPatternInit();
    this.mirror = mirrorInit();
    this.patternEditFeatureId = undefined;
    this.patternResultPolicyVersion = undefined;
    this.patternFuseResult = false;
    if (failure !== null) {
      this.resetToSelect(`Pattern failed: ${failure}`, { severity: "error", sticky: true });
    } else {
      this.resetToSelect(doneHint);
    }
    this.updateDebug();
  }

  /**
   * Re-arm the linear-pattern tool on an existing feature (parametric edit; mirrors
   * `beginExtrudeFeatureEdit`). The source body comes from the STORED `sourceBodyId`
   * — never a selection/first-body guess (a wrong silent bind would re-pattern the
   * wrong body) — and the FULL stored shape (axis + spacing + count) is seeded, not
   * just count, so a re-edit no longer silently resets the axis/spacing to defaults.
   * A missing/foreign source body, or a stored direction that is not one of the
   * three world axes the FSM offers, refuses with a sticky hint rather than guess.
   */
  async editLinearPatternFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    // Gate on `opType`, NEVER `kind` — see `editShellFeature`: `dto.rs
    // feature_kind` folds LinearPattern/CircularPattern/MirrorBody into `boolean`.
    if (!feat || feat.opType !== "LinearPattern") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return; // superseded while getOperationParams was in flight
    const bodyId = this.reeditSourceBodyFromParams(stored, "linear pattern");
    if (!bodyId) return;
    const axis = matchWorldAxis(stored?.direction);
    if (!axis) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit linear pattern: stored direction is not a world axis", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const spacing = scalarNumber(stored?.spacing);
    const count = typeof stored?.count === "number" ? stored.count : countFromValueText(feat.valueText);
    toolStore.getState().setTool("linearPattern");
    const resultPolicyVersion = stored?.resultPolicyVersion === 2 ? 2 : undefined;
    const fuseResult = typeof stored?.fuseResult === "boolean" ? stored.fuseResult : true;
    this.armLinear(bodyId, featureId, count, axis, spacing, resultPolicyVersion, fuseResult);
  }

  /** Circular-pattern counterpart of `editLinearPatternFeature` — same contract,
   *  seeding axis + angle + count from the stored `CircularPatternParams`. */
  async editCircularPatternFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.opType !== "CircularPattern") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return;
    const bodyId = this.reeditSourceBodyFromParams(stored, "circular pattern");
    if (!bodyId) return;
    const axis = matchWorldAxis(stored?.axisDirection);
    if (!axis) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit circular pattern: stored axis is not a world axis", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const angle = scalarNumber(stored?.angleDeg);
    const count = typeof stored?.count === "number" ? stored.count : countFromValueText(feat.valueText);
    toolStore.getState().setTool("circularPattern");
    const resultPolicyVersion = stored?.resultPolicyVersion === 2 ? 2 : undefined;
    const fuseResult = typeof stored?.fuseResult === "boolean" ? stored.fuseResult : true;
    this.armCircular(bodyId, featureId, count, axis, angle, resultPolicyVersion, fuseResult);
  }

  /** Mirror counterpart of `editLinearPatternFeature` — seeds the mirror plane from
   *  the stored `planeNormal` (matched against the three world planes the FSM offers). */
  async editMirrorFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.opType !== "MirrorBody") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return;
    const bodyId = this.reeditSourceBodyFromParams(stored, "mirror");
    if (!bodyId) return;
    const plane = matchWorldPlane(stored?.planeNormal);
    if (!plane) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit mirror: stored plane is not a world plane", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    toolStore.getState().setTool("mirror");
    this.armMirror(bodyId, featureId, plane);
  }

  /**
   * Resolve + validate the source body a pattern/mirror re-edit stored. NO fallback
   * to the current selection or "the first body" — either would silently re-target
   * a different body than the one this feature actually operated on. Mirrors
   * `beginExtrudeFeatureEdit`'s missing-profile guard.
   */
  private reeditSourceBodyFromParams(
    stored: Record<string, unknown> | undefined,
    what: string,
  ): string | null {
    const sourceBodyId = stored?.sourceBodyId;
    if (typeof sourceBodyId !== "string" || !documentStore.getState().bodies[sourceBodyId]) {
      viewportStore
        .getState()
        .setStatusHint(`Cannot re-edit ${what}: source body is missing or was deleted`, {
          severity: "error",
          sticky: true,
        });
      return null;
    }
    return sourceBodyId;
  }

  private firstSelectedBodyId(): string | null {
    return selectionStore.getState().selected.find((r) => r.kind === "body")?.id ?? null;
  }

  /** Every selected BODY, in selection order (a placement is multi-body — W1). */
  private selectedBodyIds(): string[] {
    return selectionStore
      .getState()
      .selected.filter((r) => r.kind === "body")
      .map((r) => r.id);
  }

  // ── transform body (WP-B W1; SCHEMA §7.3 TransformBody) ──────────────────────
  //
  // A PLACEMENT, not a modelling op: it changes where bodies ARE, never their
  // shape. Arms from a body selection, authored through the numeric chip cluster
  // `[Move|Rotate] [X|Y|Z] [value] [✓] [✕]` — the W1 surface. The drag gizmo is W2.
  //
  // Three things are load-bearing and easy to get subtly wrong:
  //
  // FOLD. A placement is ONE cumulative record per intent (SCHEMA §7.3), so a
  // second gesture on an already-placed body must RE-EDIT that record rather than
  // stack a second one. Whether that is safe is a LINEAGE question the frontend
  // cannot answer — it sees a flat history list, not "which record last touched
  // this body" — so `canFoldTransform` asks the backend and its answer is
  // authoritative. We add one frontend condition on top (below).
  //
  // FROZEN PIVOT. `rotate.center` is captured ONCE, at first authoring, from the
  // targets' combined bbox centre, and thereafter always read back from the stored
  // params. Re-deriving it on each edit would move the pivot as the body moves and
  // every re-edit would drift.
  //
  // NO PREVIEW ROUND-TRIP. Every other value tool opens a kernel preview session
  // because only OCCT knows what a fillet/shell/boolean produces. A rigid
  // placement is the one op where the frontend's own answer is EXACT — the same
  // matrix moves the same points — so the L1 ghost is kernel-accurate and a
  // PreviewOp lane here would cost a round trip per keystroke to reproduce a
  // matrix multiply we already have. Deliberate deviation from the house pattern.

  private armTransformFromSelection(): void {
    const targets = this.selectedBodyIds();
    if (targets.length === 0) {
      const verdict = getToolApplicability("transform", selectionStore.getState().selected, this.applicabilityCtx());
      viewportStore.getState().setStatusHint(verdict.reason ?? null, { sticky: true });
      return;
    }
    void this.armTransformResolvingFold(targets);
  }

  /**
   * Decide fold-vs-fresh, then arm. Fold requires BOTH:
   *
   *  1. the backend names a foldable record for the first target, and
   *  2. that record's stored `targets` are EXACTLY the current selection.
   *
   * (2) is the frontend's own condition and it is not redundant: the query is
   * asked per body, so a two-body selection whose first body was moved ALONE
   * would otherwise fold into that single-body record and silently start moving a
   * body it never covered. A mismatch appends a fresh record instead — the
   * conservative direction.
   */
  private async armTransformResolvingFold(targets: string[]): Promise<void> {
    const gen = ++this.armGen;
    const foldId = await this.client.canFoldTransform(targets[0]).catch(() => null);
    if (gen !== this.armGen) return; // tool switched / re-armed while in flight
    if (foldId) {
      const stored = await this.client.getOperationParams(foldId).catch(() => undefined);
      if (gen !== this.armGen) return;
      const seed = transformSeedFromParams(stored);
      if (seed && sameIdSet(seed.targets, targets)) {
        this.armTransform(seed.targets, foldId, seed.seed);
        return;
      }
    }
    this.armTransform(targets, undefined, { center: this.bodiesCenter(targets) });
  }

  private armTransform(targets: string[], editFeatureId?: string, seed?: TransformSeed): void {
    this.transformEditFeatureId = editFeatureId;
    this.transform = transformStep(transformInit(), { kind: "arm", targets, ...seed }).state;
    // Freeze what the visible geometry already carries — see the field's note.
    const armedParams = transformParamsOf(this.transform);
    this.transformArmedInverse = inversePlacement(
      armedParams.translate,
      armedParams.rotate.center,
      armedParams.rotate.axis,
      armedParams.rotate.angleDeg,
    );
    this.alignMovingFrame = null;
    toolStore.setState({ phase: "armed" });
    viewportStore
      .getState()
      .setStatusHint("Move / Rotate along an axis, then ✓", { sticky: true });
    this.rebuildTransformGhost();
    this.showTransformChip();
    this.updateTransformGizmo();
    this.updateDebug();
  }

  private showTransformChip(): void {
    toolChipStore
      .getState()
      .showTransform(
        this.transform.mode,
        this.transform.axis,
        transformValue(this.transform),
        this.bodiesCenter(this.transform.targets),
        {
          onTransformMode: (m) => this.onTransformEvent({ kind: "setMode", mode: m }),
          onAxis: (a) => this.onTransformEvent({ kind: "setAxis", axis: a }),
          onValue: (v) => this.onTransformEvent({ kind: "setValue", value: v }),
          onConfirm: () => void this.commitTransform(),
          onCancel: () => this.resetToSelect(),
          onCopy: (c) => this.onTransformEvent({ kind: "setCopy", copy: c }),
          onAlign: () => this.toggleAlign(),
        },
        { copy: this.transform.copy },
      );
    toolChipStore.getState().setAlignPhase(this.transform.alignPhase);
  }

  /** One chip edit: step the FSM, then re-publish the chip's view + the ghost. */
  private onTransformEvent(e: Parameters<typeof transformStep>[1]): void {
    this.transform = transformStep(this.transform, e).state;
    this.publishTransformState();
  }

  /**
   * Re-publish everything downstream of the FSM: the chip's view, the ghost and
   * the gizmo's pose. Called after EVERY placement event — chip, gizmo drag or
   * seed — so the FSM stays the one writer and the two surfaces cannot disagree.
   */
  private publishTransformState(): void {
    const chip = toolChipStore.getState();
    chip.setTransformMode(this.transform.mode);
    chip.setAxis(this.transform.axis);
    // The chip's number is a VIEW of one component, so it must be re-read after
    // every event — switching Move X → Move Y shows the stored dy, not the dx.
    chip.setValue(transformValue(this.transform));
    chip.setCopy(this.transform.copy);
    chip.setAlignPhase(this.transform.alignPhase);
    this.rebuildTransformGhost();
    this.updateTransformGizmo();
    this.updateDebug();
  }

  // ── placement gizmo (WP-B W2) ────────────────────────────────────────────────
  //
  // Three grab kinds on one gizmo — arrow (one axis), quad (two axes) and ring
  // (rotate about an axis through the pivot). Two rules make it safe:
  //
  // THE HANDLE DECIDES MODE + AXIS, and it does so THROUGH the FSM (`grab`), never
  // by writing the chip directly. The chip segments then re-render from FSM state,
  // so grabbing the Z ring and clicking [Rotate][Z] are literally the same edit.
  //
  // THE GIZMO RIDES THE BODY but never rotates with it. Its origin is the frozen
  // pivot pushed through the live placement; its arms stay world-parallel because
  // the record stores a WORLD translation and a WORLD rotation axis, and an arm
  // pointing anywhere else would author a direction the record cannot express.

  /** Where the gizmo sits right now: the frozen pivot under the live placement. */
  private transformGizmoOrigin(): Vec3 {
    const p = transformParamsOf(this.transform);
    const m = placementMatrix(p.translate, p.rotate.center, p.rotate.axis, p.rotate.angleDeg);
    return applyPlacementToPoint(m, this.transform.center);
  }

  /** Show / move the gizmo while armed; hide it otherwise. */
  private updateTransformGizmo(): void {
    if (
      this.transform.phase !== "armed" ||
      this.transform.targets.length === 0 ||
      // The align picks own the pointer, and the gizmo's handles sit exactly
      // where the user is trying to click a face.
      this.transform.alignPhase !== null
    ) {
      this.hideTransformGizmo();
      return;
    }
    this.deps.engine.showTransformGizmo(this.transformGizmoOrigin());
    this.transformGizmoShown = true;
  }

  /**
   * Begin a gizmo gesture. `alt` at grab turns on COPY: with a cumulative record
   * there is no such thing as "a copy for this one gesture" — the placement has a
   * single `copy` flag — so Alt sets the flag the [Copy] segment shows, and the
   * user can see and undo it. Returns false when the press missed every handle.
   */
  private startGizmoDrag(hit: TransformGrab, clientX: number, clientY: number, alt: boolean): boolean {
    const ray = this.engine.screenRay(clientX, clientY);
    if (!ray) return false;
    const origin = this.transformGizmoOrigin();
    this.transform = transformStep(this.transform, { kind: "grab", grab: hit, copy: alt || undefined }).state;
    const axis = WORLD_AXIS[hit.axis];
    const grabScalar = hit.kind === "axis" ? (axisDragDelta(ray, origin, axis) ?? 0) : 0;
    // A FREE (aligned) axis is not any of the three rings, so a ring grab starts
    // a NEW rotation from 0 — the same rule the chip's `transformValue` follows.
    const startAngle =
      !this.transform.rotAxisVec && this.transform.rotAxis === hit.axis
        ? this.transform.angleDeg
        : 0;
    this.gizmoDrag = {
      grab: hit,
      origin,
      startTranslate: [...this.transform.translate],
      grabScalar,
      rawAngle: startAngle,
      prevAngle: hit.kind === "ring" ? (ringAngleDeg(ray, origin, axis) ?? 0) : 0,
      rawTranslate: [...this.transform.translate],
    };
    this.dragging = "transform";
    this.engine.setTransformGizmoActive(hit);
    toolStore.setState({ phase: "dragging" });
    this.publishTransformState();
    return true;
  }

  /** One drag frame: project the pointer, snap, and hand the result to the FSM. */
  private applyGizmoDrag(clientX: number, clientY: number, fine: boolean): void {
    const drag = this.gizmoDrag;
    if (!drag) return;
    const ray = this.engine.screenRay(clientX, clientY);
    if (!ray) return;
    const axis = WORLD_AXIS[drag.grab.axis];
    // A refused projection (degenerate view) holds the previous value — the drag
    // goes inert rather than throwing the body across the scene.
    if (drag.grab.kind === "ring") {
      const now = ringAngleDeg(ray, drag.origin, axis);
      if (now === null) return;
      drag.rawAngle = accumulateAngleDeg(drag.rawAngle, drag.prevAngle, now);
      drag.prevAngle = now;
      this.transform = transformStep(this.transform, {
        kind: "dragAngle",
        angleDeg: snapRotateDeg(drag.rawAngle, fine),
      }).state;
    } else {
      const moved = this.gizmoTranslateFrame(drag, ray, axis, fine);
      if (!moved) return;
      this.transform = transformStep(this.transform, { kind: "dragTranslate", translate: moved }).state;
    }
    this.publishTransformState();
  }

  /** The snapped translation for one arrow / quad drag frame, or null if refused. */
  private gizmoTranslateFrame(
    drag: GizmoDragState,
    ray: PointerRay,
    axis: Vec3,
    fine: boolean,
  ): Vec3 | null {
    const next: Vec3 = [...drag.rawTranslate];
    if (drag.grab.kind === "axis") {
      const now = axisDragDelta(ray, drag.origin, axis);
      if (now === null) return null;
      const i = axisIndex(drag.grab.axis);
      next[i] = drag.startTranslate[i] + (now - drag.grabScalar);
    } else {
      // The quad's delta already lies IN the plane, so the normal component is
      // zero and adding it componentwise cannot disturb the third axis.
      const delta = planeDragDelta(ray, drag.origin, axis);
      if (delta === null) return null;
      for (let a = 0; a < 3; a++) next[a] = drag.startTranslate[a] + delta[a];
    }
    drag.rawTranslate = next;
    return [snapTranslate(next[0], fine), snapTranslate(next[1], fine), snapTranslate(next[2], fine)];
  }

  /** Release: the placement stays ARMED at the dragged value (Enter / ✓ commits). */
  private endGizmoDrag(): void {
    this.gizmoDrag = null;
    this.dragging = null;
    this.engine.setTransformGizmoActive(null);
    toolStore.setState({ phase: "armed" });
    this.publishTransformState();
  }

  // ── align face-to-face (WP-B W2.5) ───────────────────────────────────────────
  //
  // A ONE-SHOT SOLVE, NOT A MATE. Two picks — a planar face on a body being
  // placed, then a planar face on a body that is not — produce one rigid motion
  // that lands the first face flush on the second (normals anti-parallel, centres
  // coincident). That motion is written into the SAME armed record every other
  // placement gesture writes, so a ✓ commits it exactly as a typed 30mm would,
  // and nothing survives to re-solve when an upstream feature changes. V1 has no
  // constraint graph; a persistent mate would be a promise it cannot keep.
  //
  // The sub-flow is a PHASE of the armed placement, never a tool of its own: the
  // targets, the frozen pivot and the fold decision all stay put across it, and
  // Esc walks back one pick at a time without disarming.
  //
  // ONE HOVER WRITER. The face tint goes through `selectionStore.setHover` — the
  // same store the select tool writes and the same `setHighlightState` bridge
  // renders — rather than a second highlight path of its own. The picker is
  // inactive while a model tool is armed, so there is no contention; the flag
  // below is what stops this controller restoring a hover it never set.

  /**
   * The [Align] segment. It renders `aria-pressed`, so a second press has to
   * mean "off" — otherwise the one control that says the flow is running is the
   * one control that cannot stop it. Entering drops the ghost + gizmo and asks
   * for the first face.
   */
  private toggleAlign(): void {
    if (this.transform.phase !== "armed") return;
    if (this.transform.alignPhase !== null) {
      // Through the reducer, one rung at a time — the FSM stays the one writer.
      while (this.transform.alignPhase !== null) {
        this.transform = transformStep(this.transform, { kind: "alignCancel" }).state;
      }
      this.endAlign();
      this.publishTransformState();
      this.publishAlignHint();
      return;
    }
    this.alignMovingFrame = null;
    this.transform = transformStep(this.transform, { kind: "beginAlign" }).state;
    // Modal for the two picks: an armed placement leaves orbit free (the gizmo is
    // grabbed by hitting it), but a click that means "take THIS face" must not be
    // swallowed by a camera spin. `endAlign` hands it back.
    this.deps.engine.setOrbitSuppressed(true);
    this.publishTransformState();
    this.publishAlignHint();
  }

  /** Esc during align: back one pick, then out of the flow (the arm survives). */
  private cancelAlignStep(): void {
    if (this.transform.alignPhase === null) return;
    this.transform = transformStep(this.transform, { kind: "alignCancel" }).state;
    if (this.transform.alignPhase !== "pickDest") this.alignMovingFrame = null;
    this.clearAlignHover();
    // The LAST rung out of the flow is an exit like any other — hand orbit back.
    if (this.transform.alignPhase === null) this.deps.engine.setOrbitSuppressed(false);
    this.publishTransformState();
    this.publishAlignHint();
  }

  /** Leave the flow without solving (tool switch / commit / cancel teardown). */
  private endAlign(): void {
    this.alignMovingFrame = null;
    this.clearAlignHover();
    this.deps.engine.setOrbitSuppressed(false);
  }

  /** The prompt for whichever pick is outstanding — or the armed hint when none. */
  private publishAlignHint(): void {
    const hint =
      this.transform.alignPhase === "pickMoving"
        ? "Align: click the flat face to move · Esc backs out"
        : this.transform.alignPhase === "pickDest"
          ? "Align: click the flat face on another body to sit against · Esc backs out"
          : "Move / Rotate along an axis, then ✓";
    viewportStore.getState().setStatusHint(hint, { sticky: true });
  }

  /** A refusal during an align pick: say why, and STAY in the phase. */
  private alignRefusal(message: string): void {
    viewportStore.getState().setStatusHint(message, { severity: "error", sticky: true });
  }

  /** Hover-tint the face under the pointer while an align pick is outstanding. */
  private updateAlignHover(clientX: number, clientY: number): void {
    const hit = this.engine.probePick(clientX, clientY);
    const sel = selectionStore.getState();
    if (!hit || hit.kind !== "face" || !this.alignBodyAllowed(hit.bodyId)) {
      this.clearAlignHover();
      return;
    }
    // Unchanged hover ⇒ no store write ⇒ no repaint (render-on-demand holds).
    if (sel.hover?.kind === "face" && sel.hover.id === topoRefId(hit.bodyId, hit.topoKey)) return;
    sel.setHover({
      kind: "face",
      id: topoRefId(hit.bodyId, hit.topoKey),
      bodyId: hit.bodyId,
      topoKey: hit.topoKey,
      elementId: hit.elementId,
      anchor: { worldPoint: [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z] },
    });
    this.alignHidHover = true;
  }

  /** Drop a hover THIS controller set (never one somebody else owns). */
  private clearAlignHover(): void {
    if (!this.alignHidHover) return;
    this.alignHidHover = false;
    selectionStore.getState().setHover(null);
  }

  // ── modal-pick hover (the align pattern, generalised) ────────────────────────
  //
  // Every MODAL PICK phase — the hole seat, the boolean tool body, extrude's
  // ToFace and both boolean target picks — used to ask for a click with no
  // feedback at all: the picker is inactive while a model tool is armed, so the
  // ordinary hover tint is gone and the user clicks blind, learning what was
  // under the pointer only from what happens next. These two share the align
  // sub-flow's ONE HOVER WRITER contract verbatim (tint via
  // `selectionStore.setHover`, cleared only by whoever set it), so no second
  // highlight path exists and there is no contention to arbitrate.

  /**
   * Tint what a click would take right now.
   *
   * `allow` is the phase's own filter and MUST restate what the click handler
   * refuses: a tint on an element the click then rejects is worse than none.
   * `as: "body"` tints the WHOLE body for the phases whose click takes a body
   * (boolean tool / boolean target) rather than the face under the pointer.
   */
  private setToolHover(
    hit: PickHit | null,
    opts: { as: "element" | "body"; allow?: (h: PickHit) => boolean },
  ): void {
    if (!hit || (opts.allow && !opts.allow(hit))) {
      this.clearToolHover();
      return;
    }
    const ref: EntityRef =
      opts.as === "body"
        ? { kind: "body", id: hit.bodyId }
        : {
            kind: hit.kind,
            id: topoRefId(hit.bodyId, hit.topoKey),
            bodyId: hit.bodyId,
            topoKey: hit.topoKey,
            elementId: hit.elementId,
            anchor: { worldPoint: [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z] },
          };
    const sel = selectionStore.getState();
    // Unchanged hover ⇒ no store write ⇒ no repaint (render-on-demand holds).
    if (sel.hover?.kind === ref.kind && sel.hover.id === ref.id) return;
    sel.setHover(ref);
    this.toolHoverSet = true;
  }

  /** Drop a modal-pick hover THIS controller set (never one somebody else owns). */
  private clearToolHover(): void {
    if (!this.toolHoverSet) return;
    this.toolHoverSet = false;
    selectionStore.getState().setHover(null);
  }

  /** Whether a body is a legal target for the OUTSTANDING pick. */
  private alignBodyAllowed(bodyId: string): boolean {
    if (bodyId.startsWith("preview:")) return false;
    const isTarget = this.transform.targets.includes(bodyId);
    return this.transform.alignPhase === "pickMoving" ? isTarget : !isTarget;
  }

  /**
   * A click during an align pick. Every refusal names its own reason and leaves
   * the phase running — a two-pick flow that silently swallowed a bad click would
   * leave the user staring at an unchanged prompt with no idea why.
   */
  private tryPickAlignFace(clientX: number, clientY: number): void {
    const phase = this.transform.alignPhase;
    if (phase === null) return;
    const hit = this.engine.probePick(clientX, clientY);
    if (!hit || hit.kind !== "face" || hit.bodyId.startsWith("preview:")) {
      this.alignRefusal("Align: click a flat FACE (edges and empty space do not align)");
      return;
    }
    if (!this.alignBodyAllowed(hit.bodyId)) {
      this.alignRefusal(
        phase === "pickMoving"
          ? "Align: that face is not on a body being moved — pick one of the selected bodies"
          : "Align: pick a face on ANOTHER body — a body cannot be aligned to itself",
      );
      return;
    }
    const frame = this.alignFaceFrame(hit.bodyId, hit.topoKey, phase);
    if (!frame) {
      this.alignRefusal("Align: that face is not flat — align needs two planar faces");
      return;
    }
    if (phase === "pickMoving") {
      this.alignMovingFrame = frame;
      this.transform = transformStep(this.transform, { kind: "alignPickedMoving" }).state;
      this.clearAlignHover();
      this.publishTransformState();
      this.publishAlignHint();
      return;
    }
    this.applyAlign(frame);
  }

  /**
   * The picked face's planar frame, in the frame the SOLVE needs it in.
   *
   * The moving face is pulled back into the record's INPUT geometry (see
   * `transformArmedInverse`); the destination belongs to a body the record does
   * not touch, so it is already in the world frame the record's output lands in.
   */
  private alignFaceFrame(bodyId: string, topoKey: string, phase: AlignPhase): PlanarFaceFrame | null {
    const entry = getEntry(bodyId);
    if (!entry) return null;
    const ordinal = entry.faceIndex.ordinalForId(topoKey);
    if (ordinal < 0) return null;
    const frame = faceFrame(entry.view, ordinal);
    if (!frame || phase === "pickDest") return frame;
    const inv = this.transformArmedInverse;
    return inv ? transformFrame(inv, frame) : frame;
  }

  /** Solve against the destination frame and write the answer into the record. */
  private applyAlign(dest: PlanarFaceFrame): void {
    const moving = this.alignMovingFrame;
    if (!moving) return;
    const solved = alignPlacement(moving, dest, this.transform.center);
    if (!solved) {
      this.alignRefusal("Align: those two faces do not resolve to a placement");
      return;
    }
    this.transform = transformStep(this.transform, {
      kind: "alignApply",
      translate: solved.translate,
      angleDeg: solved.rotate.angleDeg,
      rotAxisVec: solved.rotate.axis,
    }).state;
    this.endAlign(); // solved ⇒ the picks are done: drop the hover, restore orbit
    this.publishTransformState();
    viewportStore
      .getState()
      .setStatusHint("Aligned flush — adjust or ✓ to commit", { sticky: true });
  }

  /**
   * The live L1 ghost: each target's own geometry at the composed placement.
   *
   * `copy: false` HIDES the sources while previewing (via the same
   * `previewHiddenBodies` machinery the L2 preview lane uses) so the ghost reads
   * as "the body, moved" rather than "a second body next to it". `copy: true`
   * keeps them — that IS the difference between the two.
   */
  private rebuildTransformGhost(): void {
    if (
      this.transform.phase !== "armed" ||
      isIdentityPlacement(this.transform) ||
      // While the align picks are running the SOURCES have to be visible: they
      // are what the user is about to click, and a ghost hides them (W2.5).
      this.transform.alignPhase !== null
    ) {
      this.hideTransformGhost();
      return;
    }
    const items = this.transformGhostItems();
    if (items.length === 0) return;
    this.deps.engine.showGhostPreviewMulti(items);
    if (!this.transform.copy) {
      this.deps.engine.setPreviewReplacedBodyIds([...this.transform.targets]);
      this.transformHidSources = true;
    }
  }

  /**
   * One ghost clone per target that has ingested geometry, at the shared matrix.
   *
   * The matrix is the live placement composed with the INVERSE of the one the
   * registry geometry already carries. On a fresh arm that inverse is the
   * identity and this is the plain placement. On a FOLD it is not, and without
   * it the ghost would show the stored motion applied twice — a re-edit of a
   * 30mm move would preview at 60mm and then commit at 30 (WP-B W2.5; the frame
   * distinction is the same one the align solve needs, see `alignSolve`).
   */
  private transformGhostItems(): { entry: MeshEntry; transforms: GhostTransform[] }[] {
    const p = transformParamsOf(this.transform);
    const live = placementMatrix(p.translate, p.rotate.center, p.rotate.axis, p.rotate.angleDeg);
    const m = this.transformArmedInverse
      ? composePlacements(live, this.transformArmedInverse)
      : live;
    const items: { entry: MeshEntry; transforms: GhostTransform[] }[] = [];
    for (const id of this.transform.targets) {
      const entry = getEntry(id);
      if (entry) items.push({ entry, transforms: [{ kind: "rawMatrix", m }] });
    }
    return items;
  }

  private hideTransformGhost(): void {
    this.deps.engine.hideGhostPreview();
    if (!this.transformHidSources) return;
    // Restore ONLY what this ghost hid. Guarded so a tool switch that never armed
    // a placement cannot stomp the preview lane's own hidden-body snapshot.
    this.deps.engine.setPreviewReplacedBodyIds([]);
    this.transformHidSources = false;
  }

  private async commitTransform(): Promise<void> {
    if (this.transform.phase !== "armed" || this.transform.targets.length === 0) return;
    if (this.transform.alignPhase !== null) return; // a pick is still outstanding
    const params = transformParamsOf(this.transform);
    const editFeatureId = this.transformEditFeatureId;
    const armed = this.transform;
    this.transform = transformStep(this.transform, { kind: "apply" }).state;
    this.hideTransformGhost();
    this.hideTransformGizmo();
    toolChipStore.getState().clear();
    const op: OperationOp = {
      opType: "TransformBody",
      featureId: editFeatureId,
      // `inputs[]` MIRRORS `params.targets` in order (SCHEMA §7.3) — the ordinal
      // is what names a `copy: true` result's `body_<opId>:<k>`.
      inputs: params.targets.map((bodyId) => ({ primary: { bodyId, kind: "body" as const } })),
      params,
    };
    try {
      const res = await this.client.applyOperation(op);
      this.applyResult(res);
      this.endAlign();
      selectionStore.getState().set(params.targets.map((id) => ({ kind: "body" as const, id })));
      this.transform = transformInit();
      this.transformEditFeatureId = undefined;
      this.transformArmedInverse = null;
      this.resetToSelect(placementHint(params));
    } catch (e) {
      // A refused placement stays ARMED with the reason — unlike pattern/mirror,
      // which reset to select. The user's numbers are still on screen and still
      // valid to adjust; throwing them away to read the error would be hostile.
      this.transform = armed;
      this.rebuildTransformGhost();
      this.showTransformChip();
      this.updateTransformGizmo();
      viewportStore
        .getState()
        .setStatusHint(`Move failed: ${errMessage(e)}`, { severity: "error", sticky: true });
    }
    this.updateDebug();
  }

  /**
   * Re-arm the placement tool on an existing `TransformBody` row (history
   * double-click). The targets come from the STORED params — never the current
   * selection — for the same reason the pattern re-edits do: a guess would
   * silently re-point the record at different bodies.
   */
  async editTransformFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    // Gate on `opType`, never `kind`: `dto.rs feature_kind` folds TransformBody
    // into `boolean` alongside the pattern/mirror ops.
    if (!feat || feat.opType !== "TransformBody") return;
    const gen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (gen !== this.armGen) return;
    const seed = transformSeedFromParams(stored);
    if (!seed) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit move: stored placement has no usable rotation axis", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    toolStore.getState().setTool("transform");
    // `setTool` runs `onToolChange` → `armTransformFromSelection`, whose fold
    // query is async; bump the token so that in-flight arm cannot land after this
    // one and clobber the record we were asked to edit.
    this.armGen++;
    this.armTransform(seed.targets, featureId, seed.seed);
  }

  /** Combined bbox centre of `ids` — the FROZEN pivot a fresh placement captures. */
  private bodiesCenter(ids: readonly string[]): Vec3 {
    const min: Vec3 = [Infinity, Infinity, Infinity];
    const max: Vec3 = [-Infinity, -Infinity, -Infinity];
    for (const id of ids) {
      const entry = getEntry(id);
      if (!entry) continue;
      for (let a = 0; a < 3; a++) {
        min[a] = Math.min(min[a], entry.view.bboxMin[a]);
        max[a] = Math.max(max[a], entry.view.bboxMax[a]);
      }
    }
    if (!Number.isFinite(min[0])) return [0, 0, 0]; // nothing ingested yet
    return [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
  }

  private cancelTransform(): void {
    this.hideTransformGhost();
    this.hideTransformGizmo();
    this.endAlign();
    this.transform = transformInit();
    this.transformEditFeatureId = undefined;
    this.transformArmedInverse = null;
    toolChipStore.getState().clear();
    // Republish: `onToolChange` has no updateDebug of its own, so without this the
    // debug surface keeps reporting a placement (and a gizmo) that is already gone
    // — the same reason `endDatumPick` publishes its own now-idle phase.
    this.updateDebug();
  }

  /** Drop the gizmo AND any gesture it owns (idempotent — every teardown hits it). */
  private hideTransformGizmo(): void {
    this.gizmoDrag = null;
    if (this.dragging === "transform") this.dragging = null;
    if (!this.transformGizmoShown) return; // never shown ⇒ nothing of ours to take down
    this.transformGizmoShown = false;
    this.deps.engine.setTransformGizmoActive(null);
    this.deps.engine.setTransformGizmoHover(null);
    this.deps.engine.hideTransformGizmo();
  }

  // ── datum plane tool (DATUM W1) ──────────────────────────────────────────────
  //
  // Two phases, both owned here (this controller owns model-mode pointer events
  // and the onToolChange dispatch):
  //
  //   base pick — the PlanePicker gizmo is reused VERBATIM (its orbit gate,
  //               hover chip and geometric labelling all come along for free);
  //   offset    — a live ghost quad + the datum chip. ✓ or Enter commits; there
  //               is deliberately NO click-away commit (`isArmedForClickAway`
  //               ignores the datum), because the offset phase has no drag
  //               gesture and a stray canvas click would silently author a datum.
  //
  // V1 authors OffsetFromPlane off a WORLD plane only. The backend also accepts
  // another datum's id as the base (chained offsets) — that is a later wave, and
  // is why `buildAddDatumPlane` takes a plain string rather than a plane kind.

  private startDatum(): void {
    this.datum = { base: null, offset: DEFAULT_DATUM_OFFSET };
    this.engine.setPlanePickerVisible(true);
    viewportStore
      .getState()
      .setStatusHint("Select a base plane for the datum — Esc to cancel", { sticky: true });
    this.updateDebug();
  }

  /** A base plane was clicked: hide the picker, show the ghost + offset chip. */
  private pickDatumBase(base: PickablePlane): void {
    const state = this.datum;
    if (!state || state.base !== null) return;
    state.base = base;
    this.engine.setPlanePickerVisible(false);
    const ghost = datumGhostPlane(base, state.offset);
    // The chip shows the base plane's GEOMETRIC name ("XY"/"XZ"/"YZ" by world
    // normal); the repo `kind` — which is legacy-swapped — is what gets stored.
    const label = geometricLabel(ghost.normal);
    getDatumVisuals()?.setGhost(base, state.offset, label);
    // The chip anchors ONCE (armed-chip convention): the ghost slides with the
    // offset, the chip stays where the plane was picked.
    toolChipStore.getState().showDatum(state.offset, ghost.origin, label, {
      onValue: (v) => this.onDatumOffset(v),
      onConfirm: () => void this.confirmDatum(),
      onCancel: () => {
        this.endDatumPick();
        this.resetToSelect();
      },
    });
    viewportStore
      .getState()
      .setStatusHint(`Datum offset from ${label} — Enter or ✓ to create, Esc to cancel`, {
        sticky: true,
      });
    this.updateDebug();
  }

  private onDatumOffset(offset: number): void {
    const state = this.datum;
    if (!state || state.base === null) return;
    state.offset = offset;
    toolChipStore.getState().setValue(offset);
    getDatumVisuals()?.setGhost(state.base, offset, geometricLabel(datumGhostPlane(state.base, 0).normal));
    this.updateDebug();
  }

  /**
   * Commit the armed datum. The tool chrome is torn down BEFORE the await: the
   * ghost + chip describe something uncommitted, and the REAL datum arrives
   * through the projection (mock lane: `mockApplyEditCommand`; real lane:
   * `projection-updated`) — the controller never writes it into the store itself.
   */
  private async confirmDatum(): Promise<void> {
    const state = this.datum;
    if (!state || state.base === null) return;
    const gen = ++this.commitGen;
    const base = state.base;
    const offset = state.offset;
    const name = nextDatumName(documentStore.getState().datums);
    this.endDatumPick();
    let res: ApplyOperationResult;
    try {
      res = await this.client.applyEditCommand(buildAddDatumPlane(mintUuid(), name, base, offset));
    } catch (e) {
      if (gen !== this.commitGen) return; // superseded mid-flight — touch nothing
      this.resetToSelect(`Create datum plane failed: ${errMessage(e)}`, {
        severity: "error",
        sticky: true,
      });
      return;
    }
    if (gen !== this.commitGen) return;
    this.applyResult(res);
    this.resetToSelect(`${name} created`);
  }

  /** Reverse `startDatum` (IDEMPOTENT): drop the picker, the ghost and the chip.
   *  Every teardown path funnels here — Esc, the chip ✕, a tool switch, a mode
   *  change (cancelAll) and dispose. */
  private endDatumPick(): void {
    if (!this.datum) return;
    this.datum = null;
    this.engine.setPlanePickerVisible(false);
    getDatumVisuals()?.setGhost(null, 0);
    toolChipStore.getState().clear();
    this.updateDebug(); // republish the now-idle phase (a tool switch has no other hook)
  }

  // ── pointer handling ─────────────────────────────────────────────────────────

  private onPointerDown = (e: PointerEvent): void => {
    if (toolStore.getState().mode !== "model") return;
    this.downX = e.clientX;
    this.downY = e.clientY;
    this.downButton = e.button;
    this.moved = false;
    if (e.button !== 0) return;

    // The placement gizmo claims a press that lands ON a handle and nothing else
    // — every other press while armed stays available to select (orbit is RMB+
    // Shift now, so it never collides with LMB regardless of what this tool does).
    // …but not while an align pick owns the pointer: the gizmo is hidden then,
    // and a press that still grabbed it would eat the face click (W2.5).
    if (this.transform.phase === "armed" && this.transform.alignPhase === null) {
      const grab = this.engine.hitTransformGizmo(e.clientX, e.clientY);
      if (grab && this.startGizmoDrag(grab, e.clientX, e.clientY, e.altKey)) return;
    }

    // The chip layer is a SIBLING of the canvas overlay inside the same container
    // this controller listens on, so a press on the chip bubbles here. The arrow
    // is depth-tested out of existence (`depthTest: false`) and hit-tested against
    // a fat envelope, so a chip pixel can sit over it — and then ✓ / the value
    // input would start a depth drag instead. Same exclusion the fillet, shell and
    // degraded-offset branches already apply.
    if (
      this.extrude.phase === "armed" &&
      !this.isExcludedClickAwayTarget(e.target) &&
      this.engine.hitExtrudeHandle(e.clientX, e.clientY)
    ) {
      this.dragging = "extrude";
      const ray = this.engine.screenRay(e.clientX, e.clientY);
      this.extrudeStartDepth = this.extrude.depth;
      this.extrudeGrabDepth = ray
        ? axisDepthFromRay(ray.origin, ray.dir, this.centroidWorld, this.normal)
        : this.extrude.depth;
      this.extrude = extrudeStep(this.extrude, { kind: "grab" }).state;
      this.extrudeGrabbed = true;
      this.syncExtrudeHandle(); // the second head retires once a direction is owned
      this.engine.setExtrudeHandleHover(true);
      toolStore.setState({ phase: "dragging" });
      this.updateDebug(); // publish the live "dragging" phase to the debug surface
    } else if (
      this.fillet.phase === "armed" &&
      !this.isExcludedClickAwayTarget(e.target) &&
      (this.filletDegraded || this.engine.hitExtrudeHandle(e.clientX, e.clientY))
    ) {
      // A resolved axis gets a real handle (the extrude/offset-face rule): only a
      // press ON it starts the drag, and orbit/select stay free everywhere else.
      // DEGRADED has no handle to hit-test, so it still claims every viewport press
      // — which is exactly why it has no click-away commit. The chip lives inside
      // the container's overlay, so a press ON the chip must NOT be swallowed as a
      // drag either way: excluding it is what makes the ✓ clickable.
      this.dragging = "fillet";
      this.filletDownX = e.clientX;
      this.filletDownY = e.clientY;
      // Signed continuity: a Chamfer's size lives on the NEGATIVE half of the same
      // number line, so a drag that reverses past zero re-types instead of jumping.
      this.filletStartValue =
        this.fillet.edgeOp === "Chamfer" ? -this.fillet.radius : this.fillet.radius;
      this.filletAxis = this.edgeOpScreenAxis(); // per grab — the camera may have orbited
      this.fillet = filletStep(this.fillet, { kind: "grabEdge" }).state;
      if (!this.filletDegraded) this.engine.setExtrudeHandleHover(true);
      toolStore.setState({ phase: "dragging" });
      this.throttle.setTrailingMs(EDGE_OP_DRAG_TRAILING_MS); // live while the pointer owns it
      this.updateDebug();
    } else if (this.shell.phase === "armed" && !this.isExcludedClickAwayTarget(e.target)) {
      this.dragging = "shell";
      this.shellDownY = e.clientY;
      this.shellStartThickness = this.shell.thickness;
      this.shell = shellStep(this.shell, { kind: "grab" }).state;
      toolStore.setState({ phase: "dragging" });
      this.updateDebug();
    } else if (this.offsetFace.phase === "armed" && this.startOffsetFaceGrab(e)) {
      // Handled inside the helper (which decides between the ARROW grab and the
      // degraded screen-space drag, and declines a press that is neither).
    } else if (this.revolve.phase === "armed") {
      // Defer grab to the first move: a plain click (no move) commits 360° instead.
      this.revolveArmedDown = true;
      this.revolveDownX = e.clientX;
      this.revolveLastX = e.clientX;
      this.revolveStartAngle = this.revolve.angle;
    }
  };

  /**
   * Claim a press for the armed offset, or decline it. Returns whether the drag
   * started.
   *
   * Two gestures, one tool. With a real arrow (`!offsetDegraded`) only a press ON
   * the handle counts, exactly like Extrude — every other press stays available to
   * orbit and select, which is why orbit is not suppressed there. In the DEGRADED
   * case there is no handle to hit-test, so the tool claims every viewport press
   * (the fillet/shell rule) and the chip is excluded so its ✓ stays clickable.
   *
   * Both capture their start value AT THE PRESS and report DIFFERENCES against it —
   * the grab point is never the arrow's origin, and taking the absolute depth
   * would jump the face to the cursor on the first frame.
   */
  private startOffsetFaceGrab(e: PointerEvent): boolean {
    // Only a free `Offset` has a drag at all: `Total`/`Radius`/`Diameter` are
    // absolute values with no zero to drag from, so they are typed, never dragged.
    if (this.offsetFace.distanceType !== "Offset") return false;
    if (!this.offsetDegraded) {
      if (!this.offsetAxis) return false;
      if (!this.engine.hitExtrudeHandle(e.clientX, e.clientY)) return false;
      const ray = this.engine.screenRay(e.clientX, e.clientY);
      if (!ray) return false;
      this.offsetGrabDepth = axisDepthFromRay(ray.origin, ray.dir, this.offsetAnchor, this.offsetAxis);
      this.engine.setExtrudeHandleHover(true);
    } else {
      if (this.isExcludedClickAwayTarget(e.target)) return false;
    }
    this.dragging = "offsetFace";
    this.offsetDownX = e.clientX;
    this.offsetDownY = e.clientY;
    this.offsetStartDistance = this.offsetFace.distance;
    this.offsetFace = offsetFaceStep(this.offsetFace, { kind: "grab" }).state;
    toolStore.setState({ phase: "dragging" });
    this.updateDebug();
    return true;
  }

  /** One drag frame for the armed offset (both gestures land here). */
  private applyOffsetFaceDrag(e: PointerEvent): void {
    let distance: number;
    if (!this.offsetDegraded && this.offsetAxis) {
      const ray = this.engine.screenRay(e.clientX, e.clientY);
      if (!ray) return;
      const depth = axisDepthFromRay(ray.origin, ray.dir, this.offsetAnchor, this.offsetAxis);
      // GRAB-DELTA, not the absolute depth: the arrow already sits at
      // `anchor + axis·distance`, so the pointer's own offset from the grab is the
      // only thing that may move the value.
      distance = this.offsetStartDistance + (depth - this.offsetGrabDepth);
    } else {
      // Degraded: raw screen deltas along the up-is-positive axis, the same
      // mapping the edge ops use when no world direction is resolvable.
      distance = signedValueFromDrag(
        this.offsetStartDistance,
        e.clientX - this.offsetDownX,
        e.clientY - this.offsetDownY,
        SCREEN_UP_AXIS,
        { worldPerPx: this.engine.planePixelWorld() },
      );
    }
    this.offsetFace = offsetFaceStep(this.offsetFace, { kind: "drag", distance }).state;
    toolChipStore.getState().setValue(this.offsetFace.distance);
    this.applyOffsetFaceState();
    this.sendPreview();
  }

  private onPointerMove = (e: PointerEvent): void => {
    if (Math.abs(e.clientX - this.downX) > DRAG_PX || Math.abs(e.clientY - this.downY) > DRAG_PX) {
      this.moved = true;
    }
    // Datum base pick owns the pointer: highlight the plane quad under it.
    if (this.datum && this.datum.base === null) {
      this.engine.planePickerHover(e.clientX, e.clientY);
      return;
    }
    // Region pick owns the pointer: hover-tint the region under it, nothing else.
    if (this.regionPick) {
      this.updateRegionHover(e.clientX, e.clientY);
      return;
    }
    // Align pick owns the pointer: tint the face it would take, nothing else.
    if (this.transform.alignPhase !== null) {
      this.updateAlignHover(e.clientX, e.clientY);
      return;
    }
    // The remaining MODAL PICK phases, in `onPointerUp`'s routing order. Each
    // owns the pointer outright (none of them has a drag gesture) and each filter
    // is its click handler's refusal restated — see `setToolHover`.
    if (this.hole.phase === "facePick" || this.hole.phase === "armed") {
      this.setToolHover(this.engine.probePick(e.clientX, e.clientY), {
        as: "element",
        // A curved seat is refused by the click (`tryPickHoleFace`), so it must
        // not light up as if it were takeable.
        allow: (h) =>
          h.kind === "face" &&
          !h.bodyId.startsWith("preview:") &&
          this.holeFaceFrame(h.bodyId, h.topoKey) !== null,
      });
      return;
    }
    if (this.boolean.phase === "pickTool") {
      this.setToolHover(this.engine.probePick(e.clientX, e.clientY), {
        as: "body",
        allow: (h) => !h.bodyId.startsWith("preview:") && h.bodyId !== this.boolean.targetBodyId,
      });
      return;
    }
    if (this.extrude.phase === "facePick") {
      this.setToolHover(this.engine.probePick(e.clientX, e.clientY), {
        as: "element",
        allow: (h) => h.kind === "face" && !h.bodyId.startsWith("preview:"),
      });
      return;
    }
    if (this.extrude.phase === "targetPick" || this.revolve.phase === "targetPick") {
      this.setToolHover(this.engine.probePick(e.clientX, e.clientY), {
        as: "body",
        allow: (h) => !h.bodyId.startsWith("preview:"),
      });
      return;
    }
    if (this.dragging === "transform") {
      // Shift is read off the LIVE event rather than a held-key flag: the fine
      // tier only ever matters for the frame being projected.
      this.applyGizmoDrag(e.clientX, e.clientY, e.shiftKey);
      return;
    }
    if (this.dragging === "extrude") {
      const ray = this.engine.screenRay(e.clientX, e.clientY);
      if (!ray) return;
      // GRAB-DELTA, not the absolute depth. The arrow now travels with the prism
      // (`syncExtrudeHandle`), so grabbing it mid-shaft at depth D and reporting
      // the absolute projection would add an arrow-length to D on EVERY re-grab —
      // a ratchet. Same fix, same reason as the offset-face arrow (`:5635`).
      const raw = axisDepthFromRay(ray.origin, ray.dir, this.centroidWorld, this.normal);
      const depth = this.extrudeStartDepth + (raw - this.extrudeGrabDepth);
      const modeBefore = this.extrude.booleanMode;
      this.extrude = extrudeStep(this.extrude, { kind: "drag", depth, symmetric: this.altHeld }).state;
      this.engine.setExtrudeDepth(this.extrude.depth, this.extrude.symmetric);
      toolChipStore.getState().setValue(this.extrude.depth);
      toolChipStore.getState().setSymmetric(this.extrude.symmetric); // Alt-drag syncs the ⇔ toggle
      // HOST-BOOLEAN: the reducer flips Add↔Cut with the drag direction while the arm
      // is still host-seeded. The flip has to be VISIBLE on the same frame the params
      // change, so the chip + destructive tint follow it here (the sketch-plane normal
      // of a face sketch IS the outward face normal, so depth<0 pushes into the host).
      // Cheap by construction: the compare only does work on an actual sign crossing.
      if (this.extrude.booleanMode !== modeBefore) {
        this.applyBooleanState(this.extrude.booleanMode, false, "extrude");
      }
      // AFTER the mode resolves: the arrow carries the destructive tint too, and
      // a Cut that only reddened the prism would leave the primary affordance
      // saying the opposite of what the op is about to do.
      this.syncExtrudeHandle();
      this.sendPreview();
      this.updateDebug(); // publish live phase ("dragging") + depth to the debug surface
    } else if (this.dragging === "fillet") {
      // RAW screen deltas: the sign lives entirely in `filletAxis` (which is
      // SCREEN_UP_AXIS — up grows — whenever no world direction was resolvable, so
      // the pre-unification mapping is reproduced exactly there).
      const signed = signedValueFromDrag(
        this.filletStartValue,
        e.clientX - this.filletDownX,
        e.clientY - this.filletDownY,
        this.filletAxis,
        { worldPerPx: this.engine.planePixelWorld() },
      );
      const before = this.fillet.edgeOp;
      this.fillet = filletStep(this.fillet, { kind: "drag", signed }).state;
      toolChipStore.getState().setValue(this.fillet.radius);
      // The flip has to be VISIBLE on the frame its params change, and it swaps the
      // whole preview session (opType is frozen per session) — so it replaces the
      // ordinary send rather than following it. Cheap by construction: the compare
      // only does work on an actual type crossing.
      if (this.fillet.edgeOp !== before) this.applyEdgeOpKindChange();
      else this.sendPreview();
    } else if (this.dragging === "shell") {
      const dy = this.shellDownY - e.clientY; // up-drag grows the thickness
      const thickness = radiusFromDrag(this.shellStartThickness, dy, { worldPerPx: this.engine.planePixelWorld() });
      this.shell = shellStep(this.shell, { kind: "drag", thickness }).state;
      toolChipStore.getState().setValue(thickness);
      this.sendPreview();
    } else if (this.dragging === "offsetFace") {
      this.applyOffsetFaceDrag(e);
    } else if (this.dragging === "revolve") {
      this.applyRevolveDrag(e.clientX);
    } else if (this.revolveArmedDown && this.moved && this.downButton === 0 && this.revolve.phase === "armed") {
      // First movement past the threshold promotes the armed press into an angle drag.
      this.dragging = "revolve";
      this.revolve = revolveStep(this.revolve, { kind: "grab" }).state;
      toolStore.setState({ phase: "dragging" });
      this.applyRevolveDrag(e.clientX);
    } else if (this.revolve.phase === "axisPick") {
      this.updateRevolveAxisHover(e.clientX, e.clientY);
    } else if (this.extrude.phase === "armed") {
      this.engine.setExtrudeHandleHover(this.engine.hitExtrudeHandle(e.clientX, e.clientY));
    } else if (this.offsetFace.phase === "armed" && !this.offsetDegraded) {
      // The offset arrow IS the extrude drag handle (one shared instance), so its
      // hover reads through the same probe.
      this.engine.setExtrudeHandleHover(this.engine.hitExtrudeHandle(e.clientX, e.clientY));
    } else if (this.fillet.phase === "armed" && !this.filletDegraded) {
      this.engine.setExtrudeHandleHover(this.engine.hitExtrudeHandle(e.clientX, e.clientY));
    } else if (this.transform.phase === "armed") {
      this.engine.setTransformGizmoHover(this.engine.hitTransformGizmo(e.clientX, e.clientY));
    }
  };

  private onPointerUp = (e: PointerEvent): void => {
    const wasClick =
      this.downButton === 0 && e.button === 0 && !this.moved &&
      Math.abs(e.clientX - this.downX) <= DRAG_PX && Math.abs(e.clientY - this.downY) <= DRAG_PX;
    this.downButton = -1;

    // Datum base pick owns the pointer: a click on a plane quad picks the base.
    if (this.datum && this.datum.base === null) {
      if (wasClick) {
        const kind = this.engine.planePickerHitTest(e.clientX, e.clientY);
        if (kind) this.pickDatumBase(kind);
      }
      return;
    }

    // Region pick owns the pointer: a click resolves the region under it.
    if (this.regionPick) {
      if (wasClick) this.tryPickRegion(e.clientX, e.clientY);
      return;
    }

    // Align pick owns the pointer: a CLICK takes the face, a drag was an orbit.
    if (this.transform.alignPhase !== null) {
      if (wasClick) this.tryPickAlignFace(e.clientX, e.clientY);
      return;
    }

    // The hole tool owns every viewport CLICK for as long as it is active: the
    // first places the hole, each later one moves it. A drag stays an orbit, and
    // a press on the chip is excluded so the ✓ remains clickable.
    if (this.hole.phase === "facePick" || this.hole.phase === "armed") {
      if (wasClick && !this.isExcludedClickAwayTarget(e.target)) {
        void this.tryPickHoleFace(e.clientX, e.clientY);
      }
      return;
    }

    if (this.dragging === "transform") {
      // House armed-commit pattern: release keeps the placement ARMED at the
      // dragged value; nothing reaches the timeline until Enter or the chip ✓.
      this.endGizmoDrag();
      return;
    }
    if (this.dragging === "extrude") {
      // MODEL-HARDEN Wave 1: release KEEPS the tool armed (no implicit commit).
      // Flush the trailing L2 at the final depth; the chip cluster stays editable.
      this.dragging = null;
      this.engine.setExtrudeHandleHover(false);
      this.extrude = extrudeStep(this.extrude, { kind: "release" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      this.sendPreview();
      this.updateDebug();
      return;
    }
    if (this.dragging === "fillet") {
      // Release KEEPS the tool armed at the dragged size (no implicit commit) —
      // the live kernel preview + editable chip stay, Enter / ✓ applies.
      this.dragging = null;
      if (!this.filletDegraded) this.engine.setExtrudeHandleHover(false);
      this.fillet = filletStep(this.fillet, { kind: "release" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      this.throttle.setTrailingMs(EDGE_OP_TRAILING_MS); // back to the armed floor
      this.sendPreview();
      this.updateDebug();
      return;
    }
    if (this.dragging === "shell") {
      this.dragging = null;
      this.shell = shellStep(this.shell, { kind: "release" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      this.sendPreview();
      this.updateDebug();
      return;
    }
    if (this.dragging === "offsetFace") {
      // Release KEEPS the tool armed at the dragged distance (no implicit commit):
      // the live kernel preview + editable chip stay, Enter / ✓ applies.
      this.dragging = null;
      this.engine.setExtrudeHandleHover(false);
      this.offsetFace = offsetFaceStep(this.offsetFace, { kind: "release" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      this.sendPreview();
      this.updateDebug();
      return;
    }
    if (this.dragging === "revolve") {
      // Release keeps the revolve armed at the final angle (no implicit commit);
      // Enter / chip-✓ / click-away confirm.
      this.dragging = null;
      this.revolveArmedDown = false;
      this.revolve = revolveStep(this.revolve, { kind: "release" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      this.updateDebug();
      return;
    }
    // A plain click while the revolve is armed is handled by the window-level
    // click-away commit (confirmRevolve at the current angle, 360° default) — the
    // old quickCommit branch is removed. Just drop the armed-down flag here.
    this.revolveArmedDown = false;
    // Revolve axis-pick (a click, not a drag).
    if (wasClick && this.revolve.phase === "axisPick") {
      this.tryPickRevolveAxis(e.clientX, e.clientY);
      return;
    }
    // Boolean tool-body pick (a click, not a drag).
    if (wasClick && this.boolean.phase === "pickTool") {
      const hit = this.engine.probePick(e.clientX, e.clientY);
      if (hit && hit.bodyId !== this.boolean.targetBodyId) this.pickBooleanTool(hit.bodyId);
      return;
    }
    // Boolean target-body pick during an Add/Cut with >1 candidate (a click).
    if (wasClick && this.extrude.phase === "targetPick") {
      this.tryPickExtrudeTarget(e.clientX, e.clientY);
      return;
    }
    if (wasClick && this.revolve.phase === "targetPick") {
      this.tryPickRevolveTarget(e.clientX, e.clientY);
      return;
    }
    // ToFace target pick (a click, not a drag).
    if (wasClick && this.extrude.phase === "facePick") {
      void this.tryPickExtrudeFace(e.clientX, e.clientY);
    }
  };

  // ── click-away commit (window capture — MODEL-HARDEN Wave 1) ─────────────────
  //
  // The controller's own pointer listeners are container-local, so a click OUTSIDE
  // the container (or on empty 3D space) can't reach them. A window-capture pair
  // confirms an armed extrude/revolve on a TRUE click (within DRAG_PX of the press,
  // so an orbit drag never commits) that lands off the chip / toolbar / inputs and
  // does not grab the depth/angle handle. Disabled during axis / target / region
  // pick (those phases are not `armed`).

  private onWindowPointerDown = (e: PointerEvent): void => {
    this.clickAwayArmed = false;
    if (e.button !== 0 || !this.isArmedForClickAway()) return;
    if (this.isExcludedClickAwayTarget(e.target)) return;
    if (this.pressGrabsHandle(e.clientX, e.clientY)) return; // a re-drag, not a click-away
    this.clickAwayArmed = true;
    this.clickAwayDownX = e.clientX;
    this.clickAwayDownY = e.clientY;
  };

  private onWindowPointerUp = (e: PointerEvent): void => {
    if (!this.clickAwayArmed) return;
    this.clickAwayArmed = false;
    if (e.button !== 0) return;
    const moved =
      Math.abs(e.clientX - this.clickAwayDownX) > DRAG_PX ||
      Math.abs(e.clientY - this.clickAwayDownY) > DRAG_PX;
    if (moved) return; // an orbit / angle drag — never commits
    if (this.isExcludedClickAwayTarget(e.target)) return;
    if (this.extrude.phase === "armed") void this.confirmExtrude();
    else if (this.revolve.phase === "armed") void this.confirmRevolve();
    else if (this.offsetFace.phase === "armed") void this.commitOffsetFace();
  };

  /** True while an extrude/revolve/offset is armed and no modal pick owns the pointer. */
  private isArmedForClickAway(): boolean {
    if (this.regionPick) return false;
    // OffsetFace joins ONLY in its handle gesture. The DEGRADED variant claims
    // every viewport press as a value drag (there is no arrow to miss), and a tool
    // that owns the press cannot also treat it as a click-away — the same reason
    // the edge ops and shell are absent from this list entirely.
    if (this.offsetFace.phase === "armed" && !this.offsetDegraded) return true;
    return this.extrude.phase === "armed" || this.revolve.phase === "armed";
  }

  /** A press that grabs the depth handle is a re-drag, not a click-away (extrude). */
  private pressGrabsHandle(x: number, y: number): boolean {
    if (this.extrude.phase === "armed") return this.engine.hitExtrudeHandle(x, y);
    // Same shared handle, same rule: a press ON the offset arrow starts a drag.
    if (this.offsetFace.phase === "armed" && !this.offsetDegraded) {
      return this.engine.hitExtrudeHandle(x, y);
    }
    // Revolve: any press is a potential angle drag — the moved-check decides commit
    // vs drag on release, so never suppress the click-away arm here.
    return false;
  }

  /** Chip / toolbar / input / overlay targets never trigger a click-away commit.
   *  Accepts any Element incl. SVGElement (a toolbar icon click lands on an <svg>/
   *  <path>, which is NOT an HTMLElement) — walk up via `.closest()` so an icon-inside-
   *  button still resolves to its excluded ancestor (finding 5). */
  private isExcludedClickAwayTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    if (target.closest("input,textarea,select,[contenteditable]")) return true;
    return !!target.closest(
      '[data-testid="model-tool-chip"],[role="toolbar"],[role="listbox"],[role="dialog"],button',
    );
  }

  private get engine(): ViewportEngine {
    return this.deps.engine;
  }

  /**
   * Gate/test hook: grab the extrude handle without a real pointerdown.
   *
   * There is no press to project, so the grab basis is ZEROED rather than
   * captured — `start + (raw - grab)` collapses to `raw`, i.e. the absolute
   * mapping the drag used before it became grab-relative. That is what the gate
   * helpers and the depth-exact unit tests drive.
   */
  forceExtrudeGrab(): void {
    if (this.extrude.phase !== "armed") return;
    this.dragging = "extrude";
    this.extrudeStartDepth = 0;
    this.extrudeGrabDepth = 0;
    this.extrudeGrabbed = true;
    this.extrude = extrudeStep(this.extrude, { kind: "grab" }).state;
    toolStore.setState({ phase: "dragging" });
  }

  /** True while an extrude preview is armed or dragging (gate readiness probe). */
  get extrudeActive(): boolean {
    return this.extrude.phase === "armed" || this.extrude.phase === "dragging";
  }

  /** Gate/test hook: pick a boolean tool body without a real click. */
  forceBooleanPick(toolBodyId: string): void {
    if (this.boolean.phase === "pickTool") this.pickBooleanTool(toolBodyId);
  }

  // ── extrude preview send / receive ───────────────────────────────────────────

  private get client(): CadClient {
    return this.deps.client;
  }

  /** Complete canonical operation state for both exact preview and commit. */
  private extrudePreviewParams(depth = this.extrude.depth): PreviewParams {
    const symmetric = this.extrude.symmetric;
    const params: PreviewParams = {
      distance: depth,
      draftAngleDeg: this.extrude.draftAngleDeg,
      extrudeMode: symmetric ? "Symmetric" : this.extrude.endCondition,
      booleanMode: this.extrude.booleanMode,
      twoDirections: !symmetric && this.extrude.twoDirections,
      extrudeMode2: this.extrude.endCondition2,
      distance2: this.extrude.depth2,
    };
    if (this.extrude.booleanMode !== "NewBody" && this.extrude.targetBodyId) {
      params.targetBodyId = this.extrude.targetBodyId;
    }
    if (!symmetric && this.extrude.endCondition === "ToFace" && this.extrude.targetFace) {
      params.targetFace = this.extrude.targetFace as SemanticRef;
    }
    if (
      !symmetric &&
      this.extrude.twoDirections &&
      this.extrude.endCondition2 === "ToFace" &&
      this.extrude.targetFace2
    ) {
      params.targetFace2 = this.extrude.targetFace2 as SemanticRef;
    }
    const featureId = this.previewSessions[0]?.draft.params.featureId;
    if (typeof featureId === "string") params.featureId = featureId;
    return params;
  }

  /**
   * Push the owner's CURRENT params to every open lane session, paced by the shared
   * throttle. OpType-blind by construction: the params come from `previewParamsFn`
   * (bound at arm time), never from a tool-specific argument, so every tool on the
   * lane shares one pacing path.
   */
  private sendPreview(staleRetry = false): void {
    if (this.previewSessions.length === 0) return;
    const paramsFn = this.previewParamsFn;
    if (!paramsFn) return;
    if (!staleRetry) this.stalePreviewRetryAttempted = false;
    const send = this.throttle.request(paramsFn(), performance.now());
    if (send) {
      for (const es of this.previewSessions) {
        this.client.updatePreview(es.session.sessionId, send.params, send.epoch);
      }
      this.markPreviewPending();
    }
    this.scheduleTrailing();
  }

  /**
   * Enter the "computing" state for an owner that has NO local L1 to look at while
   * the kernel works. Extrude and Revolve are deliberately excluded: the extrude's
   * unit prism and the revolve's lathe shell are already on screen at the current
   * depth/angle, so a "Computing preview…" hint would report a wait the user cannot
   * see and would fight the arm hint for the status line on every drag frame.
   */
  private markPreviewPending(): void {
    if (this.previewOwner === null) return;
    if (this.previewOwner === "extrude" || this.previewOwner === "revolve") return;
    if (this.previewPending) return;
    this.previewPending = true;
    // Only announce a wait the user would otherwise see nothing for — and NEVER
    // over an error. A re-armed preview after a failed commit fires this within
    // the same tick as the "X failed: <reason>" hint, and a progress message that
    // buried the reason would hide the only explanation the user gets.
    const hasMesh = this.previewSessions.some((es) => es.previewBodyIds.length > 0);
    const showing = viewportStore.getState().statusHint;
    if (!hasMesh && showing?.severity !== "error") {
      viewportStore.getState().setStatusHint("Computing preview…", { sticky: true });
      this.previewPendingHint = true;
    }
    this.updateDebug();
  }

  /** The kernel answered (result or failure): leave the pending state, restore the line. */
  private clearPreviewPending(): void {
    if (!this.previewPending) return;
    this.previewPending = false;
    if (!this.previewPendingHint) return; // never took the status line
    this.previewPendingHint = false;
    // Hand the line back to the owner's arm hint where one exists; otherwise just
    // stop claiming a computation that has finished.
    if (this.previewOwner === "extrude" || this.previewOwner === "revolve") {
      viewportStore.getState().setStatusHint(this.armHintFor(this.previewOwner), { sticky: true });
    } else if (this.previewArmHint) {
      // edgeOp / shell / offsetFace: no `armHintFor` entry, so the owner parks its
      // own arm hint here at arm time and gets the status line back verbatim.
      viewportStore.getState().setStatusHint(this.previewArmHint, { sticky: true });
    } else {
      viewportStore.getState().setStatusHint(null);
    }
  }

  private trailingTimer: ReturnType<typeof setTimeout> | null = null;
  private scheduleTrailing(): void {
    if (this.trailingTimer) return;
    this.trailingTimer = setTimeout(() => {
      this.trailingTimer = null;
      if (this.previewSessions.length === 0) return;
      const send = this.throttle.tick(performance.now());
      if (send) {
        for (const es of this.previewSessions) {
          this.client.updatePreview(es.session.sessionId, send.params, send.epoch);
        }
        this.scheduleTrailing();
      } else if (this.throttle.pending && this.throttle.inFlight === null) {
        // The poke period (90ms) is FIXED but the trailing floor is per-owner and
        // can be larger (edge ops 160ms, shell 200ms). With coalesced params and a
        // FREE in-flight slot, only the time floor is holding the send back — so
        // re-poke, or the newest params would sit there until the next user input.
        // (An in-flight epoch needs no poke: `onPreviewResult` pumps the queue.)
        this.scheduleTrailing();
      }
    }, 90);
  }

  private onPreviewResult(r: PreviewResult): void {
    this.resolveExactPreviewWaiter(r);
    const idx = this.previewSessions.findIndex((e) => e.session.sessionId === r.sessionId);
    if (idx < 0) return;
    const es = this.previewSessions[idx];
    this.clearPreviewPending(); // the kernel answered — whatever it said
    const now = performance.now();
    if (r.committed) {
      this.committedEpoch = r.epoch;
      this.updateDebug();
      return;
    }
    if (r.epoch < es.lastAppliedEpoch) return; // per-session out-of-order — drop
    if (idx === 0) {
      // The primary session paces the shared throttle (leading-edge + ≤1 in flight).
      const fresh = this.throttle.onResponse(r.epoch, now);
      if (!fresh) return; // stale drag result — discard
      es.lastAppliedEpoch = r.epoch;
      if (r.error) {
        this.clearPreviewCandidate(es);
        this.onPreviewFailure(r.error);
      } else if (r.bodies || r.mesh) {
        this.previewFailure = null;
        this.stalePreviewRetryAttempted = false;
        this.applyPreviewBodies(es, r);
        this.lastL2Epoch = r.epoch;
        // Restore the OWNER's arm hint (a session exists ⇒ the owner is set). A
        // recovered candidate MUST take the line back: otherwise the previous
        // "… preview failed" stays on screen contradicting a preview that now
        // works, and the user has no signal that ✓ is unblocked again.
        if (this.previewOwner === "extrude" || this.previewOwner === "revolve") {
          viewportStore.getState().setStatusHint(this.armHintFor(this.previewOwner), { sticky: true });
        } else if (this.previewArmHint) {
          viewportStore.getState().setStatusHint(this.previewArmHint, { sticky: true });
        }
      }
      const send = this.throttle.tick(now);
      if (send) {
        for (const e of this.previewSessions) {
          this.client.updatePreview(e.session.sessionId, send.params, send.epoch);
        }
      }
      this.updateDebug();
    } else {
      // Secondary sessions don't touch the throttle; per-session lastAppliedEpoch
      // is the only staleness guard (they follow the primary's epochs).
      if (r.error) {
        this.clearPreviewCandidate(es);
        this.onPreviewFailure(r.error);
      }
      else if (r.bodies || r.mesh) this.applyPreviewBodies(es, r);
      es.lastAppliedEpoch = r.epoch;
    }
  }

  /** Replace every exact candidate body; a Cut may split into multiple solids. */
  private applyPreviewBodies(es: ToolPreviewSession, result: PreviewResult): void {
    const bodies =
      result.bodies ??
      (result.mesh ? [{ bodyId: result.bodyId, mesh: result.mesh }] : []);
    this.engine.clearPreviewBody(es.previewBodyIds);
    for (const id of es.previewBodyIds) removeMesh(id);
    es.previewBodyIds = [];
    es.replacedBodyIds = result.replacedBodyIds ?? [];
    this.syncPreviewReplacedBodies();
    for (let i = 0; i < bodies.length; i++) {
      const previewId = `${es.session.previewBodyId}:${i}`;
      const view = parseMeshPayload(bodies[i].mesh);
      const entry = buildBodyObjects(view, previewId, ++this.previewMeshRev);
      swapMesh(previewId, entry);
      this.engine.setPreviewBody(entry);
      es.previewBodyIds.push(previewId);
    }
    // The kernel's exact result SUPERSEDES the L1 ghost. Both on screen at once
    // would show the faces twice — once where they are going, once where they
    // actually ended up — and the ghost is the less true of the two.
    if (this.previewOwner === "offsetFace" && bodies.length > 0 && !this.offsetGhostHidden) {
      this.offsetGhostHidden = true;
      this.engine.hideGhostPreview();
    }
  }

  /** Hide union of committed bodies claimed by live exact-preview sessions. */
  private syncPreviewReplacedBodies(): void {
    // Lightweight test engines predate exact-candidate visibility claims. Runtime
    // engine always implements this; keeping the seam optional avoids coupling
    // unrelated controller tests to a rendering-only method.
    (this.engine as Partial<ViewportEngine>).setPreviewReplacedBodyIds?.([
      ...new Set(this.previewSessions.flatMap((session) => session.replacedBodyIds)),
    ]);
  }

  /** Remove only failed/superseded session's exact candidate and visibility claim. */
  private clearPreviewCandidate(es: ToolPreviewSession): void {
    this.engine.clearPreviewBody(es.previewBodyIds);
    for (const id of es.previewBodyIds) removeMesh(id);
    es.previewBodyIds = [];
    es.replacedBodyIds = [];
    this.syncPreviewReplacedBodies();
  }

  /** The armed op's display noun — the prefix on every preview-lane message. */
  private previewOwnerNoun(): string {
    switch (this.previewOwner) {
      case "revolve":
        return "Revolve";
      // Fillet and Chamfer share one lane; `edgeOpKind` is the authored opType.
      case "edgeOp":
        return this.edgeOpKind;
      case "shell":
        return "Shell";
      case "offsetFace":
        return "Offset face";
      case "boolean":
        return "Boolean";
      case "hole":
        return "Hole";
      default:
        return "Extrude";
    }
  }

  /** Structured-log tag for whichever tool owns the shared preview lane. */
  private previewOwnerTag(): string {
    if (this.previewOwner === "edgeOp") return this.edgeOpKind.toLowerCase();
    return this.previewOwner ?? "extrude";
  }

  private onPreviewFailure(error: PreviewFailure): void {
    traceWarn(this.previewOwnerTag(), `preview failure (kind=${error.kind}): ${error.message}`);
    this.previewFailure = error;
    this.clearPreviewPending();
    // The exact candidate is gone; bring the L1 ghost back so the user is not left
    // looking at the last GOOD mesh while the tool reports a failure.
    if (this.previewOwner === "offsetFace" && this.offsetGhostHidden) {
      this.offsetGhostHidden = false;
      this.applyOffsetFaceState();
    }
    if (error.kind === "stalePreview" && !this.stalePreviewRetryAttempted) {
      this.stalePreviewRetryAttempted = true;
      this.sendPreview(true);
    }
    viewportStore
      .getState()
      .setStatusHint(`${this.previewOwnerNoun()} preview failed: ${error.message}`, {
        severity: "error",
        sticky: true,
      });
  }

  /**
   * Commit barrier — resolve once this session's exact candidate for the final
   * commit epoch has landed. A TIMEOUT PROCEEDS (`{ok:true}`) deliberately: the
   * commit is re-validated authoritatively by the backend, so the barrier exists
   * only to surface an in-flight preview failure and to give the final mesh a
   * chance to be computed. It must never be a hard dependency that can wedge a
   * commit behind a preview that never answers.
   */
  private waitForExactPreview(sessionId: string): Promise<ExactPreviewOutcome> {
    const stale = this.exactPreviewWaiters.get(sessionId);
    if (stale) {
      clearTimeout(stale.timer);
      // A superseded barrier reports `timedOut` too: it never saw its result either.
      stale.resolve({ ok: true, timedOut: true });
      this.exactPreviewWaiters.delete(sessionId);
    }
    return new Promise<ExactPreviewOutcome>((resolve) => {
      const timer = setTimeout(() => {
        this.exactPreviewWaiters.delete(sessionId);
        resolve({ ok: true, timedOut: true });
      }, exactPreviewTimeoutMs);
      this.exactPreviewWaiters.set(sessionId, { timer, resolve });
    });
  }

  /** Settle a pending commit barrier from an arriving exact result. */
  private resolveExactPreviewWaiter(r: PreviewResult): void {
    if (r.committed || r.epoch < this.commitFinalEpoch) return;
    const waiter = this.exactPreviewWaiters.get(r.sessionId);
    if (!waiter) return;
    clearTimeout(waiter.timer);
    this.exactPreviewWaiters.delete(r.sessionId);
    waiter.resolve(r.error ? { ok: false, error: r.error } : { ok: true, timedOut: false });
  }

  /**
   * Release every pending barrier (cancel / tool switch / dispose). The awaiting
   * commit re-checks `commitGen` immediately after, so the outcome is irrelevant.
   */
  private clearExactPreviewWaiters(): void {
    for (const w of this.exactPreviewWaiters.values()) {
      clearTimeout(w.timer);
      // Released without an answer — `timedOut` so a strict caller cannot read a
      // torn-down barrier as kernel confirmation. (The awaiting commit re-checks
      // `commitGen` immediately after, so the outcome is usually irrelevant.)
      w.resolve({ ok: true, timedOut: true });
    }
    this.exactPreviewWaiters.clear();
  }

  private removeExactPreviewMeshes(es: ToolPreviewSession): void {
    removeMesh(es.session.previewBodyId);
    this.clearPreviewCandidate(es);
  }

  /**
   * Commit the exact selected region (Enter / chip-✓ / click-away). Fresh features
   * commit the same full params sent to PreviewOp. Feature edits cancel the lane
   * and deep-merge only distance into stored params because exact-on-head preview
   * cannot reconstruct the feature's predecessor. Every await is generation-gated.
   */
  private async confirmExtrude(): Promise<void> {
    if (this.previewSessions.length === 0 || this.extrude.phase !== "armed") {
      trace(
        "extrude",
        `confirm IGNORED: sessions=${this.previewSessions.length} phase=${this.extrude.phase}`,
      );
      return;
    }
    if (this.previewFailure) {
      traceWarn("extrude", `confirm BLOCKED by preview failure: ${this.previewFailure.message}`);
      viewportStore
        .getState()
        .setStatusHint(`Cannot confirm invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const gen = ++this.commitGen;
    trace(
      "extrude",
      `confirm START: gen=${gen} sessions=${this.previewSessions.length} depth=${this.extrude.depth} ` +
        `end=${this.extrude.endCondition} boolean=${this.extrude.booleanMode} symmetric=${this.extrude.symmetric} ` +
        `sketch=${this.lastArmedSketch ?? "?"}`,
    );
    this.extrude = extrudeStep(this.extrude, { kind: "confirm" }).state; // → committing
    toolStore.setState({ phase: "committing" });
    const finalDepth = this.extrude.depth;
    const total = this.previewSessions.length;
    const committedBodyIds: string[] = [];

    // Profile-record guarantee (EXTRUDE-COMMIT-FIX): the regen planner resolves the
    // profile ONLY from the sketch's `Sketch` timeline record, and no interactive
    // path is obligated to have minted one (an Esc exit, a legacy in-session doc).
    // finishSketch is idempotent — unchanged content upserts nothing — so ensure it
    // here, at the exact boundary that needs it. Failure keeps the armed preview
    // (sessions untouched) so a retry is possible.
    const profileSketchId = this.previewSessions[0].draft.sketchId ?? this.lastArmedSketch;
    try {
      if (profileSketchId) await this.client.finishSketch(profileSketchId);
    } catch (e) {
      if (gen !== this.commitGen) return;
      traceWarn("extrude", `commit: profile-record guarantee FAILED: ${errMessage(e)}`);
      this.extrude = extrudeStep(this.extrude, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`Extrude failed: cannot record profile sketch: ${errMessage(e)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.commitGen) return;

    // Subscribe to onBodyLoaded BEFORE committing: a body can enter the scene while
    // a LATER session is still committing (the doc-changed → mesh-ingest is async),
    // so a subscription opened only after the loop would miss an early body and hang.
    const loaded = new Set<string>();
    this.commitBodyUnsub?.();
    this.commitBodyUnsub = this.deps.onBodyLoaded((id) => loaded.add(id));

    for (let k = 0; k < this.previewSessions.length; k++) {
      const es = this.previewSessions[k];
      // Force this session's final params out as the newest epoch, then commit.
      const now = performance.now();
      const params = this.extrudePreviewParams(finalDepth);
      this.throttle.request(params, now);
      const send = this.throttle.flush(now);
      this.commitFinalEpoch = send ? send.epoch : this.throttle.epoch;
      this.client.updatePreview(
        es.session.sessionId,
        send?.params ?? params,
        this.commitFinalEpoch,
      );

      const editFeatureId =
        typeof es.draft.params.featureId === "string" ? es.draft.params.featureId : undefined;
      trace(
        "extrude",
        `commit ${k + 1}/${total}: session=${es.session.sessionId} region=${es.draft.regionId} ` +
          `finalEpoch=${this.commitFinalEpoch} edit=${editFeatureId ?? "no"}`,
      );
      // A re-edit runs L1-only (the lane short-circuits updatePreview on featureId
      // and never emits a candidate), so only a fresh session has a barrier to wait on.
      if (!editFeatureId) {
        const exact = await this.waitForExactPreview(es.session.sessionId);
        if (gen !== this.commitGen) {
          trace("extrude", `commit ${k + 1}/${total}: superseded during exact-preview barrier`);
          return;
        }
        trace(
          "extrude",
          `commit ${k + 1}/${total}: exact-preview barrier → ${exact.ok ? "ok" : `FAILED: ${exact.error.message}`}`,
        );
        if (!exact.ok) {
          // Nothing consumed this lane session yet — release it so the re-arm below
          // does not leak it.
          void this.client.endPreview(es.session.sessionId, false);
          this.commitBodyUnsub?.();
          this.commitBodyUnsub = null;
          this.onExtrudeCommitFailed(k, total, exact.error.message, gen);
          return;
        }
      }

      let res: ApplyOperationResult | null;
      try {
        if (editFeatureId) {
          if (!this.extrudeStoredParams) {
            throw new Error("Stored Extrude parameters are unavailable");
          }
          await this.client.endPreview(es.session.sessionId, false);
          res = await this.client.applyEditCommand(
            updateScalarParamsCommand(editFeatureId, "Extrude", this.extrudeStoredParams, {
              distance: { value: finalDepth },
              // WP-C3: the arm SEEDED this from the same record, so an untouched
              // re-edit writes the identical value back (no-op) and an edited one
              // actually lands — the only two behaviours a chip may have.
              draftAngleDeg: { value: this.extrude.draftAngleDeg },
            }),
          );
        } else {
          res = await this.client.endPreview(es.session.sessionId, true);
        }
      } catch (e) {
        if (gen !== this.commitGen) return; // superseded (canceled / tool switched)
        traceWarn("extrude", `commit ${k + 1}/${total}: apply THREW: ${errMessage(e)}`);
        this.commitBodyUnsub?.();
        this.commitBodyUnsub = null;
        this.onExtrudeCommitFailed(k, total, errMessage(e), gen);
        return;
      }
      if (gen !== this.commitGen) return;
      trace(
        "extrude",
        `commit ${k + 1}/${total}: apply result rev=${res?.revision ?? "?"} ` +
          `changed=[${res?.changedBodies.map((b) => b.bodyId).join(", ") ?? ""}] ` +
          `removed=[${res?.removedBodies.join(", ") ?? ""}] error=${res?.errorMessage ?? "none"}`,
      );
      if (!res || (res.changedBodies.length === 0 && res.removedBodies.length === 0)) {
        traceWarn(
          "extrude",
          `commit ${k + 1}/${total}: REGEN FAILED (no changed bodies) — rolling back errored record: ` +
            `${res?.errorMessage ?? "no error message"}`,
        );
        this.commitBodyUnsub?.();
        this.commitBodyUnsub = null;
        // The command applied but its regen failed — pop the errored record so a
        // retried ✓ replaces it instead of stacking a duplicate.
        await this.rollbackFailedCommit();
        if (gen !== this.commitGen) return;
        this.onExtrudeCommitFailed(k, total, res?.errorMessage, gen);
        return;
      }
      this.applyResult(res);
      // Op-scoped bodies (finding 2): res.changedBodies now carries ONLY this region's
      // op bodies (incl. split children) — collect them all, not just [0]. The union
      // is deduped + selected in finishExtrude.
      for (const b of res.changedBodies) committedBodyIds.push(b.bodyId);
    }
    this.finishExtrudeAll(committedBodyIds, gen, loaded);
  }

  /**
   * A commit whose edit command WAS applied but whose regen step failed leaves an
   * errored record on the timeline while the tool re-arms with the same intent —
   * retrying ✓ would stack a duplicate failed op per click (observed: 20 stacked
   * failed Extrudes in one document). Undo exactly that one command; the armed
   * preview still carries the user's parameters, so no work is lost.
   */
  private async rollbackFailedCommit(): Promise<void> {
    try {
      const res = await this.client.undo();
      trace("extrude", `rollback: undid errored record, rev=${res.revision}`);
      this.applyResult(res);
    } catch (e) {
      // A failed rollback leaves the errored row visible — still recoverable by
      // hand (⌘Z / delete row), never worth masking the ORIGINAL failure hint.
      traceWarn("extrude", `rollback FAILED (errored row stays): ${errMessage(e)}`);
    }
  }

  /** A failed exact-region commit returns to armed and recreates its lane session. */
  private onExtrudeCommitFailed(k: number, total: number, reason: string | undefined, gen: number): void {
    traceWarn("extrude", `commit ${k + 1}/${total} FAILED → re-arming: ${reason ?? "unknown reason"}`);
    this.extrude = extrudeStep(this.extrude, { kind: "commitFailed" }).state; // committing → armed
    toolStore.setState({ phase: "armed" });
    const msg =
      total > 1
        ? `Extrude ${k + 1} of ${total} failed: ${reason ?? "unknown"} — remaining previews kept`
        : reason
          ? `Extrude failed: ${reason}`
          : "Extrude failed";
    viewportStore.getState().setStatusHint(msg, { severity: "error", sticky: true });
    void this.rearmRemainingExtrude(k, gen);
    this.updateDebug();
  }

  /** Re-arm the exact region after endPreview consumed a failed lane session. */
  private async rearmRemainingExtrude(k: number, gen: number): Promise<void> {
    if (!this.plane) return;
    // Drop the committed + failed sessions' STALE L2 preview bodies. removeMesh only
    // clears the mesh registry; the engine's scene handles linger (finding 11), so
    // clear every live L2 handle here — the surviving sessions re-render theirs via the
    // sendPreview below (only they keep a lane session).
    this.engine.clearPreviewBody();
    for (let i = 0; i < k; i++) this.removeExactPreviewMeshes(this.previewSessions[i]);
    const failed = this.previewSessions[k];
    this.removeExactPreviewMeshes(failed);
    const remaining = this.previewSessions.slice(k + 1);

    const freshSession = await this.deps.client.beginPreview(failed.draft);
    if (gen !== this.commitGen) {
      void this.deps.client.endPreview(freshSession.sessionId, false); // superseded — don't leak
      return;
    }
    // Reset per-session lastAppliedEpoch (finding 9): the throttle.reset() below restarts
    // epochs from a lower value; a surviving session that kept a LARGE lastAppliedEpoch
    // would stale-drop every new (lower) L2 epoch, freezing its preview. Zero them all.
    this.previewSessions = [
      { ...failed, session: freshSession, lastAppliedEpoch: 0 },
      ...remaining.map((s) => ({ ...s, lastAppliedEpoch: 0 })),
    ];

    // Every session on this path is an Extrude arm, so each carries its profile;
    // the filter is the type-level restatement of that, not a new tolerance.
    const profiles = this.previewSessions
      .map((e) => e.profile)
      .filter((p): p is PrismProfile => p !== undefined);
    this.centroidWorld = combinedCentroidWorld(this.plane, profiles);
    this.engine.showExtrudePreviews(this.plane, profiles, this.centroidWorld, this.normal);
    this.engine.setExtrudeDepth(this.extrude.depth, this.extrude.symmetric);
    this.engine.setPreviewTint(this.extrude.booleanMode === "Cut" ? "cut" : "normal");
    // `showExtrudePreviews` re-anchors the arrow at the CENTROID; the arm is mid-
    // gesture, so put it back on the depth it actually holds.
    this.syncExtrudeHandle();
    this.throttle.reset();
    this.sendPreview();
    this.updateDebug();
  }

  /**
   * Wait for ALL committed bodies to enter the scene, then teardown + select.
   * `loaded` carries the bodies that already arrived DURING the commit loop (via the
   * pre-loop subscription); the same subscription now also drives completion.
   */
  private finishExtrudeAll(bodyIds: string[], gen: number, loaded: Set<string>): void {
    this.commitBodyId = bodyIds[bodyIds.length - 1] ?? null;
    const pending = new Set(bodyIds.filter((id) => !loaded.has(id)));
    trace(
      "extrude",
      `commit COMPLETE: bodies=[${bodyIds.join(", ")}] awaiting mesh ingest for ${pending.size}`,
    );
    const done = (): void => {
      if (this.commitBodyTimer) {
        clearTimeout(this.commitBodyTimer);
        this.commitBodyTimer = null;
      }
      this.commitBodyUnsub?.();
      this.commitBodyUnsub = null;
      if (gen !== this.commitGen) return;
      this.finishExtrude(bodyIds);
    };
    if (pending.size === 0) {
      done();
      return;
    }
    // Re-point the live subscription at the completion check (it kept filling `loaded`).
    this.commitBodyUnsub?.();
    this.commitBodyUnsub = this.deps.onBodyLoaded((id) => {
      if (!pending.has(id)) return;
      pending.delete(id);
      if (pending.size === 0) done();
    });
    // Bounded wait (finding 8): a committed body that never ingests (e.g. hidden ⇒ no
    // mesh) must not hang the tool — finish with whatever loaded after the timeout.
    this.commitBodyTimer = setTimeout(done, bodyLoadTimeoutMs);
  }

  private finishExtrude(bodyIds: string[]): void {
    trace("extrude", `finish: teardown + select bodies=[${bodyIds.join(", ")}]`);
    const wasCut = this.extrude.booleanMode === "Cut";
    this.engine.hideExtrudePreview();
    for (const es of this.previewSessions) this.removeExactPreviewMeshes(es);
    this.engine.clearPreviewBody();
    toolChipStore.getState().clear();
    const consumedSketch = this.lastArmedSketch;
    const wasReedit = this.previewSessions.some((e) => e.draft.params.featureId !== undefined);
    // The lane sessions were consumed by the commit — release ownership so a later
    // sendPreview() cannot fire against a tool that is no longer armed.
    this.previewSessions = [];
    this.previewOwner = null;
    this.previewParamsFn = null;
    this.previewPending = false;
    this.previewPendingHint = false;
    this.extrude = extrudeStep(this.extrude, { kind: "settle" }).state;
    this.engine.setPreviewTint("normal");
    this.throttle.reset();
    const uniqueBodyIds = bodyIds.filter((id, i, a) => a.indexOf(id) === i);
    let completionHint: string;
    if (uniqueBodyIds.length > 0) {
      selectionStore.getState().set(uniqueBodyIds.map((id) => ({ kind: "body" as const, id })));
      completionHint =
        uniqueBodyIds.length > 1 ? `Extruded ${uniqueBodyIds.length} bodies` : "Extruded";
      // Consumed sketch auto-hides (Wave 2) — a FRESH arm only (a re-edit keeps it).
      // Backend-backed (TRUST wave): a local flip pops back visible on the next
      // projection. `finishExtrude` runs only after the WHOLE commit loop terminated
      // (`rollbackFailedCommit` routes to onExtrudeCommitFailed instead), so this can
      // never cross a rollback — it costs one extra undo step, accepted.
      if (consumedSketch && !wasReedit) void setSketchVisible(consumedSketch, false);
    } else {
      selectionStore.getState().clear();
      completionHint = wasCut ? "Cut completed" : "Extruded";
      if (consumedSketch && !wasReedit) void setSketchVisible(consumedSketch, false);
    }
    this.resetToSelect(completionHint);
    this.commitBodyId = null;
    this.updateDebug();
  }

  // ── edge-op (fillet / chamfer) + shell previewed commit ──────────────────────

  /**
   * The shared FRESH-commit sequence for the two value-drag preview tools
   * (fillet/chamfer + shell). Mirrors one iteration of `confirmExtrude`'s loop:
   * flush the FINAL params as the newest epoch, hold the exact-preview barrier so
   * an in-flight kernel refusal is seen BEFORE anything reaches the timeline, then
   * materialize the session through `endPreview(true)`.
   *
   * The barrier is what turns "commit blind, discover the refusal as an errored
   * history row" into "the ✓ is blocked and says why". Neither op binds a sketch,
   * so there is no profile-record guarantee here, and both are single-session, so
   * there is no per-region loop.
   *
   * `fallback` is committed directly when NO lane session is open (a `beginPreview`
   * the backend refused at arm time) — a missing preview must cost the preview,
   * never the tool.
   */
  private async commitPreviewedOp(
    fallback: OperationOp,
    gen: number,
    opts: { requireExactPreview?: boolean } = {},
  ): Promise<PreviewCommitOutcome> {
    const es = this.previewSessions[0];
    if (!es) {
      // OP-SPECIFIC STRICTNESS (OffsetFace): a missing lane session means the
      // kernel never evaluated this op at all. For every other tool that costs the
      // preview and nothing more; for an op whose operative set was frozen by a
      // separate handshake it would commit a record nothing has checked.
      if (opts.requireExactPreview) {
        return { kind: "failed", reason: "the kernel preview is unavailable" };
      }
      try {
        const res = await this.client.applyOperation(fallback);
        if (gen !== this.commitGen) return { kind: "superseded" };
        if (res.changedBodies.length === 0 && res.removedBodies.length === 0) {
          await this.rollbackFailedCommit();
          if (gen !== this.commitGen) return { kind: "superseded" };
          return { kind: "failed", reason: res.errorMessage ?? "no body changed" };
        }
        return { kind: "ok", res };
      } catch (e) {
        if (gen !== this.commitGen) return { kind: "superseded" };
        return { kind: "failed", reason: errMessage(e) };
      }
    }
    const now = performance.now();
    this.throttle.request(fallback.params as PreviewParams, now);
    const send = this.throttle.flush(now);
    this.commitFinalEpoch = send ? send.epoch : this.throttle.epoch;
    this.client.updatePreview(
      es.session.sessionId,
      send?.params ?? (fallback.params as PreviewParams),
      this.commitFinalEpoch,
    );
    const exact = await this.waitForExactPreview(es.session.sessionId);
    if (gen !== this.commitGen) return { kind: "superseded" };
    if (!exact.ok) {
      // Nothing consumed the lane session — release it so the re-arm cannot leak it.
      void this.client.endPreview(es.session.sessionId, false);
      this.releaseCommittedSession(es);
      return { kind: "failed", reason: exact.error.message };
    }
    // The barrier proceeds on TIMEOUT by design (see its doc comment). A strict
    // caller refuses that: "the kernel has not answered" is not "the kernel
    // approved", and the difference matters for an op carrying a frozen closure.
    if (opts.requireExactPreview && exact.timedOut) {
      void this.client.endPreview(es.session.sessionId, false);
      this.releaseCommittedSession(es);
      return { kind: "failed", reason: "the kernel did not confirm this operation in time" };
    }
    let res: ApplyOperationResult | null;
    try {
      res = await this.client.endPreview(es.session.sessionId, true);
    } catch (e) {
      this.releaseCommittedSession(es);
      if (gen !== this.commitGen) return { kind: "superseded" };
      return { kind: "failed", reason: errMessage(e) };
    }
    this.releaseCommittedSession(es);
    if (gen !== this.commitGen) return { kind: "superseded" };
    if (!res || (res.changedBodies.length === 0 && res.removedBodies.length === 0)) {
      // The command applied but its regen failed — pop the errored record so a
      // retried ✓ replaces it instead of stacking a duplicate (extrude's rule).
      await this.rollbackFailedCommit();
      if (gen !== this.commitGen) return { kind: "superseded" };
      return { kind: "failed", reason: res?.errorMessage ?? "no body changed" };
    }
    return { kind: "ok", res };
  }

  /** Drop a consumed/released lane session + its candidate meshes (no endPreview). */
  private releaseCommittedSession(es: ToolPreviewSession): void {
    this.removeExactPreviewMeshes(es);
    this.engine.clearPreviewBody();
    this.previewSessions = this.previewSessions.filter((s) => s !== es);
  }

  /** Release the lane after a successful fillet/chamfer/shell commit. */
  private teardownPreviewedTool(): void {
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.dragging = null;
  }

  /**
   * Apply the armed edge op. The RE-EDIT path is unchanged (a size-only scalar
   * merge, no lane session); a FRESH fillet/chamfer runs the previewed-commit
   * sequence so an OCCT-refused radius blocks the ✓ with the kernel's reason
   * instead of committing an errored row.
   */
  private async commitFillet(): Promise<void> {
    const editFeatureId = this.filletEditFeatureId;
    if (editFeatureId) {
      await this.commitEdgeOpEdit(editFeatureId);
      return;
    }
    const kind = this.edgeOpKind;
    if (this.filletEdges.length === 0) {
      this.cancelFillet();
      toolStore.getState().setTool("select");
      return;
    }
    if (this.fillet.phase !== "armed") return;
    if (
      this.filletPreparedRevision === null ||
      this.filletPreparedRevision !== documentStore.getState().revision
    ) {
      viewportStore.getState().setStatusHint(`${kind} selection changed — re-pick edges`, {
        severity: "error",
        sticky: true,
      });
      return;
    }
    if (this.previewFailure) {
      traceWarn(
        kind.toLowerCase(),
        `${kind} confirm BLOCKED by preview failure: ${this.previewFailure.message}`,
      );
      viewportStore
        .getState()
        .setStatusHint(`Cannot apply invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const step = filletStep(this.fillet, { kind: "confirm" });
    if (step.effect !== "commit") return;
    this.fillet = step.state; // → committing (also excludes us from the stale-arm guard)
    toolStore.setState({ phase: "committing" });
    const gen = ++this.commitGen;
    const edgeCount = this.filletEdges.length;
    const outcome = await this.commitPreviewedOp(
      { opType: kind, inputs: this.edgeOpInputs(), params: this.edgeOpParams() },
      gen,
      { requireExactPreview: true },
    );
    if (outcome.kind === "superseded") return;
    if (outcome.kind === "failed") {
      this.fillet = filletStep(this.fillet, { kind: "commitFailed" }).state; // → armed
      toolStore.setState({ phase: "armed" });
      viewportStore
        .getState()
        .setStatusHint(`${kind} failed: ${outcome.reason}`, { severity: "error", sticky: true });
      await this.openEdgeOpPreview(this.armGen); // re-arm the preview (work kept)
      this.updateDebug();
      return;
    }
    this.applyResult(outcome.res);
    this.teardownPreviewedTool();
    this.fillet = filletInit();
    this.filletEditFeatureId = undefined;
    this.filletStoredParams = undefined;
    this.filletEdges = [];
    this.filletPreparedRevision = null;
    this.resetToSelect(
      `${kind === "Chamfer" ? "Chamfered" : "Filleted"} ${edgeCount} edge${edgeCount > 1 ? "s" : ""}`,
    );
    this.updateDebug();
  }

  /** Size-only parametric re-edit — deep-merges into the stored params. */
  private async commitEdgeOpEdit(editFeatureId: string): Promise<void> {
    const kind = this.edgeOpKind;
    const radius = this.fillet.radius;
    // Move to `committing` so the document-revision bump this commit causes does
    // not read as an external model change and cancel our own arm.
    const step = filletStep(this.fillet, { kind: "confirm" });
    if (step.effect === "commit") this.fillet = step.state;
    this.deps.engine.setOrbitSuppressed(false);
    toolChipStore.getState().clear();
    // The result message is captured, not published, until the tool has been reset —
    // see `resetToSelect` for why publishing first would lose it.
    let failure: string | null = null;
    try {
      // A re-edit changes only the size and (optionally) the TYPE: deep-merge into
      // the stored params so the fillet's edgeIds + typed edges survive (a
      // whole-params replace would drop them). `kind` is the CURRENT type — a
      // segment flip here rewrites the record's `opType`, which core sanctions for
      // this one pair only (`session::op_type_edit_allowed`).
      if (!this.filletStoredParams) {
        throw new Error(`Stored ${kind} parameters are unavailable`);
      }
      const patch: Record<string, unknown> = { radius: { value: radius } };
      // The legacy SCHEMA §7.3 `mode` string is redundant with `opType`; keep it
      // consistent immediately when the stored params carry one. Core normalizes
      // it at the swap site regardless — that is the invariant holder, this is
      // only so the payload never LOOKS self-contradicting on the wire.
      if ("mode" in this.filletStoredParams) patch.mode = kind;
      // SCHEMA §7.3 (2026-08-03) second leg. `updateScalarParamsCommand` merges
      // SHALLOWLY over the stored params, so a CLEARED (or Fillet-typed) second
      // leg has to be removed from the base — a patch cannot delete a key. The
      // resulting op is then exactly what the record should hold.
      const base = { ...this.filletStoredParams };
      const distance2 = kind === "Chamfer" ? this.fillet.distance2 : null;
      if (distance2 === null) delete base.distance2;
      else patch.distance2 = { value: distance2 };
      const res = await this.client.applyEditCommand(
        updateScalarParamsCommand(editFeatureId, kind, base, patch),
      );
      this.applyResult(res);
    } catch (e) {
      failure = errMessage(e);
    }
    this.fillet = filletInit();
    this.filletEditFeatureId = undefined;
    this.filletStoredParams = undefined;
    this.filletEdges = [];
    this.filletPreparedRevision = null;
    if (failure !== null) {
      this.resetToSelect(`${kind} failed: ${failure}`, { severity: "error", sticky: true });
    } else {
      this.resetToSelect(`${kind} ${kind === "Chamfer" ? "distance" : "radius"} updated`);
    }
    this.updateDebug();
  }

  /**
   * Re-arm the fillet tool on an existing fillet feature (parametric edit seed;
   * mirrors editRevolveFeature). Seeds the chip with the feature's CURRENT radius;
   * committing routes through `UpdateOperationParams` (edge refs unchanged).
   */
  async editEdgeOpFeature(featureId: string, kind: EdgeOpKind = "Fillet"): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.kind !== "fillet") return;
    // `dto.rs feature_kind` folds Shell into the SAME `fillet` bucket (that is why
    // `editShellFeature` gates on opType), so the coarse kind alone would let a
    // Shell row arm the edge-op tool and commit a radius patch against shell
    // params. Reject on the exact authored opType whenever the projection carries
    // one; a payload without `opType` keeps the legacy kind-only behaviour.
    if (feat.opType && feat.opType !== "Fillet" && feat.opType !== "Chamfer") return;
    const radius = radiusFromValueText(feat.valueText);
    // Fetch the stored params so the size-only commit deep-merges instead of
    // dropping the edgeIds + typed edges (the projection does not expose them).
    const stored = await this.deps.client.getOperationParams(featureId).catch(() => undefined);
    // Selecting the tool fires cancelEdgeOp (which clears filletStoredParams and
    // resets the FSM to a Fillet arm), so the re-edit state is applied AFTER it.
    toolStore.getState().setTool("fillet");
    // Review F5 fence (pre-existing leak, sharpened by the tool-id unification):
    // `setTool` above is a NO-OP whenever the edge-op tool is ALREADY "fillet" —
    // the controller's subscriber only re-fires `onToolChange` on an actual value
    // change — so a re-edit opened while a PRIOR arm still owns a live or
    // in-flight preview session is never swept by the cancel above. And even when
    // `setTool` DOES change the tool (a fresh re-edit from `select`), `onToolChange`
    // re-arms `armEdgeOpFromSelection` off whatever edges happen to still be
    // selected in the viewport, and its `await openEdgeOpPreview(gen)` can resolve
    // AFTER this function has already returned, installing a session for those
    // (irrelevant) edges that nothing here would ever close — a leaked session,
    // and were it ever previewed against this feature's own edges, a double-apply
    // (PreviewOp always runs against the CURRENT head — the same rule `armShell`'s
    // own re-edit cites). Fence both paths: bump `armGen` so any such stale
    // continuation supersedes itself instead of installing, then re-run the
    // cancel sweep to close anything already open synchronously. Every field this
    // clears (filletStoredParams/filletEdges/filletEditFeatureId/fillet/chip) is
    // re-set by this function's own re-edit state below, in program order, so
    // nothing here can clobber it.
    this.invalidateArm();
    this.cancelFillet();
    this.filletStoredParams = stored; // set AFTER the tool-change cancel
    this.filletEdges = [];
    this.filletPreparedRevision = null;
    this.filletEditFeatureId = featureId;
    // edgeCount 1 keeps the FSM out of its bail path (a re-edit has no picks yet).
    // `auto:false` — a re-edit opens the COMMITTED type; there is no fresh pick and
    // no live preview to re-type against, so only an explicit choice may change it.
    // `touched:true` — the seeded size IS the committed size, so the pristine
    // reseed must NOT fire: flipping the segment on a committed 5 mm fillet has to
    // keep 5 mm, not rewrite it to the chamfer default.
    // SCHEMA §7.3 (2026-08-03): a two-distance chamfer re-opens with BOTH legs.
    // The second one comes from the STORED params (skip-none, so absent ⇒
    // equal-leg) and never from `valueText` — that string is a display form, and
    // `radiusFromValueText` deliberately reads only its leading number.
    const distance2 =
      kind === "Chamfer" ? storedOptionalScalar(stored?.distance2) : null;
    this.fillet = filletStep(filletInit(), {
      kind: "arm",
      edgeCount: 1,
      radius,
      edgeOp: kind,
      auto: false,
      touched: true,
      distance2,
    }).state;
    toolStore.setState({ phase: "armed" });
    this.deps.engine.setOrbitSuppressed(true); // modal: drag adjusts the size, not orbit
    viewportStore.getState().setStatusHint(this.edgeOpArmHint(), { sticky: true });
    // The edge-op re-edit gets the FULL armed cluster, not the bare numeric chip:
    // a committed row's TYPE is editable here, and a pure type flip changes no
    // number, so a chip whose only commit trigger is "the value differs" could
    // never commit one (`DimensionInput.commit` no-ops on unchanged text). ✓/Enter
    // is therefore the single commit path and `onValue` only sizes the arm — which
    // also means a blur while reaching for the [Fillet|Chamfer] segment can no
    // longer commit the op out from under the flip.
    //
    // The flip itself is chip + record only: `applyEdgeOpKindChange` gates its
    // preview reopen on `!this.filletEditFeatureId`, because a PreviewOp always
    // runs against the CURRENT head and would double-apply the edited feature.
    toolChipStore.getState().showFillet(
      radius,
      [0, 0, 0],
      (v) => this.onFilletChip(v),
      {
        onConfirm: () => void this.commitFillet(),
        onCancel: () => toolStore.getState().setTool("select"),
      },
      {
        edgeOp: this.fillet.edgeOp,
        showEdgeOpSegments: true,
        onEdgeOp: (k) => this.onEdgeOpChip(k),
        distance2: this.fillet.distance2,
        onDistance2: (v) => this.onEdgeOpDistance2(v),
      },
    );
    this.updateDebug();
  }

  // ── boolean ──────────────────────────────────────────────────────────────────

  private pickBooleanTool(toolBodyId: string): void {
    this.boolean = booleanStep(this.boolean, { kind: "pickTool", toolBodyId }).state;
    if (this.boolean.phase !== "armed") return;
    // The pick phase is over and the armed boolean is chip-driven (no drag at
    // all), so orbit — and the hover this controller owned — go back.
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    // Fallback highlight while the kernel preview is still pending (mock lane: the
    // ONLY feedback, since it never produces a candidate mesh — real lane: replaced
    // the moment the exact candidate + `replacedBodyIds` hide land, since those
    // bodies then render nothing to highlight anyway).
    selectionStore.getState().set([
      { kind: "body", id: this.boolean.targetBodyId! },
      { kind: "body", id: toolBodyId },
    ]);
    this.showArmedBooleanChip();
    // `previewArmHint` is what `clearPreviewPending`/`onPreviewResult` restore the
    // status line to once the kernel answers (boolean has no `armHintFor` entry —
    // same seam edgeOp/shell use).
    const hint = "Choose Union / Cut / Intersect, then Apply";
    viewportStore.getState().setStatusHint(hint, { sticky: true });
    this.updateDebug();
    void this.armBooleanPreview();
  }

  private setBooleanOp(op: BooleanOperation): void {
    this.boolean = booleanStep(this.boolean, { kind: "setOp", op }).state;
    toolChipStore.getState().setOp(op);
    this.engine.setPreviewTint(op === "Cut" ? "cut" : "normal");
    this.sendPreview();
  }

  /** Re-publish full Boolean controls after every failed preview/commit re-arm. */
  private showArmedBooleanChip(): void {
    const target = this.boolean.targetBodyId;
    const tool = this.boolean.toolBodyId;
    if (!target || !tool) return;
    toolChipStore.getState().showBoolean(
      this.boolean.op,
      this.bodyCenter(tool),
      (op) => this.setBooleanOp(op),
      () => void this.commitBoolean(),
    );
    this.previewArmHint = "Choose Union / Cut / Intersect, then Apply";
  }

  /** Complete canonical Boolean params for both exact preview and commit — the
   *  op-swap analog of `extrudePreviewParams`, reading the FSM live so a chip op
   *  change is picked up by whichever send fires next. */
  private booleanPreviewParams(): PreviewParams {
    return {
      operation: this.boolean.op,
      targetBodyId: this.boolean.targetBodyId ?? undefined,
      toolBodyId: this.boolean.toolBodyId ?? undefined,
    };
  }

  /** The two body inputs a Boolean draft/commit carries, synthesized from the
   *  armed target/tool exactly as the wire op does (`ipc/previewOps.ts booleanOp`). */
  private booleanPreviewDraft(): PreviewDraft {
    return {
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: this.boolean.targetBodyId!, kind: "body" } },
        { primary: { bodyId: this.boolean.toolBodyId!, kind: "body" } },
      ],
      params: this.booleanPreviewParams(),
    };
  }

  /**
   * Open the kernel-preview lane for the just-armed Boolean (target+tool picked):
   * the candidate mesh replaces the translucent two-body highlight with the real
   * fused/cut result once it lands. `armGen`-guarded — a superseded arm (Esc, tool
   * switch, or an `editBooleanFeature` re-edit landing mid-await) tears its own
   * session down instead of leaking or clobbering newer state.
   */
  private async armBooleanPreview(): Promise<void> {
    if (this.boolean.phase !== "armed") return;
    const gen = this.armGen;
    const draft = this.booleanPreviewDraft();
    let session: PreviewSession;
    try {
      session = await this.deps.client.beginPreview(draft);
    } catch (error) {
      if (gen !== this.armGen) return;
      viewportStore
        .getState()
        .setStatusHint(`Boolean preview failed: ${errMessage(error)}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    if (gen !== this.armGen) {
      void this.deps.client.endPreview(session.sessionId, false); // superseded — don't leak
      return;
    }
    this.openBooleanPreviewSession(session, draft);
    this.showArmedBooleanChip();
  }

  /** Wire a freshly opened Boolean session into the shared preview lane and send
   *  the first exact request. Shared by the initial arm and a post-failure re-arm. */
  private openBooleanPreviewSession(session: PreviewSession, draft: PreviewDraft): void {
    this.previewSessions = [{ session, draft, lastAppliedEpoch: 0, previewBodyIds: [], replacedBodyIds: [] }];
    this.previewOwner = "boolean";
    this.previewParamsFn = () => this.booleanPreviewParams();
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    // No drag on this lane (a chip op-swap, not a continuous gesture) — every send
    // should go out at its own leading edge rather than sit behind the drag floor.
    this.throttle.setTrailingMs(0);
    this.throttle.reset();
    this.engine.setPreviewTint(this.boolean.op === "Cut" ? "cut" : "normal");
    this.sendPreview();
    this.updateDebug();
  }

  private async commitBoolean(): Promise<void> {
    if (this.boolean.phase !== "armed" || !this.boolean.targetBodyId || !this.boolean.toolBodyId) return;
    const { targetBodyId, op: operation } = this.boolean;
    const editFeatureId = this.booleanEditFeatureId;
    const storedParams = this.booleanStoredParams;

    // RE-EDIT (operation swap only, wave-1): `editBooleanFeature` never opens a
    // preview session (the tool body is normally already consumed, so there is
    // nothing honest to preview) — commit straight through the scalar update,
    // preserving the stored target/tool ids VERBATIM. Re-sending them from the FSM
    // would be a fresh authoring of consumed inputs, and `applyOperation` would
    // append a SECOND boolean instead of editing the existing record.
    if (editFeatureId && storedParams) {
      this.boolean = booleanStep(this.boolean, { kind: "apply" }).state;
      toolChipStore.getState().clear();
      // The result message is captured, not published, until the tool has been reset —
      // see `resetToSelect` for why publishing first would lose it.
      let failure: string | null = null;
      try {
        const res = await this.client.applyEditCommand(
          updateScalarParamsCommand(editFeatureId, "Boolean", storedParams, { operation }),
        );
        this.applyResult(res);
        selectionStore.getState().set([{ kind: "body", id: targetBodyId }]);
      } catch (e) {
        failure = errMessage(e);
      }
      this.boolean = booleanInit();
      this.booleanEditFeatureId = undefined;
      this.booleanStoredParams = undefined;
      if (failure !== null) {
        this.resetToSelect(`${operation} failed: ${failure}`, { severity: "error", sticky: true });
      } else {
        this.resetToSelect(`Boolean changed to ${operation}`);
      }
      this.updateDebug();
      return;
    }

    // FRESH path: commit through the kernel-preview lane — the previewed candidate
    // and the committed op must be the exact same op (SCHEMA §7.6), so this mirrors
    // confirmExtrude's barrier: flush the final params as the newest epoch, wait for
    // the matching exact candidate, then commit that same lane session.
    if (this.previewFailure) {
      viewportStore
        .getState()
        .setStatusHint(`Cannot confirm invalid preview: ${this.previewFailure.message}`, {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const session = this.previewSessions[0];
    if (!session) {
      trace("boolean", "commit IGNORED: no open preview session yet");
      return;
    }
    const gen = ++this.commitGen;
    this.boolean = booleanStep(this.boolean, { kind: "apply" }).state;
    toolStore.setState({ phase: "committing" });
    toolChipStore.getState().clear();

    const now = performance.now();
    const params = this.booleanPreviewParams();
    this.throttle.request(params, now);
    const send = this.throttle.flush(now);
    this.commitFinalEpoch = send ? send.epoch : this.throttle.epoch;
    this.client.updatePreview(session.session.sessionId, send?.params ?? params, this.commitFinalEpoch);

    const exact = await this.waitForExactPreview(session.session.sessionId);
    if (gen !== this.commitGen) return;
    if (!exact.ok) {
      void this.client.endPreview(session.session.sessionId, false);
      this.onBooleanCommitFailed(exact.error.message, gen);
      return;
    }

    let res: ApplyOperationResult | null;
    try {
      res = await this.client.endPreview(session.session.sessionId, true);
    } catch (e) {
      if (gen !== this.commitGen) return;
      this.onBooleanCommitFailed(errMessage(e), gen);
      return;
    }
    if (gen !== this.commitGen) return;
    if (!res || (res.changedBodies.length === 0 && res.removedBodies.length === 0)) {
      // Applied but the regen failed: pop the errored record so a retried Apply
      // cannot stack a duplicate (the same defect class extrude's rollback closes).
      await this.rollbackFailedCommit();
      if (gen !== this.commitGen) return;
      this.onBooleanCommitFailed(res?.errorMessage, gen);
      return;
    }
    this.applyResult(res);
    for (const es of this.previewSessions) this.removeExactPreviewMeshes(es);
    this.previewSessions = [];
    this.previewOwner = null;
    this.previewParamsFn = null;
    this.previewFailure = null;
    this.previewArmHint = null;
    this.engine.clearPreviewBody();
    this.engine.setPreviewTint("normal");
    this.throttle.reset();
    selectionStore.getState().set([{ kind: "body", id: targetBodyId }]);
    this.boolean = booleanInit();
    this.booleanEditFeatureId = undefined;
    this.booleanStoredParams = undefined;
    this.resetToSelect(`${operation} applied`);
    this.updateDebug();
  }

  /** A failed exact-preview barrier or a failed/regen-failing commit returns the
   *  Boolean to armed and re-opens a fresh lane session (mirrors onExtrudeCommitFailed). */
  private onBooleanCommitFailed(reason: string | undefined, gen: number): void {
    traceWarn("boolean", `commit FAILED → re-arming: ${reason ?? "unknown reason"}`);
    const operation = this.boolean.op;
    this.boolean = { ...this.boolean, phase: "armed" };
    toolStore.setState({ phase: "armed" });
    viewportStore
      .getState()
      .setStatusHint(`${operation} failed: ${reason ?? "unknown reason"}`, {
        severity: "error",
        sticky: true,
      });
    void this.rearmBooleanPreview(gen);
    this.updateDebug();
  }

  /** Re-open the lane session after `onBooleanCommitFailed` consumed the failed one. */
  private async rearmBooleanPreview(gen: number): Promise<void> {
    if (this.boolean.phase !== "armed") return;
    for (const es of this.previewSessions) this.removeExactPreviewMeshes(es);
    this.engine.clearPreviewBody();
    this.previewSessions = [];
    this.previewOwner = null;
    this.previewParamsFn = null;

    const draft = this.booleanPreviewDraft();
    // Keep retry/cancel usable even while opening replacement preview stalls.
    this.showArmedBooleanChip();
    const session = await this.deps.client.beginPreview(draft);
    if (gen !== this.commitGen) {
      void this.deps.client.endPreview(session.sessionId, false); // superseded — don't leak
      return;
    }
    this.openBooleanPreviewSession(session, draft);
  }

  /**
   * Re-edit an existing Boolean feature — OPERATION SWAP ONLY (Union/Cut/Intersect).
   *
   * A committed boolean CONSUMES its tool body: the timeline removed it, so it is not
   * in the document any more and there is nothing to re-pick. Offering a body re-pick
   * here would be dishonest UI, so the whole re-edit is the one thing that IS still
   * authorable — the operation — sent as an `UpdateOperationParams` that preserves the
   * stored `targetBodyId`/`toolBodyId` verbatim.
   *
   * Gates on `opType`, NEVER `kind`: `dto.rs feature_kind` folds the pattern/mirror ops
   * into `boolean`, so a `kind` guard would also catch a LinearPattern.
   */
  async editBooleanFeature(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.opType !== "Boolean") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return; // superseded while getOperationParams was in flight
    const operation = matchBooleanOperation(stored?.operation);
    const targetBodyId = typeof stored?.targetBodyId === "string" ? stored.targetBodyId : null;
    const toolBodyId = typeof stored?.toolBodyId === "string" ? stored.toolBodyId : null;
    if (!operation || !targetBodyId || !toolBodyId) {
      viewportStore
        .getState()
        .setStatusHint("Cannot re-edit boolean: stored operation or bodies are missing", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    // Already on the boolean tool? `setTool` is then a no-op that skips the
    // onToolChange teardown, so cancel explicitly (extrude/revolve finding 10).
    if (toolStore.getState().modelTool === "boolean") this.cancelBoolean();
    toolStore.getState().setTool("boolean"); // may arm a selection-driven pickTool
    // Drive the FSM to `armed` on the STORED pair, overriding whatever the
    // selection-driven start above produced.
    let fsm = booleanStep(booleanInit(), { kind: "start", targetBodyId }).state;
    fsm = booleanStep(fsm, { kind: "pickTool", toolBodyId }).state;
    this.boolean = booleanStep(fsm, { kind: "setOp", op: operation }).state;
    this.booleanEditFeatureId = featureId;
    this.booleanStoredParams = stored;
    toolStore.setState({ phase: "armed" });
    // `setTool("boolean")` above may have run `startBooleanFromSelection`, which
    // suppresses orbit for its pick phase. This re-edit is chip-only and never
    // reaches that phase, so hand orbit straight back.
    this.deps.engine.setOrbitSuppressed(false);

    // The tool body is normally RETIRED (consumed by the original commit) — there is
    // nothing to highlight, so the re-edit is chip-only. Highlight the pair only in
    // the rare case both bodies still exist (e.g. a boolean that kept its tool).
    const bodies = documentStore.getState().bodies;
    const toolRetired = bodies[toolBodyId] === undefined;
    if (!toolRetired && bodies[targetBodyId]) {
      selectionStore.getState().set([
        { kind: "body", id: targetBodyId },
        { kind: "body", id: toolBodyId },
      ]);
    }
    toolChipStore.getState().showBoolean(
      operation,
      this.bodyCenter(toolRetired ? targetBodyId : toolBodyId),
      (next) => this.setBooleanOp(next),
      () => void this.commitBoolean(),
    );
    viewportStore.getState().setStatusHint("Change the boolean operation · Apply", { sticky: true });
    this.updateDebug();
  }

  private bodyCenter(bodyId: string): Vec3 {
    const entry = getEntry(bodyId);
    if (!entry) return [0, 0, 0];
    const mn = entry.view.bboxMin;
    const mx = entry.view.bboxMax;
    return [(mn[0] + mx[0]) / 2, (mn[1] + mx[1]) / 2, (mn[2] + mx[2]) / 2];
  }

  // ── undo / redo ──────────────────────────────────────────────────────────────

  async undo(): Promise<void> {
    const res = await this.client.undo();
    this.applyResult(res);
    if (res.opLabel) viewportStore.getState().setStatusHint(`Undid ${res.opLabel}`);
  }

  async redo(): Promise<void> {
    const res = await this.client.redo();
    this.applyResult(res);
    if (res.opLabel) viewportStore.getState().setStatusHint(`Redid ${res.opLabel}`);
  }

  // ── parametric edit (double-click extrude feature) ───────────────────────────

  /** Re-arm the extrude tool on an existing extrude feature (parametric edit seed). */
  editExtrudeFeature(featureId: string): void {
    void this.beginExtrudeFeatureEdit(featureId);
  }

  private async beginExtrudeFeatureEdit(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.kind !== "extrude") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return;
    const profile =
      stored && typeof stored.profile === "object" && stored.profile !== null
        ? (stored.profile as { sketchId?: unknown; regionId?: unknown })
        : null;
    if (typeof profile?.sketchId !== "string" || typeof profile.regionId !== "string") {
      viewportStore
        .getState()
        .setStatusHint("Extrude profile is missing or needs repair", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const storedDistance =
      stored && typeof stored.distance === "object" && stored.distance !== null
        ? Number((stored.distance as { value?: unknown }).value)
        : Number.NaN;
    const depth =
      Number.isFinite(storedDistance) ? storedDistance : parseFloat(feat.valueText) || DEFAULT_EXTRUDE_DEPTH;
    // If the extrude tool is ALREADY armed, setTool("extrude") is a no-op and skips the
    // onToolChange → cancelPreview that would end the open lane sessions — so end them
    // explicitly here or the re-edit arm leaks them (finding 10).
    if (toolStore.getState().modelTool === "extrude") this.cancelPreview();
    toolStore.getState().setTool("extrude");
    this.extrudeStoredParams = stored;
    await this.armExtrude(profile.sketchId, profile.regionId, featureId, depth);
  }

  /** Re-arm the revolve tool on an existing revolve feature (param-only angle edit). */
  editRevolveFeature(featureId: string): void {
    void this.beginRevolveFeatureEdit(featureId);
  }

  /**
   * Revolve counterpart of `beginExtrudeFeatureEdit` (REVOLVE-REGION-PARITY). The
   * profile comes from the feature's OWN stored params — never `lastArmedSketch` or
   * "the document's first sketch", which re-armed the WRONG sketch (and then its
   * first region) in any multi-sketch document. A missing/unrepaired profile refuses
   * with a sticky hint rather than guessing.
   */
  private async beginRevolveFeatureEdit(featureId: string): Promise<void> {
    const feat = documentStore.getState().features.find((f) => f.id === featureId);
    if (!feat || feat.kind !== "revolve") return;
    const requestGen = ++this.armGen;
    const stored = await this.client.getOperationParams(featureId).catch(() => undefined);
    if (requestGen !== this.armGen) return; // superseded while getOperationParams was in flight
    const profile =
      stored && typeof stored.profile === "object" && stored.profile !== null
        ? (stored.profile as { sketchId?: unknown; regionId?: unknown })
        : null;
    if (typeof profile?.sketchId !== "string" || typeof profile.regionId !== "string") {
      viewportStore
        .getState()
        .setStatusHint("Revolve profile is missing or needs repair", {
          severity: "error",
          sticky: true,
        });
      return;
    }
    const storedAngle =
      stored && typeof stored.angleDeg === "object" && stored.angleDeg !== null
        ? Number((stored.angleDeg as { value?: unknown }).value)
        : Number.NaN;
    const angle = Number.isFinite(storedAngle) ? storedAngle : angleFromValueText(feat.valueText);
    // If the revolve tool is ALREADY armed, setTool("revolve") is a no-op and skips the
    // onToolChange → cancelRevolve that tears the previous arm down — so cancel here
    // explicitly or the re-edit leaks the prior preview + chip (extrude finding 10).
    if (toolStore.getState().modelTool === "revolve") this.cancelRevolve();
    toolStore.getState().setTool("revolve");
    await this.armRevolve(profile.sketchId, featureId, angle, profile.regionId, stored);
  }

  // ── shared helpers ────────────────────────────────────────────────────────────

  private semanticRefFor(e: EntityRef): SemanticRef {
    return {
      primary: { bodyId: e.bodyId ?? "", elementId: e.elementId, kind: e.kind === "edge" ? "edge" : "face" },
      anchor: e.anchor ? { worldPoint: e.anchor.worldPoint, surfaceUv: e.anchor.surfaceUv } : undefined,
    };
  }

  private applyResult(res: {
    revision: number;
    features: FeatureRecord[];
    changedBodies?: { bodyId: string }[];
    removedBodies?: string[];
    appliedOps?: number;
  }): void {
    const doc = documentStore.getState();
    // Register any freshly-created body + drop removed ones (tree + visibility).
    const bodies = { ...doc.bodies };
    let n = Object.keys(bodies).length;
    for (const ref of res.changedBodies ?? []) {
      if (!bodies[ref.bodyId]) bodies[ref.bodyId] = { id: ref.bodyId, name: `Body ${++n}`, visible: true };
    }
    for (const id of res.removedBodies ?? []) delete bodies[id];
    const features = res.features.map(toFeatureMeta);
    doc.applyChange({
      revision: res.revision,
      features,
      bodies,
      dirty: true,
      // H7b: the result carries the cursor, which is what makes an insert-at-cursor
      // (authoring while rolled back) land at the right place — `nextAppliedOps`
      // could only ever clamp, so the fresh op read as an unapplied draft and the
      // H3 inline-value gate refused to edit it.
      appliedOps:
        res.appliedOps ?? nextAppliedOps(doc.appliedOps, doc.features.length, features.length),
    });
  }

  private updateDebug(): void {
    if (!this.deps.debug) return;
    // Only the PROFILE-based sessions carry a region + profile. Extrude arms every
    // session with both, so for the extrude surface below this is the same list —
    // the narrowing is the type-level restatement of that, not a new tolerance.
    const profiled = this.previewSessions.filter(
      (e): e is ToolPreviewSession & { region: SketchRegion; profile: PrismProfile } =>
        e.region !== undefined && e.profile !== undefined,
    );
    (window as unknown as { __extrudePreview?: unknown }).__extrudePreview = {
      l1Present: this.engine.isExtrudePreviewVisible(),
      phase: this.extrude.phase,
      // Revolve phase so e2e can assert armed-after-release for the revolve gesture
      // (MODEL-HARDEN Wave 1 debug surface extension).
      revolvePhase: this.revolve.phase,
      // Edge-op + shell phases: e2e asserts armed-after-release for their commit
      // gesture the same way (they have no 3D handle to scan for).
      filletPhase: this.fillet.phase,
      edgeOpKind: this.edgeOpKind,
      // Whether the DRAG DIRECTION still owns the edge-op type, and which tier
      // produced the direction it reads. e2e asserts the rule that binds them:
      // auto is armed only where the direction is sign-correct (bisector).
      edgeOpAuto: this.fillet.auto,
      edgeOpAxisSource: this.filletAxisSource,
      // The CHAMFER second leg (`null` = equal-leg, SCHEMA §7.3) — the only
      // readout e2e has of a value whose chip shows `=` rather than a number.
      edgeOpDistance2: this.fillet.distance2,
      shellPhase: this.shell.phase,
      // OffsetFace (SCHEMA §7.3). `offsetFaceCount` is the FROZEN CLOSURE, not the
      // picks — the difference between the two is the whole point of the
      // `PrepareOffsetFace` handshake and has no other visible surface.
      // `offsetPrepared` is the commit gate: null means no valid handshake, so a
      // ✓ would refuse.
      offsetFacePhase: this.offsetFace.phase,
      offsetDistance: this.offsetFace.distance,
      offsetDistanceType: this.offsetFace.distanceType,
      offsetChainTangent: this.offsetFace.chainTangentFaces,
      offsetFaceCount: this.offsetFaces.length,
      offsetPrepared: this.offsetPreparedRevision,
      // Whether the 3D arrow + ghost lane is live, or the tool fell back to the
      // screen-space value drag (a curved face, or normals that diverge too far).
      offsetDegraded: this.offsetDegraded,
      offsetValueError: this.offsetFace.valueError,
      // Hole tool (WP-C T3). The picked seat and every conditional dimension are
      // published: the chip cluster renders only the ACTIVE profile's fields, so
      // this is the only surface on which e2e can see that the other profile's
      // numbers survived a flip.
      holePhase: this.hole.phase,
      holeType: this.hole.holeType,
      holeDiameter: this.hole.diameter,
      holeDepth: this.hole.depth,
      holePoint: this.hole.point ? [...this.hole.point] : null,
      holeBodyId: this.hole.targetBodyId || null,
      holeCbDiameter: this.hole.cbDiameter,
      holeCbDepth: this.hole.cbDepth,
      holeCsDiameter: this.hole.csDiameter,
      holeCsAngleDeg: this.hole.csAngleDeg,
      holeEdit: this.holeEditFeatureId ?? null,
      // Placement tool (WP-B W1). `transformFold` is the record a ✓ would
      // REWRITE — the one bit of the fold decision that has no visible surface,
      // and therefore the one e2e has to read here rather than infer from the
      // history row count alone.
      transformPhase: this.transform.phase,
      transformMode: this.transform.mode,
      transformAxis: this.transform.axis,
      transformValue: transformValue(this.transform),
      transformTranslate: [...this.transform.translate],
      transformAngleDeg: this.transform.angleDeg,
      transformCenter: [...this.transform.center],
      transformTargets: [...this.transform.targets],
      transformFold: this.transformEditFeatureId ?? null,
      transformCopy: this.transform.copy,
      // Align sub-flow (WP-B W2.5). `transformRotAxisVec` is the only surface on
      // which a FREE rotation axis is visible at all — no chip can show it — so
      // e2e reads the solved placement here and checks it against geometry.
      transformAlignPhase: this.transform.alignPhase,
      transformRotAxisVec: this.transform.rotAxisVec ? [...this.transform.rotAxisVec] : null,
      // Gizmo surface (WP-B W2): whether it is on screen and which handle (if
      // any) a gesture currently owns. e2e reads the grab to prove that dragging
      // a ring is what re-typed the placement to Rotate about that axis.
      transformGizmo: this.transformGizmoShown,
      transformGrab: this.gizmoDrag ? { ...this.gizmoDrag.grab } : null,
      // Datum tool (DATUM W1): "idle" | "basePick" | "offset" + the armed values,
      // so e2e can assert the two-phase gesture without any DOM of its own.
      datumPhase: this.datum ? (this.datum.base === null ? "basePick" : "offset") : "idle",
      datumBase: this.datum?.base ?? null,
      datumOffset: this.datum?.offset ?? null,
      booleanMode: this.extrude.booleanMode,
      symmetric: this.extrude.symmetric,
      endCondition: this.extrude.endCondition,
      twoDirections: this.extrude.twoDirections,
      draftAngleDeg: this.extrude.draftAngleDeg,
      hasTargetFace: this.extrude.targetFace !== null,
      depth: this.extrude.depth,
      angle: this.revolve.angle,
      // Boolean state plus exact Extrude/Revolve region context.
      booleanTargetId: this.extrude.targetBodyId ?? this.revolve.targetBodyId ?? null,
      // What the direction rule has to work with — the only way to tell "no
      // material either way" from "the probe never ran" when a lane looks inert.
      booleanAuto: this.extrude.booleanAuto,
      materialPos: this.extrude.sides.pos,
      materialNeg: this.extrude.sides.neg,
      regionCount: this.previewSessions.length || this.revolveRegionIds.length || 1,
      selectedRegionIds: this.regionPick
        ? [...this.regionPick.select.selected]
        : profiled.map((entry) => entry.region.regionId),
      sketchId: this.lastArmedSketch,
      regionIds: profiled.map((entry) => entry.region.regionId),
      // The revolve arm's BOUND regions. Distinct from `regionIds` above, which
      // only sees sessions carrying a region — a revolve session carries its
      // profile and its draft, so the extrude-shaped surface reads empty for it
      // and cannot show which region a revolve actually bound (WP-C3).
      revolveRegionIds: [...this.revolveRegionIds],
      profileBounds: profiled.map((entry) => profileBounds(entry.profile)),
      candidateParams:
        this.previewSessions.length > 0 ? this.extrudePreviewParams() : null,
      previewError: this.previewFailure,
      // Which tool owns the open lane sessions (null when the lane is free), and
      // whether an exact preview is outstanding with nothing yet drawn for it.
      // Extrude never reports pending — its L1 prism is already on screen.
      previewOwner: this.previewOwner,
      previewPending: this.previewPending,
      sessionId: this.previewSessions[0]?.session.sessionId ?? null,
      // How many lane sessions are OPEN right now. Distinct from `regionCount`,
      // which falls back to the armed region count when the lane is closed.
      previewSessionCount: this.previewSessions.length,
      commitBodyId: this.commitBodyId,
      lastL2Epoch: this.lastL2Epoch,
      finalEpoch: this.commitFinalEpoch,
      committedEpoch: this.committedEpoch,
      throttleEpoch: this.throttle.epoch,
      inFlight: this.throttle.inFlight,
      // MODEL-OPS W0: hole count of the armed profile(s). A region whose fill
      // subtracts a hole yields a second recovered ring here, which is what makes
      // the extrude preview a tube instead of a slab — the only externally
      // observable difference, so e2e asserts on it.
      profileHoleCounts: profiled.map((s) => s.profile.holes.length),
    };

    // DEV-OBSERVABILITY Wave F — mirror the phase vector into the log lane, but
    // only when it actually MOVED: updateDebug is called from ~40 sites incl.
    // per-drag ones, so an unguarded event here would be drag-frequency. Note
    // this rides `deps.debug` (the early return above), so the SNAPSHOT lane is
    // `?vpdebug`-only; the `fsm` transition events from fsmLog are not.
    const sig = [
      this.extrude.phase,
      this.revolve.phase,
      this.fillet.phase,
      this.shell.phase,
      this.offsetFace.phase,
      this.boolean.phase,
    ].join("|");
    if (sig !== this.lastPhaseSig) {
      this.lastPhaseSig = sig;
      logDebug("fsm", `phases ${sig}`, {
        extrude: this.extrude.phase,
        revolve: this.revolve.phase,
        edgeOp: this.fillet.phase,
        shell: this.shell.phase,
        offsetFace: this.offsetFace.phase,
        boolean: this.boolean.phase,
        previewOwner: this.previewOwner,
        sessions: this.previewSessions.length,
      });
    }
  }

  // ── keyboard ──────────────────────────────────────────────────────────────────

  private onKeyDown = (e: KeyboardEvent): void => {
    // Esc cancels the datum tool from EITHER phase (base pick / offset) and owns
    // the key, so the global Esc-ladder does not also fire.
    if (e.key === "Escape" && this.datum) {
      e.preventDefault();
      e.stopPropagation();
      this.endDatumPick();
      this.resetToSelect();
      return;
    }
    // Enter confirms an armed datum (offset phase). Skipped while a chip input has
    // focus — DimensionInput consumes Enter itself and calls onConfirm (no double).
    if (e.key === "Enter" && this.datum && this.datum.base !== null && !isEditableTarget(e.target)) {
      e.preventDefault();
      void this.confirmDatum();
      return;
    }
    // Esc during region pick cancels cleanly back to idle (restore orbit), and owns
    // the key so the global Esc-ladder does not also fire.
    if (e.key === "Escape" && this.regionPick) {
      e.preventDefault();
      e.stopPropagation();
      this.cancelRegionPick();
      this.resetToSelect();
      return;
    }
    // Esc during an align pick walks BACK one pick (pickDest → pickMoving → off)
    // and leaves the placement armed — the Esc ladder, one rung at a time (W2.5).
    if (e.key === "Escape" && this.transform.alignPhase !== null) {
      e.preventDefault();
      e.stopPropagation();
      this.cancelAlignStep();
      return;
    }
    // Esc during a boolean target pick steps back to armed(NewBody) — NOT the whole
    // tool (the Esc ladder tail). Own the key so the global cancel doesn't also fire.
    if (e.key === "Escape" && this.extrude.phase === "facePick") {
      e.preventDefault();
      e.stopPropagation();
      this.cancelFacePick();
      return;
    }
    if (e.key === "Escape" && (this.extrude.phase === "targetPick" || this.revolve.phase === "targetPick")) {
      e.preventDefault();
      e.stopPropagation();
      this.cancelTargetPick();
      return;
    }
    // Enter confirms Revolve's multi-region selection (≥1).
    if (e.key === "Enter" && this.regionPick && !isEditableTarget(e.target)) {
      e.preventDefault();
      this.confirmRegionSelect();
      return;
    }
    // Enter confirms the armed extrude/revolve (capture phase). Skipped when a chip
    // input has focus — DimensionInput consumes Enter itself and stops propagation,
    // so this only fires for a canvas-focused Enter (no double-commit).
    if (e.key === "Enter" && !isEditableTarget(e.target)) {
      if (this.extrude.phase === "armed") {
        e.preventDefault();
        void this.confirmExtrude();
        return;
      }
      if (this.revolve.phase === "armed") {
        e.preventDefault();
        void this.confirmRevolve();
        return;
      }
      // Edge ops + shell join the same explicit gesture: release keeps them armed,
      // Enter (or the chip ✓) is what writes to the timeline.
      if (this.fillet.phase === "armed") {
        e.preventDefault();
        void this.commitFillet();
        return;
      }
      if (this.shell.phase === "armed") {
        e.preventDefault();
        void this.commitShell();
        return;
      }
      if (this.offsetFace.phase === "armed") {
        e.preventDefault();
        void this.commitOffsetFace();
        return;
      }
      if (this.hole.phase === "armed") {
        e.preventDefault();
        void this.commitHole();
        return;
      }
      // A placement joins the same explicit gesture: nothing writes to the
      // timeline until Enter or the chip ✓. An OUTSTANDING align pick holds it
      // back — the user is mid-gesture, and committing the pre-align placement
      // is not what "Enter" means there.
      if (this.transform.phase === "armed" && this.transform.alignPhase === null) {
        e.preventDefault();
        void this.commitTransform();
        return;
      }
    }
    if (e.key === "Alt") {
      this.altHeld = true;
      if (this.dragging === "extrude") {
        this.extrude = extrudeStep(this.extrude, { kind: "drag", depth: this.extrude.depth, symmetric: true }).state;
        this.engine.setExtrudeDepth(this.extrude.depth, true);
        toolChipStore.getState().setSymmetric(true);
        this.sendPreview();
      } else if (this.dragging === "revolve") {
        this.applyRevolveDrag(this.revolveLastX); // re-evaluate the snap without Alt
      }
    }
  };

  private onKeyUp = (e: KeyboardEvent): void => {
    if (e.key === "Alt") {
      this.altHeld = false;
      if (this.dragging === "extrude") {
        this.extrude = extrudeStep(this.extrude, { kind: "drag", depth: this.extrude.depth, symmetric: false }).state;
        this.engine.setExtrudeDepth(this.extrude.depth, false);
        toolChipStore.getState().setSymmetric(false);
        this.sendPreview();
      } else if (this.dragging === "revolve") {
        this.applyRevolveDrag(this.revolveLastX); // re-apply the 45° snap
      }
    }
  };

  // ── cancel / teardown ──────────────────────────────────────────────────────────

  /**
   * Release the shared kernel-preview lane: end every open session, drop its exact
   * candidate meshes, and clear everything keyed to those sessions (commit
   * barriers, the failure latch, the pending flag, the throttle + its trailing
   * timer). OWNER-AGNOSTIC on purpose — the extrude-specific engine teardown (the
   * L1 prism, the depth handle, the Cut tint, the FSM reset) stays in
   * `cancelPreview`, so a future tool's cancel path can reuse this half as-is.
   */
  private closePreviewSessions(): void {
    for (const es of this.previewSessions) {
      // `Promise.resolve(...)` normalizes a lane that returns nothing and keeps the
      // post-dispose rejection guard the old dispose() path had (finding 6).
      void Promise.resolve(this.client.endPreview(es.session.sessionId, false)).catch(() => {});
      this.removeExactPreviewMeshes(es);
    }
    this.previewSessions = [];
    this.previewOwner = null;
    this.previewParamsFn = null;
    this.previewPending = false;
    this.previewPendingHint = false;
    this.clearExactPreviewWaiters();
    this.previewFailure = null;
    this.stalePreviewRetryAttempted = false;
    this.engine.clearPreviewBody();
    this.throttle.reset();
    if (this.trailingTimer) {
      clearTimeout(this.trailingTimer);
      this.trailingTimer = null;
    }
  }

  private cancelPreview(): void {
    // Cancel the exact-region Extrude preview session (tool switched / Esc).
    this.closePreviewSessions();
    // Extrude's ToFace / boolean-target picks are modal; this is their teardown
    // funnel, so it is where an un-exited one must give the camera back.
    this.clearToolHover();
    this.engine.setOrbitSuppressed(false);
    this.extrudeStoredParams = undefined;
    this.commitBodyUnsub?.();
    this.commitBodyUnsub = null;
    if (this.commitBodyTimer) {
      clearTimeout(this.commitBodyTimer);
      this.commitBodyTimer = null;
    }
    this.engine.hideExtrudePreview();
    this.engine.setPreviewTint("normal");
    this.extrude = extrudeInit();
    this.dragging = null;
  }

  private cancelFillet(): void {
    // Release any open edge-op lane session FIRST (Esc / tool switch / a stale-model
    // cancel all land here): every session is ended with endPreview(false) and its
    // candidate meshes dropped, so no lane session can leak past the arm.
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.deps.engine.setOrbitSuppressed(false);
    // R3 (offset-face precedent): the drag arrow is the ONE shared `DragHandle`, so
    // a cancel that left it visible would hand the next tool a floating arrow it
    // never asked for.
    this.engine.hideValueHandle();
    this.fillet = filletInit(); // carries edgeOp back to "Fillet"
    this.filletEdges = [];
    this.filletPreparedRevision = null;
    this.filletOutward = null;
    this.filletTangent = null;
    this.filletAxis = SCREEN_UP_AXIS;
    this.filletAxisSource = "screen";
    this.filletEditFeatureId = undefined;
    this.filletStoredParams = undefined;
    if (this.dragging === "fillet") this.dragging = null;
    toolChipStore.getState().clear();
    this.updateDebug(); // republish the now-idle phase (a tool switch has no other hook)
  }

  private cancelBoolean(): void {
    // Release any open Boolean lane session FIRST: this also restores the
    // target/tool bodies `setPreviewReplacedBodyIds` hid once a candidate mesh
    // landed (`closePreviewSessions` → `engine.clearPreviewBody` →
    // `restorePreviewHiddenBodies`), so an Esc mid-preview leaves the scene exactly
    // as it was before the pick — a no-op when boolean never opened one (onToolChange
    // already tore it down via `cancelPreview`, or the tool never reached `armed`).
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    this.boolean = booleanInit();
    this.booleanEditFeatureId = undefined;
    this.booleanStoredParams = undefined;
    toolChipStore.getState().clear();
  }

  private cancelShell(): void {
    // Release any open shell lane session FIRST (see cancelFillet).
    this.closePreviewSessions();
    this.previewArmHint = null;
    this.deps.engine.setOrbitSuppressed(false);
    this.shell = shellInit();
    this.shellFaces = [];
    this.shellEditFeatureId = undefined;
    this.shellStoredParams = undefined;
    if (this.dragging === "shell") this.dragging = null;
    toolChipStore.getState().clear();
    this.updateDebug(); // republish the now-idle phase (a tool switch has no other hook)
  }

  private cancelPattern(): void {
    this.deps.engine.hideGhostPreview();
    this.linear = linearPatternInit();
    this.circular = circularPatternInit();
    this.mirror = mirrorInit();
    this.patternEditFeatureId = undefined;
    this.patternResultPolicyVersion = undefined;
    this.patternFuseResult = false;
    toolChipStore.getState().clear();
  }

  private cancelRevolve(): void {
    // Release the shared lane FIRST (Esc / tool switch / a geometryToken bump under an
    // armed revolve all land here): every open session is ended with endPreview(false)
    // and its candidate meshes dropped, so no lane session leaks.
    this.closePreviewSessions();
    this.commitRevolveBodyUnsub?.();
    this.commitRevolveBodyUnsub = null;
    if (this.commitRevolveBodyTimer) {
      clearTimeout(this.commitRevolveBodyTimer);
      this.commitRevolveBodyTimer = null;
    }
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    this.engine.hideRevolvePreview();
    this.engine.setPreviewTint("normal");
    this.revolve = revolveInit();
    this.revolveProfile = null;
    this.revolveProfiles = [];
    this.revolveRegionIds = [];
    this.revolveRegionIdentityVersions = [];
    this.revolveAxis = null;
    this.revolveAxisLineId = null;
    this.revolveAxisCandidates = [];
    this.revolveEditFeatureId = undefined;
    this.revolveStoredParams = undefined;
    this.revolveArmedDown = false;
    if (this.dragging === "revolve") this.dragging = null;
    toolChipStore.getState().clear();
    this.updateDebug();
  }

  private cancelAll(): void {
    this.invalidateArm();
    this.commitGen++;
    this.cancelRegionPick();
    this.cancelPreview();
    this.cancelFillet();
    this.cancelBoolean();
    this.cancelRevolve();
    this.cancelShell();
    this.cancelOffsetFace();
    this.cancelHole();
    this.cancelPattern();
    this.cancelTransform();
    this.endDatumPick();
    this.cancelMeasure();
    toolChipStore.getState().clear();
    toolStore.setState({ phase: toolStore.getState().modelTool === "select" ? "idle" : "armed" });
    this.updateDebug();
  }

  dispose(): void {
    // Supersede any in-flight commit sequence: its awaits resume after teardown
    // (the barrier + body waits are released below) and must not touch dead state.
    this.commitGen++;
    // …and any in-flight ARM, for the same reason. An arm can be several awaits
    // deep (OffsetFace: one handshake round trip plus one promotion per face), and
    // a disposed controller's continuation would otherwise finish arming and
    // publish tool state — including the `?vpdebug` surface — after teardown.
    this.invalidateArm();
    // Drop the datum tool's picker/ghost/chip: a viewport remount disposes the
    // controller mid-arm, and the chip store outlives it (it is a zustand store,
    // not engine state) — without this the chip would survive with dead handlers.
    this.endDatumPick();
    // Same reason as endDatumPick above: measureStore is a zustand store that
    // OUTLIVES this controller, so a viewport remount mid-measure would leave
    // orphaned labels anchored to a disposed engine.
    this.cancelMeasure();
    // Same again for the align sub-flow's hover: `selectionStore` outlives this
    // controller, so a remount mid-pick would leave a face tinted for ever.
    this.endAlign();
    // …and for the modal-pick hover, for the same reason. `setOrbitSuppressed` is
    // reasserted here too: the ENGINE outlives a controller remount, so a
    // dispose mid-pick would otherwise strand the camera unable to orbit.
    this.clearToolHover();
    this.deps.engine.setOrbitSuppressed(false);
    const c = this.deps.container;
    c.removeEventListener("pointerdown", this.onPointerDown);
    c.removeEventListener("pointermove", this.onPointerMove);
    c.removeEventListener("pointerup", this.onPointerUp);
    window.removeEventListener("keydown", this.onKeyDown, true);
    window.removeEventListener("keyup", this.onKeyUp, true);
    window.removeEventListener("pointerdown", this.onWindowPointerDown, true);
    window.removeEventListener("pointerup", this.onWindowPointerUp, true);
    if (this.trailingTimer) clearTimeout(this.trailingTimer);
    if (this.commitBodyTimer) clearTimeout(this.commitBodyTimer);
    if (this.commitRevolveBodyTimer) clearTimeout(this.commitRevolveBodyTimer);
    this.commitBodyUnsub?.();
    this.commitRevolveBodyUnsub?.();
    // End every OPEN preview lane session (finding 6): a viewport remount disposes the
    // controller while a tool is armed — without this the lane sessions leak.
    // Fire-and-forget with a disposed-guard catch (the result is irrelevant post-dispose).
    this.closePreviewSessions();
    for (const u of this.unsubs) u();
    this.unsubs.length = 0;
  }
}

/** Human message from a rejected backend call (ApiError → JS Error message). */
function errMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const VEC_EPS = 1e-6;

function isVec3(v: unknown): v is Vec3 {
  return Array.isArray(v) && v.length === 3 && v.every((n) => typeof n === "number" && Number.isFinite(n));
}

function vecClose(a: Vec3, b: Vec3): boolean {
  return Math.abs(a[0] - b[0]) < VEC_EPS && Math.abs(a[1] - b[1]) < VEC_EPS && Math.abs(a[2] - b[2]) < VEC_EPS;
}

/**
 * Match a stored direction Vec3 against a named world axis. The linear/circular
 * pattern FSM only ever offers X/Y/Z (`WORLD_AXIS`), so a fresh arm always commits
 * one of exactly those three; a re-edit that finds anything else means the record
 * was authored outside this arm path (or is corrupt) and must refuse rather than
 * silently snap to X.
 */
function matchWorldAxis(v: unknown): PatternAxis | null {
  if (!isVec3(v)) return null;
  for (const axis of ["X", "Y", "Z"] as const) {
    if (vecClose(v, WORLD_AXIS[axis])) return axis;
  }
  return null;
}

/** Mirror counterpart of `matchWorldAxis` for the mirror plane normal (`WORLD_PLANE_NORMAL`). */
function matchWorldPlane(v: unknown): MirrorPlane | null {
  if (!isVec3(v)) return null;
  for (const plane of ["XY", "XZ", "YZ"] as const) {
    if (vecClose(v, WORLD_PLANE_NORMAL[plane])) return plane;
  }
  return null;
}

/** Match a stored boolean `operation` against the three the chip offers — an
 *  unknown token refuses the re-edit rather than defaulting to Union. */
function matchBooleanOperation(v: unknown): BooleanOperation | null {
  return v === "Union" || v === "Cut" || v === "Intersect" ? v : null;
}

/** The numeric `.value` of a wire `Scalar` object (`{value, expr?}`), or `undefined`
 *  if `v` is not one (mirrors `beginExtrudeFeatureEdit`'s inline distance extraction). */
function scalarNumber(v: unknown): number | undefined {
  if (v && typeof v === "object" && "value" in (v as Record<string, unknown>)) {
    const n = (v as { value: unknown }).value;
    if (typeof n === "number" && Number.isFinite(n)) return n;
  }
  return undefined;
}

/**
 * The `lineId` of a stored `AxisRef` (`{kind:"sketchLine", sketchId, lineId}`, SCHEMA
 * §7.3) — the axis a revolve re-edit must re-render, not re-guess. Anything else
 * (absent axis, an `edge` variant the revolve tool never authors, a malformed value)
 * yields `null`, which the caller treats as "no stored axis", NOT as a licence to
 * substitute a different line.
 */
function axisLineIdFromParams(stored: Record<string, unknown> | undefined): string | null {
  const axis = stored?.axis;
  if (!axis || typeof axis !== "object") return null;
  const { kind, lineId } = axis as { kind?: unknown; lineId?: unknown };
  return kind === "sketchLine" && typeof lineId === "string" && lineId.length > 0 ? lineId : null;
}

/** True when a keyboard event targets a text field (skip the capture-Enter commit). */
function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

/**
 * World centroid the drag handle + chip anchor to. A single profile keeps its area
 * centroid (byte-identical single-region); N profiles use the combined (u,v) bbox
 * midpoint so the ONE handle sits between all the prisms (Wave 2 multi-select).
 */
function combinedCentroidWorld(plane: SketchPlane, profiles: PrismProfile[]): Vec3 {
  if (profiles.length === 0) return [0, 0, 0];
  if (profiles.length === 1) {
    const b = profileBounds(profiles[0]);
    const c = planePointToWorld(plane, { x: b.centroidU, y: b.centroidV });
    return [c.x, c.y, c.z];
  }
  let minU = Infinity;
  let maxU = -Infinity;
  let minV = Infinity;
  let maxV = -Infinity;
  for (const p of profiles) {
    const b = profileBounds(p);
    if (b.minU < minU) minU = b.minU;
    if (b.maxU > maxU) maxU = b.maxU;
    if (b.minV < minV) minV = b.minV;
    if (b.maxV > maxV) maxV = b.maxV;
  }
  const c = planePointToWorld(plane, { x: (minU + maxU) / 2, y: (minV + maxV) / 2 });
  return [c.x, c.y, c.z];
}

/**
 * A point KNOWN to be inside `profile`, in world space — the centroid of its
 * largest cap triangle.
 *
 * The bbox centroid the arrow is anchored at is fine as a visual anchor but wrong
 * as a probe origin: for an annulus it lands in the hole, and across two regions
 * it lands in the gap between them. A triangle of the region's own triangulation
 * cannot be outside the region, and the largest one is the most robust against
 * slivers.
 */
function profileSampleWorld(plane: SketchPlane, profile: PrismProfile): Vec3 | null {
  const { positions, indices } = profile.cap;
  let bestArea = -1;
  let best: { u: number; v: number } | null = null;
  for (let i = 0; i + 2 < indices.length; i += 3) {
    const [a, b, c] = [indices[i], indices[i + 1], indices[i + 2]];
    const ax = positions[a * 2];
    const ay = positions[a * 2 + 1];
    const bx = positions[b * 2];
    const by = positions[b * 2 + 1];
    const cx = positions[c * 2];
    const cy = positions[c * 2 + 1];
    const area = Math.abs((bx - ax) * (cy - ay) - (cx - ax) * (by - ay));
    if (area > bestArea) {
      bestArea = area;
      best = { u: (ax + bx + cx) / 3, v: (ay + by + cy) / 3 };
    }
  }
  if (!best) return null;
  const w = planePointToWorld(plane, { x: best.u, y: best.v });
  return [w.x, w.y, w.z];
}

/** Prefer a probe that STARTS inside material; otherwise the nearer surface. */
function strongerProbe(a: SideProbe | null, b: SideProbe | null): SideProbe | null {
  if (!a) return b;
  if (!b) return a;
  if (a.inside !== b.inside) return a.inside ? a : b;
  return b.gap < a.gap ? b : a;
}

/** World centroid over N pickable regions (the region-select chip anchor). */
function regionsCentroidWorld(plane: SketchPlane, regions: SketchRegion[]): Vec3 {
  const profiles = regions
    .map((r) => profileFromRegion(r))
    .filter((p): p is PrismProfile => p !== null);
  return combinedCentroidWorld(plane, profiles);
}

/** Perpendicular distance from a plane point (u,v) to the segment a→b. */
function distPointSeg(u: number, v: number, a: [number, number], b: [number, number]): number {
  const vx = b[0] - a[0];
  const vy = b[1] - a[1];
  const wx = u - a[0];
  const wy = v - a[1];
  const c2 = vx * vx + vy * vy;
  let t = c2 > 0 ? (vx * wx + vy * wy) / c2 : 0;
  t = Math.max(0, Math.min(1, t));
  return Math.hypot(u - (a[0] + t * vx), v - (a[1] + t * vy));
}

/** A deterministic axis just left of a profile (re-edit fallback when no line exists). */
function fallbackAxis(ring: [number, number][]): LatheAxis {
  let minU = Infinity;
  let minV = Infinity;
  let maxV = -Infinity;
  for (const [u, w] of ring) {
    if (u < minU) minU = u;
    if (w < minV) minV = w;
    if (w > maxV) maxV = w;
  }
  const x = minU - 1;
  return { a: [x, minV], b: [x, maxV] };
}
