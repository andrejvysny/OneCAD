/*
 * SketchController — imperative glue between the sketch tool machines, the
 * ViewportEngine, the CadClient (mock solver) and the stores (F-WP6).
 *
 * Lives inside ViewportRoot (created after the engine initializes, so it never
 * runs in jsdom where WebGL is absent — component tests keep the seeded chrome).
 * Responsibilities:
 *   - enter/exit sketch mode (client.enterSketch/finishSketch/cancelSketch,
 *     ortho + plane-normal camera, DOF/status → stores),
 *   - translate container pointer events → tool-machine events (click/move/esc)
 *     against snapped plane coords,
 *   - drive the rubber-band preview, snap indicator + hint, and the H/V ghost,
 *   - on commit: id-assign → auto-constrain → sketchUpsert round-trip → refresh
 *     geometry + DOF badges.
 */
import type { CadClient } from "@/ipc/client";
import type {
  EnterSketchTarget,
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchSession,
  SketchSolveStatus,
  SketchUpsertResult,
} from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { chooseGridStep } from "@/viewport/engine/GridPlane";
import { toolStore } from "@/stores/toolStore";
import { viewportStore, type Projection, type StatusSeverity } from "@/stores/viewportStore";
import {
  documentStore,
  docSketchStatus,
  nextSketchName,
  type SketchMeta,
} from "@/stores/documentStore";
import { selectionStore, topoRefId } from "@/stores/selectionStore";
import { sketchSelectionStore, sameSketchSel, type SketchSel } from "@/stores/sketchSelectionStore";
import { settingsStore } from "@/stores/settingsStore";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { applySolvedPositions } from "@/ipc/sketchWireMap";
import { promoteOne } from "@/ipc/promote";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { buildSnapCache, computeSnap, SNAP_PX, type SnapCandidateCache, type SnapResult } from "./snapEngine";
import { inferConstraints, inferHV, entityPoints } from "./autoConstrain";
import {
  commitDimensionConstraint,
  enqueueSketchMutation,
  filletSketchCorner,
  flushSketchMutations,
  lockedEntityIds,
  offsetSketchChain,
  rejectConflictHint,
  trimEntity,
  LOCKED_GEOMETRY_HINT,
} from "./sketchService";
import { filletGeometry, findFilletCorner, sharedCorner, type FilletFailure } from "./sketchFilletMath";
import { buildChain, chainSideSign, offsetChain, type Chain, type OffsetFailure } from "./offsetMath";
import { formatLength, lengthSuffix } from "@/units/format";
import type { SketchSnapshot } from "@/stores/sketchStore";
import { hitTestSketch } from "./sketchHitTest";
import { clickSelection, dragIntent, shouldApplyDrag, type DragIntent } from "./selectGesture";
import {
  marqueeHits,
  marqueeModeFor,
  marqueeRectFrom,
  type MarqueeMode,
  type MarqueeRect,
} from "./marqueeSelect";
import { mirrorEntities } from "./mirrorMath";
import { trimPreview, entityToDraft } from "./trimMath";
import { trace } from "@/debug/trace";
import { logDebug, logError } from "@/debug/log";
import {
  dimensionInit,
  dimensionStep,
  buildDimensionConstraint,
  dimensionSuffix,
  isConflictStatus,
  pickDimensionTarget,
  type DimState,
} from "./dimensionTool";
import {
  DEFAULT_POLYGON_SIDES,
  draftToEntityFields,
  resolveToolConstraints,
  type DraftEntity,
  type ToolConstraintSpec,
  type ToolMachine,
  type ToolState,
  type ToolStep,
} from "./toolMachine";
import {
  dimQuantum,
  liveDimInit,
  liveDimStep,
  type DimFieldId,
  type DimLocks,
  type DimQuantum,
  type LiveDimEvent,
  type LiveDimState,
  type ToolDimension,
} from "./liveDimension";
import {
  liveDimChipId,
  liveDimStore,
  placementFor,
  type LiveDimAnchors,
  type LiveDimChipField,
  type LiveDimHandlers,
  type LiveDimPlacement,
} from "@/stores/liveDimStore";
import {
  dedupeDimConstraints,
  dimConstraintSpecs,
  resolveDimConstraints,
} from "./liveDimConstraints";
import { LIVE_TOOL_MACHINES } from "./liveToolMachines";

const DRAG_PX = 4;

/**
 * Log a sketch-tool step that actually PRODUCED something (committed geometry or
 * a finished gesture) — DEV-OBSERVABILITY Wave F.
 *
 * Deliberately called from the discrete sites only (click, polygon side count).
 * The pointer-MOVE site steps the same machines at rAF frequency and must never
 * reach the log lane, which is why this is not folded into `machine.step`.
 */
function logSketchStep(toolId: string, event: string, stepped: ToolStep): void {
  const committed = stepped.committed?.length ?? 0;
  if (committed === 0 && stepped.done !== true) return;
  logDebug("fsm", `sketch/${toolId}: ${event} committed ${committed}`, {
    tool: toolId,
    event,
    committed,
    done: stepped.done === true,
    kinds: stepped.committed?.map((c) => c.type),
  });
}

/** Per-tool container cursor (U8). Draw tools show a crosshair for aiming; pick
 *  tools (select/trim/mirror) keep the default arrow — omitted here since the
 *  `?? "default"` fallback covers them identically. */
const CURSOR_BY_TOOL: Record<string, string> = {
  line: "crosshair",
  rect: "crosshair",
  centerRect: "crosshair",
  circle: "crosshair",
  ellipse: "crosshair",
  arc: "crosshair",
  polygon: "crosshair",
  slot: "crosshair",
  point: "crosshair",
  dimension: "crosshair",
};

/** Digit keys the polygon tool consumes as a side count (W2-C). */
const POLYGON_SIDES_KEY = /^[3-9]$/;

/** Keys that OPEN a live dimension chip when typed over the viewport (SP-1 W3).
 *  `-` and `.` are in here so "-30" and ".5" can be typed from the first stroke. */
const LIVE_DIM_TYPE_KEY = /^[0-9.\-]$/;

/**
 * Per-tool status hint while a draw tool is armed but idle, naming the live
 * dimension affordance. Shown only while `show.liveDimensions` is on — the line
 * promises something the pref can turn off. Polygon has its own (side count).
 */
const DRAW_TOOL_HINT: Record<string, string> = {
  line: "Line — click to place · type a number for length · Tab for angle",
  rect: "Rectangle — click a corner · type a number for width · Tab for height",
  centerRect: "Center rectangle — click the centre · type a number for width · Tab for height",
  circle: "Circle — click the centre · type a number for diameter · Tab for radius",
  arc: "Arc — click the centre · type a number for radius",
  ellipse: "Ellipse — click the centre · type a number for the major axis",
  slot: "Slot — click to place · type a number for length · Tab for angle",
};

/** Seed parameters for the WP-C T2b sketch edit tools, in document mm. Sticky
 *  per session through the tool's chip; re-seeded on a fresh session. */
const DEFAULT_FILLET_RADIUS = 5;
const DEFAULT_OFFSET_DISTANCE = 5;

/** The plane-pick phase's own prompt (also the fallback line every refusal rides). */
const PLANE_PICK_PROMPT = "Select a plane to start the sketch — Esc to cancel";

/**
 * A model face offered as a sketch host — from a selection ref, a plane-pick
 * click, or a viewport double-click. Deliberately NOT `PickHit`: the controller
 * must not depend on Three.js types, and the three producers carry the anchor in
 * three different shapes.
 */
export interface FacePickTarget {
  bodyId: string;
  /** Snapshot-scoped evidence (`"f:22"`) — the rung that resolves a fresh pick. */
  topoKey?: string;
  /** Persistent Rust-minted id when already promoted; else promoted on demand. */
  elementId?: string;
  worldPoint?: [number, number, number];
}

/**
 * The id of the NEWEST sketch hosted on `elementId`, or null.
 *
 * Matching is on the ElementId alone: it is Rust-minted and globally unique, and
 * deliberately does NOT embed a BodyId (the identity rule), so a body check would
 * be redundant. "Newest" is the LAST entry in projection iteration order.
 *
 * CAVEAT (flagged, not fixed): the real backend projects sketches from a
 * `BTreeMap` keyed by SketchId, so that order is uuid-lexicographic, not creation
 * order — with TWO sketches on the SAME face the "newest" picked here is
 * arbitrary-but-deterministic rather than genuinely newest. Both are valid hosts,
 * so the failure mode is "re-entered the other coincident sketch", never a wrong
 * bind. The mock lane inserts in creation order, so it is literally newest there.
 */
export function newestSketchOnFace(
  sketches: Record<string, SketchMeta>,
  elementId: string,
): string | null {
  let found: string | null = null;
  for (const s of Object.values(sketches)) {
    if (s.hostFace?.elementId === elementId) found = s.id;
  }
  return found;
}

export interface SketchControllerDeps {
  engine: ViewportEngine;
  client: CadClient;
  container: HTMLElement;
}

/**
 * Everything the QUEUED commit turn needs to know about the live dimensions of
 * the gesture that produced it — captured BY VALUE at the click, BEFORE the
 * machine step consumes the anchors and before the locks are cleared.
 *
 * The queued turn must never read `this.machineState` / `this.liveDim`: a fast
 * click burst (polyline) settles each commit one turn later, by which time those
 * fields already describe a LATER gesture, and the constraints would be authored
 * against the wrong phase. (`lastChainLineId` is the deliberate exception — it is
 * written only by the commit turn itself, so reading it there is what makes it
 * see the segment that actually settled.)
 */
interface DimCommitContext {
  toolId: string;
  /** Gesture anchors BEFORE the committing click (they identify the phase and
   *  carry the previous chain segment's direction). */
  anchors: Point2[];
  locks: DimLocks;
  /** Polygon side count — also the batch index of its construction circumcircle. */
  sides?: number;
}

export class SketchController {
  // Set FIRST in dispose(); every rAF callback + queued write-back bails on it so a
  // late frame / settled RPC never touches a torn-down controller.
  private disposed = false;
  private machine: ToolMachine | null = null;
  private machineState: ToolState | null = null;
  private lastSnap: SnapResult | null = null;
  private altHeld = false;

  // Live dimensions (SP-1). `liveDim` holds the chip input FSM's state — focus,
  // the focused field's raw text, and the pinned values. Locks are
  // CONTROLLER-owned: cleared when a step commits / the gesture ends / the tool
  // changes / the session tears down, and deliberately kept across a
  // non-committing click so an arc's typed radius survives phase 1 → 2.
  private liveDim: LiveDimState = liveDimInit();
  // The descriptors currently on screen (Wave 3). Kept so a keystroke over the
  // VIEWPORT knows which field a digit opens without re-stepping the machine.
  private liveDims: ToolDimension[] = [];
  // Whether the chip set is open in the store. Mirrors `liveDimStore.open`, held
  // here so `syncLiveDims` can tell a first `show` from a per-move `update`.
  private liveDimsOpen = false;
  private liveDimPlacement: LiveDimPlacement = "tr";
  // The last Line this chain committed — the only entity an ABSOLUTE typed angle
  // can be expressed against (`Angle` is relative). Written by the commit turn,
  // cleared whenever the chain it belongs to ends.
  private lastChainLineId: string | null = null;

  // Entity-derived snap candidates (guide/quadrant/intersection points), rebuilt
  // only when the session's entity array reference changes — session arrays are
  // replaced immutably on every commit, so `!==` is a valid, cheap invalidation
  // check that turns the O(n²) intersection scan from "every pointer move" into
  // "once per sketch edit" (perf; see snapAt).
  private snapCacheKey: SketchEntity[] | null = null;
  private snapCache: SnapCandidateCache | null = null;

  // Dimension tool (non-drawing): a pick-accumulator FSM + its open chip.
  private dimensionActive = false;
  private dimState: DimState = dimensionInit();
  private priorProjection: Projection | null = null;
  private entering = false;
  // A sketch→sketch retarget that landed while an enter/switch was still in flight
  // (latest-wins). Drained once the in-flight open settles.
  private pendingSwitchId: string | null = null;
  // Set ONLY while openSession writes activeSketchId itself: that write echoes the
  // session the controller just opened, it is never a user retarget.
  private selfActiveSketchWrite = false;
  // Plane-pick phase: bare sketch entry shows the plane picker; a click on a
  // quad creates the sketch on that plane and opens the normal session.
  private planePicking = false;
  // The picker's own prompt (with any refusal already folded in), so the
  // face-hover line below can be swapped in and then swapped back out without
  // losing the reason the picker is up at all.
  private planePickHint: { message: string; severity?: StatusSeverity } | null = null;
  // The body FACE hovered during the plane-pick phase (W3 trigger b). Held so the
  // hover highlight + prompt only churn on an actual change, and so every
  // teardown path (`endPlanePick`) can drop them.
  private faceHover: FacePickTarget | null = null;
  // A double-click face entry is in flight (W3 trigger c). Separate from
  // `entering`: the RE-ENTER branch flips the mode and must leave `entering`
  // clear so the resulting `enter()` actually opens the session.
  private faceEntryInFlight = false;

  private downX = 0;
  private downY = 0;
  private downButton = -1;
  private moved = false;
  private pendingMove: PointerEvent | null = null;
  private moveScheduled = false;
  // Idle-hover hit-test (select tool): rAF-coalesced so a fast mouse only raycasts
  // once per frame (P2). Carries the latest client point; the flag gates scheduling.
  private pendingHover: { x: number; y: number } | null = null;
  private hoverScheduled = false;

  // Trim / Mirror tools (non-drawing, click-only). Parallel lanes to `selectActive`,
  // each gated STRICTLY on its tool id so the select/draw/dimension paths are untouched.
  private trimActive = false;
  private mirrorActive = false;

  // Sketch Fillet / Offset (WP-C T2b) — click tools on the SAME lane as Trim /
  // Mirror (hover ghost + click applies, repeatable while armed). Each owns one
  // live parameter, held in its open chip rather than re-typed per apply.
  private filletActive = false;
  private offsetActive = false;
  private filletRadius = DEFAULT_FILLET_RADIUS;
  private offsetDistance = DEFAULT_OFFSET_DISTANCE;
  /** Fillet TWO-PICK path: the first line picked, awaiting its corner partner.
   *  Null while the one-click corner path is in play. */
  private filletFirstPick: string | null = null;

  // Select tool (non-drawing): click-select + point-handle drag via the gesture
  // lane. A parallel path to `dimensionActive`, gated STRICTLY on tool === "select".
  private selectActive = false;
  // Set on a pointerdown that lands on a draggable handle (orbit suppressed then);
  // the gesture only OPENS once the pointer passes DRAG_PX (else it collapses to a click).
  private dragArmed: DragIntent | null = null;
  private dragStarting = false; // beginGesture in flight (first move past DRAG_PX)
  private dragging = false; // gesture open (solveDrag lane live)
  private dragBase: SketchEntity[] = []; // pre-drag entities (latest-wins applies onto this)
  private dragPlane: SketchPlane | null = null;
  private dragStatus: SketchSolveStatus = "UnderConstrained";
  private dragLastSeq = 0; // highest applied solveDrag seq (stale-drop)
  // Worker SolveDrag positions are INCREMENTAL — each response carries only the
  // points that changed since the PREVIOUS response (SolverLane g.last_reported).
  // Accumulate them across the gesture; rendering each response alone onto
  // dragBase would snap earlier-moved coupled points back to their pre-drag pose.
  private dragAccum: Record<string, [number, number]> = {};
  private pendingTarget: [number, number] | null = null; // coalesced latest drag target
  private solveScheduled = false;
  // A pointerup / Esc that arrived while beginGesture was still in flight — its end
  // is deferred here so a fast flick never double-commits the gesture.
  private dragEndPending: { target?: [number, number]; restore: boolean } | null = null;

  // Marquee / box select (W2-D). `armed` on an empty-space LMB down (which CLAIMS the
  // gesture from LMB orbit); `active` once it passes DRAG_PX. Mutually exclusive with
  // `dragArmed` — a down either lands on a draggable handle or it does not.
  private marqueeArmed = false;
  private marqueeActive = false;
  private marqueeMode: MarqueeMode = "window";
  private marqueeEl: HTMLDivElement | null = null;
  private pendingMarquee: { x: number; y: number } | null = null;
  private marqueeScheduled = false;

  private readonly unsubs: Array<() => void> = [];

  constructor(private readonly deps: SketchControllerDeps) {
    const c = deps.container;
    c.addEventListener("pointerdown", this.onPointerDown);
    c.addEventListener("pointermove", this.onPointerMove);
    c.addEventListener("pointerup", this.onPointerUp);
    // Marquee safety net: a box drag that RELEASES outside the viewport (over a
    // floating panel, or off the window) never reaches the container's own pointerup,
    // which would strand the overlay AND leave LMB orbit suppressed. These run after
    // the container handler (bubble → window), so a normal in-viewport release has
    // already finished the marquee and they no-op.
    window.addEventListener("pointerup", this.onWindowPointerUp);
    window.addEventListener("pointercancel", this.onWindowPointerCancel);
    window.addEventListener("keydown", this.onKeyDown, true);
    window.addEventListener("keyup", this.onKeyUp, true);

    // React to mode + tool changes.
    let lastMode = toolStore.getState().mode;
    let lastTool = toolStore.getState().sketchTool;
    this.unsubs.push(
      toolStore.subscribe((s) => {
        if (s.mode !== lastMode) {
          lastMode = s.mode;
          if (s.mode === "sketch") void this.enter();
          else this.exit();
        }
        if (s.mode === "sketch" && s.sketchTool !== lastTool) {
          lastTool = s.sketchTool;
          this.selectMachine(s.sketchTool);
        }
      }),
    );

    // React to a sketch→sketch retarget (tree activate / chrome) while ALREADY in
    // sketch mode: the mode subscription above never fires for it, so without this
    // the chrome shows B while the controller keeps writing into A.
    let lastActiveSketchId = viewportStore.getState().activeSketchId;
    this.unsubs.push(
      viewportStore.subscribe((s) => {
        if (s.activeSketchId === lastActiveSketchId) return;
        lastActiveSketchId = s.activeSketchId;
        this.onActiveSketchChanged(s.activeSketchId);
      }),
    );

    // Enter immediately if we mount already in sketch mode (e.g. ?sketchdemo).
    if (toolStore.getState().mode === "sketch") void this.enter();
  }

  // ── enter / exit ──────────────────────────────────────────────────────────

  private async enter(): Promise<void> {
    if (this.entering) return;
    this.entering = true;
    sketchSelectionStore.getState().clear();
    try {
      // setMode('sketch') fires the mode subscription BEFORE it assigns
      // activeSketchId; yield one microtask so we read the real target.
      await Promise.resolve();
      if (toolStore.getState().mode !== "sketch") return;
      const activeId = viewportStore.getState().activeSketchId;
      // No target ⇒ bare "new sketch" intent. A selected model FACE is a
      // sketch plane too (MODEL-OPS W2), so honour it before falling back to
      // the three world quads — that is the whole point of sketch-on-face: a
      // part is built by sketching on what you already made.
      if (!activeId) {
        const faceEntry = await this.tryEnterOnSelectedFace();
        if (faceEntry.entered) return;
        // A REFUSAL rides into the picker prompt rather than being published
        // separately: beginPlanePick sets the status hint itself, so a hint
        // written before it would be silently overwritten (the same last-word-
        // wins trap `ModelToolController.resetToSelect` documents). A refused
        // FACE short-circuits the datum rung below — the face is the more
        // specific pick, so its reason is the one to show.
        if (faceEntry.refusal) {
          this.beginPlanePick(faceEntry.refusal);
          return;
        }
        // A selected DATUM plane is a sketch host too (DATUM W1) — the tree's
        // datum double-click selects one and flips the mode, and this is what
        // picks it up. Checked AFTER the face (a face is the more specific pick
        // when somehow both are selected) and BEFORE the world-plane picker.
        const datumEntry = await this.tryEnterOnSelectedDatum();
        if (datumEntry.entered) return;
        this.beginPlanePick(datumEntry.refusal);
        return;
      }
      const opened = await this.openSession(activeId);
      if (!opened) this.failOutOfSketchMode();
    } finally {
      this.entering = false;
      this.drainPendingSwitch();
    }
  }

  /** An existing-sketch open failed: fall back to model mode instead of stranding
   *  sketch chrome with no session. exit() clears the status hint, so re-set the
   *  failure message (set by openSession) after the mode flip. */
  private failOutOfSketchMode(): void {
    if (toolStore.getState().mode !== "sketch") return;
    const prev = viewportStore.getState().statusHint;
    toolStore.getState().setMode("model");
    if (prev) {
      viewportStore.getState().setStatusHint(prev.message, { severity: prev.severity, sticky: prev.sticky });
    } else {
      viewportStore.getState().setStatusHint(null);
    }
  }

  /**
   * `viewportStore.activeSketchId` moved while the controller is live.
   *
   * The decision is made ONLY against the controller's own OPEN SESSION, never
   * against a last-seen store value: `setMode` writes the id AFTER enter() has
   * already started and openSession echoes it back, so a last-seen diff would
   * self-switch on every normal entry.
   */
  private onActiveSketchChanged(id: string | null): void {
    if (id === null) return; // a mode exit cleared it — exit() owns that teardown
    if (this.selfActiveSketchWrite) return; // openSession echoing its own session
    if (toolStore.getState().mode !== "sketch") return;
    if (this.entering) {
      this.pendingSwitchId = id; // latest-wins; drained when the open settles
      return;
    }
    const openId = sketchStore.getState().session?.sketchId ?? null;
    if (openId === id) return; // already the open session
    // No session and no picker up ⇒ enter() owns this id, this is not a retarget.
    if (openId === null && !this.planePicking) return;
    void this.switchTo(id);
  }

  /**
   * Retarget the controller at another sketch WITHOUT leaving sketch mode.
   *
   * The backend holds ONE sketch-session slot, so A must be CLOSED before B opens —
   * hence the awaited cancel→finish (the same pair exit() uses: cancel squashes the
   * worker gesture + session, finish mints/refreshes A's `Sketch` timeline record).
   * The projection is deliberately NOT restored: sketch→sketch stays ortho and the
   * projection saved at the ORIGINAL entry must survive to the eventual exit.
   */
  private async switchTo(newId: string): Promise<void> {
    this.entering = true;
    try {
      const closing = this.teardownSession({ restoreProjection: false });
      sketchStore.getState().setSession(null);
      if (closing) {
        trace("sketch", `switch: close ${closing.sketchId} → open ${newId}`);
        await flushSketchMutations();
        try {
          await this.deps.client.cancelSketch(closing.sketchId);
          await this.deps.client.finishSketch(closing.sketchId);
        } catch (e) {
          logError("sketch", "switch: closing the previous sketch FAILED", {
            sketchId: closing.sketchId,
            error: e,
          });
        }
        // A is CLOSED now — whatever happens to this switch below. Its always-visible
        // static layer still holds the copy fetched at entry, and the only signal
        // SketchStaticSync diffs is the geometryToken (the MODE never changes across a
        // switch, so its sketch→model refetch never fires). The real lane happens to
        // bump the token via the projection; the mock lane publishes nothing, so stamp
        // it here — unconditionally, because the superseded/failed paths below leave A
        // just as closed and just as stale.
        documentStore.getState().bumpSketchGeometry(closing.sketchId);
      }
      // Superseded mid-close: a mode exit already tore everything down, or a newer
      // target landed (the drain below runs it instead).
      if (this.disposed || toolStore.getState().mode !== "sketch") return;
      if (this.pendingSwitchId !== null && this.pendingSwitchId !== newId) return;
      this.pendingSwitchId = null;
      const opened = await this.openSession(newId);
      if (!opened) this.failOutOfSketchMode(); // never strand chrome on a dead session
    } finally {
      this.entering = false;
      this.drainPendingSwitch();
    }
  }

  /** Run the retarget that arrived while an enter/switch was in flight. A target that
   *  matches the session that just opened is that open's OWN echo, not a user
   *  retarget — drop it. */
  private drainPendingSwitch(): void {
    const pending = this.pendingSwitchId;
    this.pendingSwitchId = null;
    if (pending === null || this.disposed) return;
    if (pending === sketchStore.getState().session?.sketchId) return;
    if (toolStore.getState().mode !== "sketch") return;
    void this.switchTo(pending);
  }

  /** openSession's own activeSketchId write, bracketed so the retarget subscription
   *  reads it as an echo rather than a user switch. */
  private setActiveSketchSelf(id: string): void {
    this.selfActiveSketchWrite = true;
    try {
      viewportStore.getState().setActiveSketch(id);
    } finally {
      this.selfActiveSketchWrite = false;
    }
  }

  /** Enter the client session for `target` and wire it into the stores + engine.
   *  Returns false when the client rejected (session not opened). */
  private async openSession(target: EnterSketchTarget): Promise<boolean> {
    let session: SketchSession;
    try {
      session = await this.deps.client.enterSketch(target);
    } catch (e) {
      logError("sketch", "enterSketch FAILED", { target, error: e });
      viewportStore.getState().setStatusHint(`Enter sketch failed: ${sketchErr(e)}`, { severity: "error", sticky: true });
      return false;
    }
    if (toolStore.getState().mode !== "sketch") {
      // The user Esc'd / left model-side during the deferred enter: the backend
      // session is live but there is no UI to drive it. Cancel it, and — for a
      // FRESH create (plane pick, target is not an existing id) — also delete the
      // empty sketch it just minted so it doesn't orphan in the tree. (Creations
      // are serialized by the controller's `entering` flag, so this fires once.)
      void this.deps.client.cancelSketch(session.sketchId);
      if (typeof target !== "string") {
        void this.deps.client
          .deleteSketch(session.sketchId)
          .catch((e: unknown) =>
            logError("sketch", "orphan cleanup FAILED", { sketchId: session.sketchId, error: e }),
          );
      }
      return true; // the session opened; the user just left
    }

    // A freshly created sketch (plane pick) isn't in the tree yet — register it,
    // then make it the active + selected sketch so the chrome + inspector bind.
    this.setActiveSketchSelf(session.sketchId);
    const sketches = documentStore.getState().sketches;
    if (!sketches[session.sketchId]) {
      documentStore.getState().addSketch({
        id: session.sketchId,
        name: nextSketchName(sketches),
        visible: true,
        dof: session.dof,
        status: docSketchStatus(session.status),
        geometryToken: `pending:${session.sketchId}`,
        // The HOST the session was just opened against. Optimistic exactly like
        // the three fields above it: the real lane's next `projection-updated`
        // replaces the whole row with backend truth (`SketchDto.hostFace`), and
        // the mock lane has no projection stream, so this row IS its projection —
        // which is what lets the mock-lane double-click re-entry find the sketch.
        ...(typeof target !== "string" && "newOnFace" in target
          ? {
              hostFace: {
                bodyId: target.newOnFace.bodyId,
                elementId: target.newOnFace.elementId,
              },
            }
          : {}),
      });
    }
    selectionStore.getState().set([{ kind: "sketch", id: session.sketchId }]);

    sketchStore.getState().setSession(session);
    sketchStore.getState().clearSketchUndo(); // fresh session ⇒ no carried-over history
    sketchStore.getState().setConflicting(session.conflicting ?? []); // seed from the enter solve
    this.pushSolve(session.sketchId, session.dof, session.status);

    // Capture the pre-sketch projection ONCE per sketch-mode visit: a sketch→sketch
    // switch re-opens a session while already ortho, and overwriting here would make
    // the eventual exit "restore" ortho instead of the user's real projection.
    this.priorProjection ??= viewportStore.getState().projection;
    this.deps.engine.enterSketch(session.plane, session.entities, session.status);
    viewportStore.getState().setProjection("ortho");

    this.selectMachine(toolStore.getState().sketchTool);
    return true;
  }

  /**
   * If a single model FACE is selected, start the sketch ON it.
   *
   * The basis comes from the BACKEND (`faceSketchPlane` → the kernel's own face
   * descriptor + the lock-tested in-plane axis rule), never from the picked
   * triangle's normal: the frame is frozen with the sketch and every entity
   * coordinate is expressed in it, so it has to be authoritative and
   * replay-stable. A non-planar face is refused by the backend and reported,
   * rather than sketching on an approximated plane.
   *
   * `entered: false` means the caller falls through to the world-plane picker;
   * `refusal` (when set) is the reason, for the picker prompt to CARRY. A refusal
   * published as its own status hint here would be overwritten the moment
   * `beginPlanePick` writes its prompt (the last-word-wins trap the datum path
   * already documents), so the user would be told nothing at all — which is
   * exactly what a "non-planar face" pick used to do.
   */
  private async tryEnterOnSelectedFace(): Promise<{ entered: boolean; refusal?: string }> {
    const faces = selectionStore.getState().selected.filter((r) => r.kind === "face");
    if (faces.length !== 1) return { entered: false };
    const face = faces[0];
    const bodyId = face.bodyId;
    if (!bodyId) return { entered: false };

    const resolved = await this.resolveFaceHost({
      bodyId,
      topoKey: face.topoKey,
      elementId: face.elementId,
      worldPoint: face.anchor?.worldPoint,
    });
    // An unidentifiable pick falls through SILENTLY, exactly as before: the user
    // asked for "a sketch", not for this face, so the picker is the answer.
    if (resolved.kind === "unresolved") return { entered: false };
    if (resolved.kind === "refused") return { entered: false, refusal: resolved.reason };
    if (toolStore.getState().mode !== "sketch") return { entered: false };

    const opened = await this.openSession({ newOnFace: resolved.target, plane: resolved.plane });
    return { entered: opened };
  }

  /**
   * Promote (if needed) + PREFLIGHT one picked face into an `enterSketch` target.
   *
   * The single funnel all three entry triggers share (selected face, plane-pick
   * click, viewport double-click), so the identity rules live in exactly one
   * place. Pure resolution — it opens nothing and touches no store, which is what
   * lets the double-click path validate a face BEFORE flipping the mode.
   *
   * `faceSketchPlane` is validation-only here; the frame it returns is the
   * BACKEND's (the kernel's own face descriptor + the lock-tested in-plane axis
   * rule), carried through by reference and never rebuilt on this side. A
   * non-planar face comes back `refused` rather than sketched on approximately.
   */
  private async resolveFaceHost(
    pick: FacePickTarget,
  ): Promise<
    | { kind: "ok"; target: { bodyId: string; elementId: string; topoKey?: string; worldPoint?: [number, number, number] }; plane: SketchPlane }
    | { kind: "refused"; reason: string }
    | { kind: "unresolved" }
  > {
    // A pick is promoted to a minted ElementId by ViewportRoot; if that has not
    // landed yet, promote here rather than sending a snapshot-scoped TopoKey the
    // attachment could not survive an edit with.
    const elementId = await this.promoteFace(pick);
    if (!elementId) return { kind: "unresolved" };

    // `topoKey` rides along: this id was just minted (never consumed by an op),
    // so it is genuinely absent from the worker's on-demand element-map
    // partition — the topoKey rung is what makes THIS flow resolve at all.
    let plane: SketchPlane;
    try {
      plane = await this.deps.client.faceSketchPlane(pick.bodyId, elementId, pick.topoKey);
    } catch (e) {
      return { kind: "refused", reason: `Cannot sketch on that face: ${sketchErr(e)}` };
    }
    return {
      kind: "ok",
      // `topoKey` rides through to `add_sketch_on_face` for the SAME reason it
      // rides to `faceSketchPlane` above — it is the rung that actually resolves
      // a freshly promoted, never-consumed ElementId.
      target: {
        bodyId: pick.bodyId,
        elementId,
        topoKey: pick.topoKey,
        worldPoint: pick.worldPoint,
      },
      plane,
    };
  }

  /** The pick's persistent ElementId, promoting the TopoKey on demand. */
  private async promoteFace(pick: FacePickTarget): Promise<string | undefined> {
    if (pick.elementId) return pick.elementId;
    if (!pick.topoKey) return undefined;
    const promoted = await promoteOne(this.deps.client, pick.bodyId, {
      topoKey: pick.topoKey,
      anchor: pick.worldPoint ? { worldPoint: pick.worldPoint } : undefined,
    });
    return promoted?.elementId;
  }

  /**
   * Double-click a model FACE in model mode → sketch on it (W3 trigger c).
   *
   * Re-entry wins over creation: a face that ALREADY hosts a sketch opens that
   * sketch (the newest — see `newestSketchOnFace`) through the same
   * `setMode('sketch', id)` path the static-sketch double-click uses, rather than
   * stacking a second identical projected boundary on the same plane.
   *
   * A fresh create validates BEFORE it flips the mode, so a non-planar face
   * reports and leaves the user exactly where they were — no sketch chrome, no
   * plane picker, no mode change.
   */
  async enterOnFace(pick: FacePickTarget): Promise<void> {
    if (this.faceEntryInFlight || this.entering || this.disposed) return;
    if (toolStore.getState().mode !== "model") return; // model-mode gesture only
    this.faceEntryInFlight = true;
    try {
      // ONE promotion for both branches: the re-entry match is by ElementId, and
      // a second promote for the create branch would be a redundant round-trip.
      const elementId = await this.promoteFace(pick);
      if (this.disposed || toolStore.getState().mode !== "model") return;
      if (!elementId) {
        viewportStore
          .getState()
          .setStatusHint("Cannot sketch on that face: it could not be identified", { severity: "error" });
        return;
      }
      const existing = newestSketchOnFace(documentStore.getState().sketches, elementId);
      if (existing) {
        // Deliberately NOT guarded by `entering`: the mode flip is what makes
        // `enter()` open the session, and `enter()` no-ops while it is set.
        toolStore.getState().setMode("sketch", existing);
        return;
      }
      const resolved = await this.resolveFaceHost({ ...pick, elementId });
      if (this.disposed || toolStore.getState().mode !== "model") return;
      if (resolved.kind !== "ok") {
        viewportStore.getState().setStatusHint(
          resolved.kind === "refused"
            ? resolved.reason
            : "Cannot sketch on that face: it could not be identified",
          { severity: "error" },
        );
        return; // model mode untouched — the double-click simply did nothing
      }
      this.entering = true; // `enter()` no-ops on the mode flip below; we own the open
      try {
        sketchSelectionStore.getState().clear();
        toolStore.getState().setMode("sketch");
        const opened = await this.openSession({ newOnFace: resolved.target, plane: resolved.plane });
        if (!opened) this.failOutOfSketchMode();
      } finally {
        this.entering = false;
        this.drainPendingSwitch();
      }
    } finally {
      this.faceEntryInFlight = false;
    }
  }

  /**
   * If a single DATUM plane is selected, start the sketch ON it (DATUM W1).
   *
   * The basis is read straight off the projection (`DatumMeta.plane`), which the
   * BACKEND resolved — never re-derived here from `basePlaneId + offset`. The
   * core stamps the sketch with exactly that frame, so a second derivation on
   * this side would be a competing source of truth for the sketch's own
   * coordinate system.
   *
   * An UNRESOLVED datum (`resolvedValid: false`) is refused with a `refusal`
   * message rather than sketched on: its cached frame is a meaningless identity
   * plane, and silently placing a sketch there is worse than saying no.
   *
   * `entered: false` means the caller falls through to the world-plane picker;
   * `refusal` (when set) is the reason, for the picker prompt to carry.
   */
  private async tryEnterOnSelectedDatum(): Promise<{ entered: boolean; refusal?: string }> {
    const refs = selectionStore.getState().selected.filter((r) => r.kind === "datum");
    if (refs.length !== 1) return { entered: false };
    const datum = documentStore.getState().datums[refs[0].id];
    if (!datum) return { entered: false };
    if (!datum.resolvedValid) {
      return { entered: false, refusal: `Cannot sketch on ${datum.name}: the datum did not resolve` };
    }
    const entered = await this.openSession({
      newOnDatum: { datumId: datum.id },
      plane: datum.plane,
    });
    return { entered };
  }

  /** Show the plane picker and prompt; a quad click resolves via confirmPlanePick.
   *  `refusal` prefixes the prompt when the picker is a FALLBACK from a rejected
   *  host (an unresolved datum), so the reason is not lost to this prompt. */
  private beginPlanePick(refusal?: string): void {
    this.planePicking = true;
    // Default arrow while picking (a tool preserved across an auto-switch entry
    // may have set a drawing cursor before the picker appeared).
    this.deps.container.style.cursor = "default";
    this.deps.engine.setPlanePickerVisible(true);
    this.showPlanePickPrompt(refusal);
  }

  /** Publish the pick-phase prompt (with `refusal` folded in) AND remember it, so
   *  a transient face-hover line can be swapped in and then restored verbatim. */
  private showPlanePickPrompt(refusal?: string): void {
    const message = refusal ? `${refusal}. ${PLANE_PICK_PROMPT}` : PLANE_PICK_PROMPT;
    const severity = refusal ? ("error" as const) : undefined;
    this.planePickHint = { message, severity };
    viewportStore.getState().setStatusHint(message, { sticky: true, ...(severity ? { severity } : {}) });
  }

  /** Reverse beginPlanePick (idempotent): hide the picker + clear its hint. */
  private endPlanePick(): void {
    if (!this.planePicking) return;
    this.planePicking = false;
    this.setFaceHover(null); // before the hint clear below — it would restore the prompt
    this.planePickHint = null;
    this.deps.engine.setPlanePickerVisible(false);
    this.deps.engine.setDatumHover(null); // datums stay visible; drop the pick tint
    viewportStore.getState().setStatusHint(null);
  }

  /**
   * Hover (or un-hover) a body FACE during the plane-pick phase (W3 trigger b).
   *
   * The highlight goes through `selectionStore.setHover` rather than
   * `engine.setHighlightState` directly: ViewportRoot already subscribes the
   * store to the highlight layer, and a second writer would be clobbered by the
   * next selection event. One writer, the standard face tint.
   *
   * NO planarity check here — that costs a backend round-trip, and paying one per
   * pointer move to grey out a face is not worth it. The CLICK validates.
   */
  private setFaceHover(pick: FacePickTarget | null): void {
    const sameKey = this.faceHover?.bodyId === pick?.bodyId && this.faceHover?.topoKey === pick?.topoKey;
    if (sameKey) return;
    this.faceHover = pick;
    const sel = selectionStore.getState();
    if (!pick || !pick.topoKey) {
      sel.setHover(null);
      if (this.planePickHint) {
        viewportStore.getState().setStatusHint(this.planePickHint.message, {
          sticky: true,
          ...(this.planePickHint.severity ? { severity: this.planePickHint.severity } : {}),
        });
      }
      return;
    }
    sel.setHover({
      kind: "face",
      id: topoRefId(pick.bodyId, pick.topoKey),
      bodyId: pick.bodyId,
      topoKey: pick.topoKey,
      elementId: pick.elementId,
      ...(pick.worldPoint ? { anchor: { worldPoint: pick.worldPoint } } : {}),
    });
    const name = documentStore.getState().bodies[pick.bodyId]?.name ?? pick.bodyId;
    viewportStore
      .getState()
      .setStatusHint(`Face of ${name} — click to sketch on it`, { sticky: true });
  }

  /** A body FACE was clicked during the plane pick: open a sketch on it (W3).
   *  Mirrors confirmDatumPick — a REFUSED face leaves the picker up, carrying the
   *  reason in the prompt, rather than dropping the user into nothing. */
  private async confirmFacePick(pick: FacePickTarget): Promise<void> {
    if (this.entering) return; // a double-click must not create two sketches
    this.entering = true;
    try {
      const resolved = await this.resolveFaceHost(pick);
      if (this.disposed || toolStore.getState().mode !== "sketch") return;
      if (resolved.kind !== "ok") {
        this.setFaceHover(null);
        this.showPlanePickPrompt(
          resolved.kind === "refused"
            ? resolved.reason
            : "Cannot sketch on that face: it could not be identified",
        );
        return; // stay in the pick phase — the world quads are still on screen
      }
      this.endPlanePick();
      const opened = await this.openSession({ newOnFace: resolved.target, plane: resolved.plane });
      if (!opened && toolStore.getState().mode === "sketch") {
        // Re-show the quads but keep the failure hint visible (don't overwrite
        // the statusHint openSession just set).
        this.planePicking = true;
        this.deps.engine.setPlanePickerVisible(true);
      }
    } finally {
      this.entering = false;
      this.drainPendingSwitch();
    }
  }

  /** A DATUM plane was clicked during the plane pick: open a sketch hosted on it.
   *  Mirrors confirmPlanePick (including the re-show-on-failure recovery). */
  private async confirmDatumPick(datumId: string): Promise<void> {
    if (this.entering) return; // a double-click must not create two sketches
    const datum = documentStore.getState().datums[datumId];
    if (!datum) return;
    if (!datum.resolvedValid) {
      viewportStore.getState().setStatusHint(
        `Cannot sketch on ${datum.name}: the datum did not resolve`,
        { severity: "error", sticky: true },
      );
      return; // stay in the pick phase — the world quads are still on screen
    }
    this.entering = true;
    try {
      this.endPlanePick();
      const opened = await this.openSession({
        newOnDatum: { datumId: datum.id },
        plane: datum.plane,
      });
      if (!opened && toolStore.getState().mode === "sketch") {
        this.planePicking = true;
        this.deps.engine.setPlanePickerVisible(true);
      }
    } finally {
      this.entering = false;
      this.drainPendingSwitch();
    }
  }

  /** A plane was clicked: leave pick mode and open a fresh sketch on it. On a
   *  client failure the picker comes back (with the error in the status bar) so
   *  the user can retry or Esc out — never stranded in pick chrome w/o quads. */
  private async confirmPlanePick(kind: PickablePlane): Promise<void> {
    if (this.entering) return; // a double-click must not create two sketches
    this.entering = true;
    try {
      this.endPlanePick();
      const opened = await this.openSession({ newOnPlane: kind });
      if (!opened && toolStore.getState().mode === "sketch") {
        // Re-show the quads but keep the failure hint visible (don't overwrite
        // the statusHint openSession just set).
        this.planePicking = true;
        this.deps.engine.setPlanePickerVisible(true);
      }
    } finally {
      this.entering = false;
      this.drainPendingSwitch();
    }
  }

  /**
   * Shared session teardown for exit() and switchTo(): drop every tool / preview /
   * drag lane and the engine's sketch overlay. Returns the session that was open and
   * leaves it in the store — the CALLER runs its own close sequence against it and
   * nulls it, so the ordering both paths need stays theirs.
   *
   * `restoreProjection` is false for a sketch→sketch switch (see switchTo).
   */
  private teardownSession(opts: { restoreProjection: boolean }): SketchSession | null {
    sketchSelectionStore.getState().clear();
    this.endPlanePick();
    this.machine = null;
    this.machineState = null;
    this.lastSnap = null;
    this.clearLiveDimGesture();
    this.snapCache = null;
    this.snapCacheKey = null;
    if (this.dimensionActive) this.cancelDimension();
    this.dimensionActive = false;
    this.resetDrag();
    this.cancelMarquee(); // before setSketchDrawingActive(false) below — idempotent
    this.selectActive = false;
    this.trimActive = false;
    this.mirrorActive = false;
    this.endSketchEditTool();
    this.filletActive = false;
    this.offsetActive = false;
    this.filletRadius = DEFAULT_FILLET_RADIUS;
    this.offsetDistance = DEFAULT_OFFSET_DISTANCE;
    this.deps.container.style.cursor = "";
    this.deps.engine.setSketchDrawingActive(false);
    this.deps.engine.setSketchPreview([]);
    this.deps.engine.setSketchSnap(null, false);
    this.deps.engine.setSketchGhost(null, null);
    this.deps.engine.setSketchTrimGhost(null);
    this.deps.engine.exitSketch();
    viewportStore.getState().setStatusHint(null);
    if (opts.restoreProjection && this.priorProjection) {
      viewportStore.getState().setProjection(this.priorProjection);
      this.priorProjection = null;
    }
    sketchStore.getState().clearSketchUndo();
    return sketchStore.getState().session;
  }

  private exit(): void {
    const session = this.teardownSession({ restoreProjection: true });
    if (session) {
      // EXTRUDE-COMMIT-FIX: every keep-exit must mint/refresh the sketch's `Sketch`
      // TIMELINE record — the regen planner resolves modeling-op profiles ONLY from
      // it, and only `finishSketch` authors it (a cancel-only exit left every
      // interactive sketch recordless, failing every later extrude commit with
      // "profile sketch not found in plan"). Sequence: cancel FIRST (best-effort
      // worker-gesture teardown + session squash — take-once, so the finish's
      // squash is a no-op), then finish (solve + regions + record upsert).
      const sketchId = session.sketchId;
      trace("sketch", `exit: cancel+finish ${sketchId} (mint/refresh Sketch timeline record)`);
      void (async () => {
        try {
          await this.deps.client.cancelSketch(sketchId);
          await this.deps.client.finishSketch(sketchId);
        } catch (e) {
          logError("sketch", "exit: finishSketch FAILED (timeline record may be missing)", {
            sketchId,
            error: e,
          });
        }
      })();
    }
    sketchStore.getState().setSession(null);
  }

  private selectMachine(tool: string): void {
    if (this.planePicking) return; // no drawing tool while picking a plane
    // Leaving the dimension tool tears down any in-flight chip/pick.
    if (this.dimensionActive && tool !== "dimension") this.cancelDimension();
    // Leaving the select tool aborts any drag + clears the sketch selection (the
    // enter/exit clears cover mode changes; this covers a same-mode tool switch).
    if (this.selectActive && tool !== "select") {
      this.resetDrag();
      // A tool switch mid-marquee (keyboard shortcut / toolbar click while the button
      // is still down) must drop the overlay AND give LMB orbit back — the flag is
      // shared with the armed draw tools, so a leak here silently kills orbit.
      this.cancelMarquee();
      sketchSelectionStore.getState().clear();
    }
    // Leaving the mirror tool drops its in-progress pick set (the mirror phase is
    // derived from the sketch selection — a stale set would leak into the next tool).
    if (this.mirrorActive && tool !== "mirror") {
      sketchSelectionStore.getState().clear();
    }
    // Leaving Fillet / Offset drops the parameter chip + any half-made pick.
    if (
      (this.filletActive && tool !== "sketchFillet") ||
      (this.offsetActive && tool !== "sketchOffset")
    ) {
      this.endSketchEditTool();
    }

    // LIVE_TOOL_MACHINES, not TOOL_MACHINES: the `withLiveDims` decorator is
    // transparent with no locks and no quantum (it passes the event through by
    // identity), so this is a drop-in that only starts doing something once
    // `stepCtx` hands it one of the two.
    const m = LIVE_TOOL_MACHINES[tool] ?? null;
    this.machine = m;
    this.machineState = m ? m.init() : null;
    // A tool change is a new gesture: nothing typed carries over, and the
    // previous tool's last segment is not this one's angle reference.
    this.clearLiveDimGesture();
    this.dimensionActive = tool === "dimension";
    this.selectActive = tool === "select";
    this.trimActive = tool === "trim";
    this.mirrorActive = tool === "mirror";
    this.filletActive = tool === "sketchFillet";
    this.offsetActive = tool === "sketchOffset";
    // The dimension tool owns the pointer (no orbit) so clicks pick entities.
    this.deps.engine.setSketchDrawingActive(!!m || this.dimensionActive);
    this.deps.engine.setSketchPreview([]);
    this.deps.engine.setSketchGhost(null, null);
    this.deps.engine.setSketchTrimGhost(null); // drop any lingering trim doomed-piece ghost
    if (!m && !this.dimensionActive) this.deps.engine.setSketchSnap(null, false);
    // Set AFTER the planePicking early-return above, so the pick phase keeps the
    // default arrow; set before the per-tool hint branches below so every tool
    // (including their early returns) gets a cursor.
    this.deps.container.style.cursor = CURSOR_BY_TOOL[tool] ?? "default";

    if (this.dimensionActive) {
      this.dimState = dimensionInit();
      viewportStore.getState().setStatusHint("Dimension — click a line, circle, arc, or two points", { sticky: true });
      return;
    }
    if (this.trimActive) {
      viewportStore.getState().setStatusHint("Click a segment to trim · Esc to exit", { sticky: true });
      return;
    }
    if (this.mirrorActive) {
      this.updateMirrorHint();
      return;
    }
    if (this.filletActive) {
      this.openSketchValueChip("Fillet", this.filletRadius, (v) => {
        this.filletRadius = v;
        toolChipStore.getState().setValue(v);
      });
      this.updateFilletHint();
      return;
    }
    if (this.offsetActive) {
      this.openSketchValueChip("Offset", this.offsetDistance, (v) => {
        this.offsetDistance = v;
        toolChipStore.getState().setValue(v);
      });
      viewportStore
        .getState()
        .setStatusHint("Offset — hover a chain on the side to offset, then click · Esc to exit", {
          sticky: true,
        });
      return;
    }
    if (m?.id === "polygon") {
      this.updatePolygonHint();
      return;
    }
    const hint = m && this.liveDimsEnabled() ? (DRAW_TOOL_HINT[m.id] ?? null) : null;
    viewportStore.getState().setStatusHint(hint, hint ? { sticky: true } : undefined);
  }

  /**
   * Polygon status hint — the live side count plus how to change it, which
   * DEPENDS ON PHASE: idle, a digit is the side-count verb; armed, a digit opens
   * the radius chip and Tab reaches the count instead.
   */
  private updatePolygonHint(): void {
    const n = this.machineState?.sides ?? DEFAULT_POLYGON_SIDES;
    const armed = (this.machineState?.anchors.length ?? 0) > 0 && this.liveDimsEnabled();
    const how = armed ? "type to set radius · Tab for sides" : "3–9 to change";
    viewportStore.getState().setStatusHint(`Polygon — ${n} sides · ${how}`, { sticky: true });
  }

  // ── pointer handling ────────────────────────────────────────────────────

  /** Fencing gate for the controller's own queued write-backs (commit / mirror /
   *  drag). A bumped session generation (a newer setSession superseded this turn), a
   *  torn-down session, or a disposed controller ⇒ silently drop the write. */
  private sessionStale(gen: number): boolean {
    if (this.disposed) return true;
    const s = sketchStore.getState();
    return s.session === null || s.sessionGeneration !== gen;
  }

  /**
   * Context for the tool machines: the degeneracy floor (reject a click/drag
   * below ~4px of world so tiny/zero-extent geometry never commits, constant on
   * screen at any zoom) plus the live-dimension projection inputs the
   * `withLiveDims` decorator reads (the raw machines ignore both).
   */
  private stepCtx(): { minSize: number; locks: DimLocks; quantum: DimQuantum | null } {
    return {
      minSize: 4 * this.deps.engine.planePixelWorld(),
      locks: this.liveDim.locks,
      quantum: this.dimQuantumNow(),
    };
  }

  /**
   * The rounding granularity a CURSOR-placed dimension lands on right now, or
   * null for "do not round".
   *
   * Three ways to get null, all of them "something more specific already decided
   * where this point goes": the pref is off, Alt suppresses snapping wholesale,
   * or a GEOMETRY snap won (endpoint … onCurve, and the alignment guides) — those
   * tiers place the point EXACTLY on real geometry, and rounding afterwards would
   * quietly move it off. Only a free point (`none`) or the grid tier — which
   * rounding REPLACES during a draw gesture — is ours to quantize.
   *
   * Locks are unaffected: a typed value pins whatever this returns.
   */
  private dimQuantumNow(): DimQuantum | null {
    if (!settingsStore.getState().snapTo.dimensionRound) return null;
    if (this.altHeld) return null;
    const kind = this.lastSnap?.kind;
    if (kind !== undefined && kind !== "none" && kind !== "grid") return null;
    const minor = chooseGridStep(this.deps.engine.getCameraDistance()).minor;
    return dimQuantum(minor, settingsStore.getState().displayUnit);
  }

  /**
   * Is the dimension quantum currently REPLACING the grid snap tier?
   *
   * Grid quantizes x and y independently; a draw gesture in progress means a
   * LENGTH and an ANGLE, so the two tiers answer different questions and running
   * both would let the grid pull the point back off the rounded length. Only
   * while a machine is armed (≥1 anchor) — with nothing placed there is no
   * dimension to round, so the grid is still the right tier for the first point.
   */
  private dimensionRoundingActive(): boolean {
    if (!this.machineState || this.machineState.anchors.length === 0) return false;
    return settingsStore.getState().snapTo.dimensionRound && !this.altHeld;
  }

  private snapAt(clientX: number, clientY: number): SnapResult | null {
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return null;
    const session = sketchStore.getState().session;
    const sessionEntities = session?.entities ?? null;
    // Reference-equality cache: a commit replaces the array (applySolvedPositions
    // returns the SAME reference iff nothing moved), so this rebuilds only on an
    // actual sketch edit, not on every rAF move.
    if (sessionEntities !== this.snapCacheKey) {
      this.snapCache = buildSnapCache(sessionEntities ?? []);
      this.snapCacheKey = sessionEntities;
    }
    const settings = settingsStore.getState();
    return computeSnap(raw, sessionEntities ?? [], {
      gridStep: chooseGridStep(this.deps.engine.getCameraDistance()).minor,
      pixelWorld: this.deps.engine.planePixelWorld(),
      enableGrid: settings.snapTo.grid && !this.dimensionRoundingActive(),
      enableGuideLines: settings.snapTo.sketchGuideLines,
      enableGuidePoints: settings.snapTo.sketchGuidePoints,
      enableQuadrant: settings.snapTo.quadrant,
      enableIntersection: settings.snapTo.intersection,
      enableOnCurve: settings.snapTo.onCurve,
      suppress: this.altHeld,
      recentPoints: this.machineState?.anchors ?? [],
      cache: this.snapCache ?? undefined,
    });
  }

  private onPointerMove = (e: PointerEvent): void => {
    // Plane-pick phase owns the pointer: highlight the plane under the cursor.
    // A DATUM plane is a pickable sketch host here too (DATUM W1) and WINS — it
    // is offset off the origin, so the ray can hit both, and the datum is the
    // thing the user deliberately created.
    if (this.planePicking) {
      const datumId = this.deps.engine.datumHitTest(e.clientX, e.clientY);
      this.deps.engine.setDatumHover(datumId);
      if (datumId) {
        this.deps.engine.clearPlanePickerHover();
        this.setFaceHover(null);
        return;
      }
      // A body FACE is the LAST rung (W3 trigger b), mirroring the resolve order
      // in onPointerUp: the datum and the three world quads are the things the
      // picker itself put on screen, so they own the pointer first.
      if (this.deps.engine.planePickerHover(e.clientX, e.clientY)) {
        this.setFaceHover(null);
        return;
      }
      // Only while idle: this rung raycasts every body mesh, and an LMB-held move
      // is an orbit whose release `wasClick` rejects anyway — so there is nothing
      // to advertise as clickable and no reason to pay for it mid-drag.
      if (e.buttons !== 0) return;
      this.setFaceHover(facePickFrom(this.deps.engine.probePick(e.clientX, e.clientY)));
      return;
    }
    if (this.selectActive) {
      this.onSelectPointerMove(e);
      return;
    }
    // Trim / Mirror / Fillet / Offset are click tools; a move past DRAG_PX with LMB
    // held is an orbit, not a click — track it so pointerup doesn't fire a stray
    // delete/pick.
    if (this.trimActive || this.mirrorActive || this.filletActive || this.offsetActive) {
      if ((e.buttons & 1) === 0) {
        // Idle move: every one of them gets live feedback (hover tint, plus Trim's
        // destructive doomed-piece ghost and Fillet/Offset's result ghost),
        // coalesced to one raycast/frame.
        this.scheduleHoverHit(e.clientX, e.clientY);
        return;
      }
      const far =
        Math.abs(e.clientX - this.downX) > DRAG_PX || Math.abs(e.clientY - this.downY) > DRAG_PX;
      if (this.downButton === 0 && (e.buttons & 1) !== 0 && far) this.moved = true;
      return;
    }
    if (!this.machine && !this.dimensionActive) return;
    this.pendingMove = e;
    if (e.buttons !== 0 && this.downButton === 0) this.moved = true;
    if (this.moveScheduled) return;
    this.moveScheduled = true;
    requestAnimationFrame(() => {
      if (this.disposed) return;
      this.moveScheduled = false;
      const ev = this.pendingMove;
      this.pendingMove = null;
      if (!ev) return;
      const snap = this.snapAt(ev.clientX, ev.clientY);
      if (!snap) return;
      this.lastSnap = snap;
      const showHints = settingsStore.getState().show.snappingHints;
      this.deps.engine.setSketchSnap(snap, showHints);
      // Dimension mode: the indicator aids aiming; there is no rubber-band preview.
      if (!this.machine || !this.machineState) return;
      const stepped = this.machine.step(this.machineState, { kind: "move", pt: snap.point }, this.stepCtx());
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview(this.decorate(stepped.preview));
      // The RAW preview feeds the H/V ghost: `updateGhost` deliberately ignores
      // construction drafts (the arc radius rubber-band), but a construction-mode
      // line still gets H/V auto-constrained at commit, so the hint must still show.
      this.updateGhost(stepped.preview, snap.point);
      this.syncLiveDims(stepped.dims ?? []);
      this.updateLiveDimPlacement(ev.clientX, ev.clientY);
    });
  };

  private onPointerDown = (e: PointerEvent): void => {
    this.downX = e.clientX;
    this.downY = e.clientY;
    this.downButton = e.button;
    this.moved = false;
    if (this.selectActive) this.onSelectPointerDown(e);
  };

  private onPointerUp = (e: PointerEvent): void => {
    if (this.selectActive) {
      this.onSelectPointerUp(e);
      return;
    }
    const wasClick =
      this.downButton === 0 &&
      e.button === 0 &&
      !this.moved &&
      Math.abs(e.clientX - this.downX) <= DRAG_PX &&
      Math.abs(e.clientY - this.downY) <= DRAG_PX;
    this.downButton = -1;
    if (!wasClick) return;
    if (this.planePicking) {
      // RESOLVE ORDER (pinned by test): datum → world quad → body face. The datum
      // and the quads are chrome the picker itself raised, so they win over
      // whatever model geometry happens to lie under them; the face is the
      // fallback that makes "S with nothing selected, then click the part" work.
      const datumId = this.deps.engine.datumHitTest(e.clientX, e.clientY);
      if (datumId) {
        void this.confirmDatumPick(datumId);
        return;
      }
      const kind = this.deps.engine.planePickerHitTest(e.clientX, e.clientY);
      if (kind) {
        void this.confirmPlanePick(kind);
        return;
      }
      const face = facePickFrom(this.deps.engine.probePick(e.clientX, e.clientY));
      if (face) void this.confirmFacePick(face);
      return;
    }
    if (this.dimensionActive) {
      this.handleDimensionClick(e.clientX, e.clientY);
      return;
    }
    if (this.trimActive) {
      this.handleTrimClick(e.clientX, e.clientY);
      return;
    }
    if (this.mirrorActive) {
      this.handleMirrorClick(e.clientX, e.clientY, e.shiftKey || e.metaKey);
      return;
    }
    if (this.filletActive) {
      this.handleFilletClick(e.clientX, e.clientY);
      return;
    }
    if (this.offsetActive) {
      this.handleOffsetClick(e.clientX, e.clientY);
      return;
    }
    if (!this.machine || !this.machineState) return;
    const snap = this.snapAt(e.clientX, e.clientY) ?? this.lastSnap;
    if (!snap) return;
    // The CLICK's own snap is the last winning one for `dimQuantumNow` below: a
    // move rAF may never have run (a tap, or a click in the same frame), and a
    // stale kind from the previous position would decide this point's rounding.
    this.lastSnap = snap;
    // Captured BEFORE the step consumes the anchors / the commit clears the locks.
    const dims = this.dimCommitContext();
    const stepped = this.machine.step(this.machineState, { kind: "click", pt: snap.point }, this.stepCtx());
    logSketchStep(this.machine.id, "click", stepped);
    this.applySteppedClick(stepped, dims);
  };

  /** Drop what belongs to the CURRENT gesture alone: the values the user pinned,
   *  the chips showing them, and the chain's angle reference. Idempotent. */
  private clearLiveDimGesture(): void {
    this.liveDim = liveDimInit();
    this.lastChainLineId = null;
    this.closeLiveDims();
  }

  /** The live-dimension facts of the gesture as it stands RIGHT NOW — see
   *  {@link DimCommitContext} for why this is captured rather than read later. */
  private dimCommitContext(): DimCommitContext | null {
    if (!this.machine || !this.machineState) return null;
    return {
      toolId: this.machine.id,
      anchors: [...this.machineState.anchors],
      locks: { ...this.liveDim.locks },
      ...(this.machineState.sides !== undefined ? { sides: this.machineState.sides } : {}),
    };
  }

  /**
   * The tail of a click that stepped a draw machine: adopt the new state, publish
   * the preview, queue any commit, and drop what the gesture no longer owns.
   *
   * Extracted from `onPointerUp` so the chip's Enter (Wave 3) commits through
   * exactly THIS path rather than a second copy that drifts from it.
   */
  private applySteppedClick(stepped: ToolStep, dims: DimCommitContext | null): void {
    this.machineState = stepped.state;
    this.deps.engine.setSketchPreview(this.decorate(stepped.preview));
    const committed = stepped.committed ?? [];
    if (committed.length > 0) {
      void this.commit(this.decorate(committed), stepped.committedConstraints, dims ?? undefined);
    }
    if (stepped.done) {
      this.deps.engine.setSketchGhost(null, null);
      this.lastChainLineId = null; // a fresh chain has no previous segment
    }
    // A step that produced geometry (or ended the gesture) CONSUMES its locks —
    // the typed number described that entity, not the next one. A non-committing
    // click keeps them, which is what carries an arc's radius phase 1 → 2.
    if (committed.length > 0 || stepped.done) this.liveDim = liveDimInit();
    // Republish for the phase the click landed IN — an empty set closes the chips
    // (the gesture finished), a non-empty one re-opens them for the next leg.
    this.syncLiveDims(stepped.dims ?? []);
    if (this.machine?.id === "polygon") this.updatePolygonHint();
  }

  // ── live dimension chips (SP-1 Wave 3) ────────────────────────────────────

  /** Is the chip lane switched on? Also the ONLY reader of `show.liveDimensions`. */
  private liveDimsEnabled(): boolean {
    return settingsStore.getState().show.liveDimensions;
  }

  /**
   * Publish the current phase's chip descriptors: open/refresh the store, and
   * push each chip's world anchor straight at the overlay driver.
   *
   * The anchors deliberately do NOT travel through React — they move every rAF,
   * and re-rendering a focused text input at that rate would fight the typing.
   */
  private syncLiveDims(dims: ToolDimension[]): void {
    const plane = sketchStore.getState().session?.plane;
    if (!this.liveDimsEnabled() || dims.length === 0 || !plane) {
      this.closeLiveDims();
      return;
    }
    const anchors: LiveDimAnchors = {};
    for (const d of dims) {
      const w = planePointToWorld(plane, d.anchor);
      anchors[d.field] = [w.x, w.y, w.z];
    }
    const chips: LiveDimChipField[] = dims.map(({ anchor: _anchor, ...chip }) => chip);
    const store = liveDimStore.getState();
    if (this.liveDimsOpen) store.update(chips, anchors);
    else store.show(chips, anchors, this.liveDimHandlers());
    this.liveDimsOpen = true;
    this.liveDims = dims;
    for (const d of dims) {
      const at = anchors[d.field];
      if (at) this.deps.engine.moveChip(liveDimChipId(d.field), at);
    }
  }

  /** The chips are gone: close the store (React unmounts the hosts, which is what
   *  unregisters them from the overlay). Idempotent. */
  private closeLiveDims(): void {
    this.liveDims = [];
    if (!this.liveDimsOpen) return;
    this.liveDimsOpen = false;
    liveDimStore.getState().clear();
  }

  /** Flip the chips to the quadrant with room. Cheap, and computed at most once
   *  per pointer move — the store only takes a write when the side changes. */
  private updateLiveDimPlacement(clientX: number, clientY: number): void {
    const c = this.deps.container;
    const next = placementFor(clientX, clientY, c.clientWidth, c.clientHeight);
    if (next === this.liveDimPlacement) return;
    this.liveDimPlacement = next;
    liveDimStore.getState().setPlacement(next);
  }

  /** The chip → controller callbacks. All of them run the same pure FSM. */
  private liveDimHandlers(): LiveDimHandlers {
    return {
      onFocus: (field) => this.applyLiveDimStep({ kind: "focus", field }),
      onText: (text) => this.applyLiveDimStep({ kind: "text", text }),
      onTab: (back, value) =>
        this.applyLiveDimStep({ kind: "tab", fields: this.liveDimFieldIds(), back, value }),
      onEnter: (value) => this.applyLiveDimStep({ kind: "enter", value }),
      onEscape: () => this.applyLiveDimStep({ kind: "escField" }),
      // Only a blur from the field the FSM still considers focused is a real
      // "focus left the chip set" (viewport click). A Tab to a sibling chip
      // fires the DEPARTED field's blur after the FSM already moved — React's
      // focus effect on the new field synchronously blurs the old input — and
      // processing that stale blur would null the focus and swallow every
      // keystroke into the new field.
      onBlur: (field, value) => {
        if (this.liveDim.focus !== field) return;
        this.applyLiveDimStep({ kind: "blurCommit", value });
      },
    };
  }

  /** Tab order = the frame's own field order (never re-sorted). */
  private liveDimFieldIds(): DimFieldId[] {
    return this.liveDims.map((d) => d.field);
  }

  /**
   * Run one chip event through the FSM and make the world agree with the result:
   * mirror focus/text into the store, apply a side-count pick as the MACHINE verb
   * it really is, re-step at the cursor so the preview shows the new lock at
   * once, and commit the gesture when Enter said so.
   */
  private applyLiveDimStep(ev: LiveDimEvent): void {
    if (this.disposed) return;
    const step = liveDimStep(this.liveDim, ev);
    this.liveDim = step.state;
    liveDimStore.getState().setFocus(step.state.focus, step.state.text);
    // A side count never MOVES a point, so it is not a geometry lock: it is the
    // polygon machine's own `sides` verb. The lock is kept anyway so the chip
    // still reads as pinned (it authors nothing either way).
    if (step.locked === "sides") this.stepSides(step.state.locks.sides ?? DEFAULT_POLYGON_SIDES);
    // Only a LOCK changes geometry — re-stepping on every character would repaint
    // the rubber-band at typing speed to produce the identical point.
    if (ev.kind !== "text") this.restepAtCursor();
    if (step.commitGesture) this.commitLiveDimGesture();
  }

  /** Push a typed side count into the polygon machine. */
  private stepSides(n: number): void {
    if (!this.machine || !this.machineState) return;
    const stepped = this.machine.step(this.machineState, { kind: "sides", n }, this.stepCtx());
    logSketchStep(this.machine.id, "sides", stepped);
    this.machineState = stepped.state;
    this.deps.engine.setSketchPreview(this.decorate(stepped.preview));
    this.updatePolygonHint();
  }

  /**
   * Re-step the machine at the point it is already on, so a lock taken from a
   * chip shows up in the rubber-band immediately instead of at the next pointer
   * move. The cursor is re-PROJECTED through the new locks (`projectEvent`), so
   * the direction the user aimed survives while the pinned number wins.
   */
  private restepAtCursor(): void {
    if (!this.machine || !this.machineState) return;
    const cursor = this.machineState.cursor;
    if (!cursor) return;
    const stepped = this.machine.step(this.machineState, { kind: "move", pt: cursor }, this.stepCtx());
    this.machineState = stepped.state;
    this.deps.engine.setSketchPreview(this.decorate(stepped.preview));
    this.updateGhost(stepped.preview, stepped.state.cursor ?? cursor);
    this.syncLiveDims(stepped.dims ?? []);
  }

  /**
   * Enter on a chip commits the gesture through the SAME tail a mouse click
   * takes (`applySteppedClick`) — a second commit path would drift from it, and
   * the whole point of the extraction is that the two are byte-identical.
   */
  private commitLiveDimGesture(): void {
    if (!this.machine || !this.machineState) return;
    const cursor = this.machineState.cursor;
    if (!cursor) return;
    // Captured BEFORE the step consumes the anchors / the commit clears the locks.
    const dims = this.dimCommitContext();
    const stepped = this.machine.step(this.machineState, { kind: "click", pt: cursor }, this.stepCtx());
    logSketchStep(this.machine.id, "enter", stepped);
    this.applySteppedClick(stepped, dims);
  }

  /**
   * A digit / `-` / `.` typed over the VIEWPORT opens the first chip on it.
   *
   * Requires an ARMED machine: with no anchor placed there is no dimension yet,
   * which is what leaves the polygon tool's idle 3-9 side-count verb alone.
   * Returns whether the key was claimed.
   */
  private tryStartLiveDimTyping(e: KeyboardEvent): boolean {
    if (!LIVE_DIM_TYPE_KEY.test(e.key) || e.metaKey || e.ctrlKey || e.altKey) return false;
    if (!this.liveDimsEnabled() || !this.machineState) return false;
    if (this.machineState.anchors.length === 0 || this.liveDims.length === 0) return false;
    const step = liveDimStep(this.liveDim, {
      kind: "typeStart",
      ch: e.key,
      fields: this.liveDimFieldIds(),
    });
    if (step.state.focus === null) return false;
    this.liveDim = step.state;
    liveDimStore.getState().setFocus(step.state.focus, step.state.text);
    e.stopPropagation();
    e.preventDefault();
    return true;
  }

  // ── commit round-trip ─────────────────────────────────────────────────────

  /** Public thin wrapper: serialize the commit through the shared mutation queue so
   *  a burst of clicks (fast polyline) rebases each segment on the settled prior one.
   *  `specs` are the tool's OWN constraints over `committed` (slot / polygon);
   *  `dims` is the gesture's live-dimension context, captured by value. */
  private commit(
    committed: DraftEntity[],
    specs?: ToolConstraintSpec[],
    dims?: DimCommitContext,
  ): Promise<void> {
    return enqueueSketchMutation(() => this.commitNow(committed, specs, dims));
  }

  private async commitNow(
    committed: DraftEntity[],
    specs?: ToolConstraintSpec[],
    dims?: DimCommitContext,
  ): Promise<void> {
    if (this.disposed) return;
    const gen = sketchStore.getState().sessionGeneration;
    // Re-read the session INSIDE the queued turn and rebase the committed drafts onto
    // it (id-assign here too) so a prior queued commit's result is already folded in.
    const session = sketchStore.getState().session;
    if (!session) return;
    const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };

    const newEntities: SketchEntity[] = committed.map((d) => ({
      id: sketchStore.getState().nextEntityId(),
      ...draftToEntityFields(d),
    }));
    const { toolAuthored, dimAuthored } = this.buildCommitConstraints(session, newEntities, specs, dims);

    const entities = [...session.entities, ...newEntities];
    const base = [...session.constraints, ...toolAuthored];
    const applied = await this.upsertWithDimFallback(session, gen, entities, base, dimAuthored);
    if (!applied) return;
    const { result, constraints, rejectHint } = applied;

    // F-WP9: the solver may have MOVED points (constraint-driven); write the
    // solved positions back into the geometry (backend point UUIDs were already
    // reverse-mapped to `entityId.Position` keys by the client). No-op when the
    // solve returned no movement (identity upsert) — same array reference.
    const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});

    const next: SketchSession = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
    sketchStore.getState().setSession(next);
    sketchStore.getState().setConflicting(result.conflicting ?? []);
    this.deps.engine.updateSketchSession(next.plane, solvedEntities, next.status);
    this.pushSolve(session.sketchId, result.dof, result.status);
    // EXACTLY ONE undo entry either way — a rejected dimension still leaves the
    // entity the user drew, so undo has to land on the state before the click,
    // not between the two upserts.
    sketchStore.getState().pushUndoSnapshot(before, { kind: "commit" });
    if (rejectHint) viewportStore.getState().setStatusHint(rejectHint, { severity: "error" });
    // The chain's angle reference for the NEXT segment. `?? null` on purpose: a
    // batch with no line (circle, ellipse) leaves nothing to measure against.
    this.lastChainLineId = newEntities.find((e) => e.type === "Line")?.id ?? null;
  }

  /**
   * The two constraint sets a commit authors: the TOOL's own (slot tangency /
   * polygon regularity, merged with the auto-constraint inference) and the ones
   * the user TYPED during the gesture.
   *
   * The typed set is deliberately NOT routed through `resolveToolConstraints`:
   * that call drops the intra-batch half of the inference whenever specs are
   * present, and a typed length must not cost a rect the H/V it would otherwise
   * have inferred. It is deduped against the tool set instead — a typed 0°
   * authors Horizontal and the inference produces the same Horizontal, and the
   * draw path has no reject lane for the inferred set (see `dedupeDimConstraints`).
   */
  private buildCommitConstraints(
    session: SketchSession,
    newEntities: SketchEntity[],
    specs: ToolConstraintSpec[] | undefined,
    dims: DimCommitContext | undefined,
  ): { toolAuthored: SketchConstraint[]; dimAuthored: SketchConstraint[] } {
    const nextId = (): string => sketchStore.getState().nextConstraintId();
    // Tool-authored constraints resolve their index refs against the id-assigned
    // entities and SUPPRESS the intra-batch half of the inference — see
    // `resolveToolConstraints` for why the draw path cannot afford a duplicate.
    const inferred = inferConstraints(newEntities, session.entities, { nextConstraintId: nextId });
    const toolAuthored = resolveToolConstraints(specs, newEntities, inferred, nextId);
    if (!dims || Object.keys(dims.locks).length === 0) return { toolAuthored, dimAuthored: [] };
    const dimSpecs = dimConstraintSpecs(dims.toolId, dims.anchors, dims.locks, {
      // Read HERE rather than captured at the click: this field is written only
      // by the commit turn, and the queue is serial, so inside the turn it names
      // the segment that actually settled. At click time it would still be null
      // for the very next segment of a fast chain.
      ...(this.lastChainLineId ? { prevLineId: this.lastChainLineId } : {}),
      ...(dims.sides !== undefined ? { sides: dims.sides } : {}),
    });
    const resolved = resolveDimConstraints(dimSpecs, newEntities, nextId);
    return { toolAuthored, dimAuthored: dedupeDimConstraints(resolved, toolAuthored) };
  }

  /**
   * Upsert the batch WITH the typed dimensions, and on a conflict re-upsert
   * WITHOUT them.
   *
   * The entity survives: the user DREW it, and refusing the whole gesture because
   * one typed number over-constrains would throw away work they did not get wrong.
   * The dropped dimension is named in the returned hint. Returns null when the
   * turn was superseded (or the call failed) — the caller publishes nothing then.
   */
  private async upsertWithDimFallback(
    session: SketchSession,
    gen: number,
    entities: SketchEntity[],
    base: SketchConstraint[],
    dimAuthored: SketchConstraint[],
  ): Promise<{ result: SketchUpsertResult; constraints: SketchConstraint[]; rejectHint?: string } | null> {
    const withDims = dimAuthored.length > 0 ? [...base, ...dimAuthored] : base;
    const result = await this.solveUpsert(session.sketchId, gen, entities, withDims);
    if (!result) return null;
    if (dimAuthored.length === 0 || !isConflictStatus(result.status)) {
      return { result, constraints: withDims };
    }
    // Name the clashing party from the FAILED solve, before it is discarded.
    const rejectHint = rejectConflictHint(withDims, result.conflicting, dimAuthored[0].id, "Dimension");
    const retry = await this.solveUpsert(session.sketchId, gen, entities, base);
    return retry ? { result: retry, constraints: base, rejectHint } : null;
  }

  /** One upsert round-trip with the two guards every commit needs: report a
   *  failure, and drop the result when a late exit / newer session superseded
   *  this turn mid-await. Null ⇒ the caller publishes nothing. */
  private async solveUpsert(
    sketchId: string,
    gen: number,
    entities: SketchEntity[],
    constraints: SketchConstraint[],
  ): Promise<SketchUpsertResult | null> {
    try {
      const result = await this.deps.client.sketchUpsert(sketchId, entities, constraints);
      return this.sessionStale(gen) ? null : result;
    } catch (e) {
      if (this.sessionStale(gen)) return null;
      viewportStore.getState().setStatusHint(`Sketch solve failed: ${sketchErr(e)}`, { severity: "error", sticky: true });
      return null;
    }
  }

  private pushSolve(id: string, dof: number, status: SketchSession["status"]): void {
    documentStore.getState().setSketchSolve(id, dof, docSketchStatus(status));
    viewportStore.setState({ dofBadge: dof });
  }

  // ── Dimension tool ─────────────────────────────────────────────────────────

  /** A click in dimension mode: resolve the pick, step the FSM, (re)open the chip. */
  private handleDimensionClick(clientX: number, clientY: number): void {
    const session = sketchStore.getState().session;
    if (!session) return;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return;
    const tol = SNAP_PX * this.deps.engine.planePixelWorld(); // same reach as snapping
    const target = pickDimensionTarget(raw, session.entities, tol);
    if (!target) {
      // A click on empty space cancels a half-made pick (but leaves an open chip).
      if (!this.dimState.ready) this.cancelDimension();
      return;
    }
    const step = dimensionStep(this.dimState, { kind: "pick", target });
    this.dimState = step.state;
    if (this.dimState.ready) this.openDimensionChip();
    else {
      toolChipStore.getState().clear();
      viewportStore.getState().setStatusHint("Dimension — pick a second point", { sticky: true });
    }
  }

  /** Open (or re-seed) the dimension chip at the armed spec's anchor. */
  private openDimensionChip(): void {
    const spec = this.dimState.ready;
    const session = sketchStore.getState().session;
    if (!spec || !session) return;
    const world = planePointToWorld(session.plane, spec.anchor).toArray() as [number, number, number];
    toolChipStore.getState().showDimension(
      spec.value,
      dimensionSuffix(spec.kind),
      world,
      (v) => void this.commitDimensionValue(v),
      () => this.cancelDimension(),
    );
    viewportStore.getState().setStatusHint(null);
  }

  /** Chip Enter: author the armed dimension through the solver (reject on conflict). */
  private async commitDimensionValue(value: number): Promise<void> {
    const step = dimensionStep(this.dimState, { kind: "commit", value });
    this.dimState = step.state;
    toolChipStore.getState().clear();
    if (!step.emit) return;
    const id = sketchStore.getState().nextConstraintId();
    const constraint = buildDimensionConstraint(step.emit, id);
    try {
      const { rejected, hint } = await commitDimensionConstraint(this.deps.client, constraint);
      viewportStore
        .getState()
        .setStatusHint(rejected ? hint ?? "Dimension removed — it would over-constrain the sketch" : null);
    } catch (e) {
      viewportStore.getState().setStatusHint(`Dimension failed: ${sketchErr(e)}`, { severity: "error", sticky: true });
    }
  }

  /** Esc / tool change / empty click: drop the in-flight dimension + chip. */
  private cancelDimension(): void {
    this.dimState = dimensionInit();
    toolChipStore.getState().clear();
    if (this.dimensionActive) {
      viewportStore.getState().setStatusHint("Dimension — click a line, circle, arc, or two points", { sticky: true });
    }
  }

  // ── Trim tool ────────────────────────────────────────────────────────────────
  //
  // Parametric trim: a click on an entity removes the SPAN between the entity's two
  // nearest crossings with other entities (the clicked segment), leaving the
  // surviving pieces (`trimEntity` → `trimPieces`). An entity with no qualifying
  // crossings falls back to whole-entity delete. A miss is a no-op. Esc falls
  // through to the global ladder → back to select. Hover shows a destructive
  // doomed-piece ghost (see `renderTrimGhost`).

  private handleTrimClick(clientX: number, clientY: number): void {
    const session = sketchStore.getState().session;
    if (!session) return;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return;
    const tol = SNAP_PX * this.deps.engine.planePixelWorld();
    const hit = hitTestSketch(raw, session.entities, tol);
    if (!hit) return; // miss → no-op
    // Trim REPLACES the target with its surviving pieces — a destructive edit the
    // backend refuses outright on locked geometry (W2 L3). Refuse here, loudly:
    // the alternative is a rejected upsert batch for an action the user had no
    // way to know was illegal.
    if (lockedEntityIds(session.entities).has(hit.entityId)) {
      this.deps.engine.setSketchTrimGhost(null);
      viewportStore.getState().setStatusHint(`${LOCKED_GEOMETRY_HINT} — it cannot be trimmed`);
      return;
    }
    // Drop the hover ghost immediately; the write-back's updateSketchSession also
    // clears it, but this makes the click feel instant.
    this.deps.engine.setSketchTrimGhost(null);
    void trimEntity(this.deps.client, hit.entityId, [raw.x, raw.y], {
      minSize: 4 * this.deps.engine.planePixelWorld(),
    });
  }

  // ── Mirror tool ──────────────────────────────────────────────────────────────
  //
  // Two-phase FSM keyed off the sketch selection (no separate phase field):
  //   Phase A (selection empty) — clicks SELECT entities to mirror (select-tool
  //     `clickSelection` semantics: plain replaces, Shift/Meta toggles, miss clears).
  //   Phase B (selection non-empty) — a Shift/Meta click keeps editing the set; a
  //     plain click that hits a LINE body NOT in the set is the mirror axis and
  //     performs the mirror; any other plain click is a no-op (never clears).
  // After a mirror the selection switches to the new copies (repeatable); Esc exits
  // via the global ladder (→ select), which clears the set on the tool change.

  private handleMirrorClick(clientX: number, clientY: number, additive: boolean): void {
    const session = sketchStore.getState().session;
    if (!session) return;
    const hit = this.hitAt(clientX, clientY);
    const selected = sketchSelectionStore.getState().selected;

    // Phase A, or an additive click in Phase B: keep building the mirror set.
    if (selected.length === 0 || additive) {
      sketchSelectionStore.getState().set(clickSelection(selected, hit, additive));
      this.updateMirrorHint();
      return;
    }

    // Phase B plain click: the axis is a LINE body-pick NOT already in the set.
    const selectedIds = new Set(selected.map((s) => s.entityId));
    if (!hit || hit.point || selectedIds.has(hit.entityId)) return; // no-op (incl. axis-in-selection)
    const axis = session.entities.find((e) => e.id === hit.entityId);
    if (!axis || axis.type !== "Line") return; // only a line can be a mirror axis
    const sources = [...selectedIds]
      .map((id) => session.entities.find((e) => e.id === id))
      .filter((e): e is SketchEntity => !!e);
    if (sources.length === 0) return;
    void this.performMirror(sources, axis);
  }

  /** Phase-appropriate mirror status hint (derived from the current selection). */
  private updateMirrorHint(): void {
    const empty = sketchSelectionStore.getState().selected.length === 0;
    viewportStore
      .getState()
      .setStatusHint(
        empty ? "Select entities to mirror first" : "Click the mirror axis line · Esc to exit",
        { sticky: true },
      );
  }

  /** Public thin wrapper: serialize the mirror through the shared mutation queue. */
  private performMirror(sources: SketchEntity[], axis: SketchEntity): Promise<void> {
    return enqueueSketchMutation(() => this.performMirrorNow(sources, axis));
  }

  /** Reflect `sources` across the axis line, then ONE sketchUpsert (session + copies,
   *  session constraints + the new Symmetric/Equal). Selection moves to the copies. */
  private async performMirrorNow(sources: SketchEntity[], axis: SketchEntity): Promise<void> {
    if (this.disposed) return;
    const gen = sketchStore.getState().sessionGeneration;
    const session = sketchStore.getState().session;
    if (!session || !axis.p0 || !axis.p1) return;
    const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
    const { entities: mirrored, constraints: mirrorCons } = mirrorEntities(
      sources,
      axis.id,
      axis.p0,
      axis.p1,
      {
        entityId: () => sketchStore.getState().nextEntityId(),
        constraintId: () => sketchStore.getState().nextConstraintId(),
      },
    );
    if (mirrored.length === 0) return;

    const entities = [...session.entities, ...mirrored];
    const constraints = [...session.constraints, ...mirrorCons];
    let result;
    try {
      result = await this.deps.client.sketchUpsert(session.sketchId, entities, constraints);
    } catch (e) {
      if (this.sessionStale(gen)) return;
      viewportStore.getState().setStatusHint(`Mirror failed: ${sketchErr(e)}`, { severity: "error", sticky: true });
      return;
    }
    if (this.sessionStale(gen)) return; // exited / superseded during await

    const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});
    const next: SketchSession = {
      ...session,
      entities: solvedEntities,
      constraints,
      dof: result.dof,
      status: result.status,
    };
    sketchStore.getState().setSession(next);
    sketchStore.getState().setConflicting(result.conflicting ?? []);
    this.deps.engine.updateSketchSession(next.plane, solvedEntities, next.status);
    this.pushSolve(session.sketchId, result.dof, result.status);
    sketchStore.getState().pushUndoSnapshot(before, { kind: "mirror" });
    // Selection switches to the mirrored copies (body picks) → back in Phase B, repeatable.
    sketchSelectionStore.getState().set(mirrored.map((e) => ({ entityId: e.id })));
    viewportStore
      .getState()
      .setStatusHint(
        `Mirrored ${sources.length} ${sources.length === 1 ? "entity" : "entities"} · click another axis or Esc to exit`,
      );
  }

  // ── Sketch Fillet / Offset shared plumbing (WP-C T2b) ────────────────────────

  /**
   * Endpoint-coincidence tolerance for the chain/corner walks — SCREEN-SCALED but
   * TIGHT (a quarter pixel of world), with an absolute floor for the zoomed-way-in
   * case. It must not be the 8px pick reach: both tools build their geometry from
   * one of the two endpoints, so welding a visibly-apart pair would author a
   * corner/chain that is not where the user sees one.
   */
  private weldTol(): number {
    return Math.max(1e-6, 0.25 * this.deps.engine.planePixelWorld());
  }

  /** The 8px pick reach every sketch tool hit-tests with. */
  private pickTol(): number {
    return SNAP_PX * this.deps.engine.planePixelWorld();
  }

  /** Open an armed edit tool's live-parameter chip at the sketch origin (a stable
   *  anchor: the camera framed the plane on entry, so (0,0) is on screen). */
  private openSketchValueChip(label: string, value: number, onValue: (v: number) => void): void {
    const session = sketchStore.getState().session;
    if (!session) return;
    const world = planePointToWorld(session.plane, { x: 0, y: 0 }).toArray() as [number, number, number];
    toolChipStore.getState().showSketchValue(value, world, label, onValue);
  }

  /** Drop everything an armed Fillet / Offset owns (chip, half-pick, ghost). */
  private endSketchEditTool(): void {
    this.filletFirstPick = null;
    if (this.filletActive || this.offsetActive) {
      toolChipStore.getState().clear();
      this.deps.engine.setSketchPreview([]);
      sketchSelectionStore.getState().clear();
    }
  }

  // ── Sketch Fillet tool ───────────────────────────────────────────────────────
  //
  // Two ways in, one operation:
  //   ONE-CLICK  — click near a corner (two lines sharing an endpoint); the
  //                nearest such corner within the pick reach wins.
  //   TWO-PICK   — click a line with no corner under the cursor to arm it, then
  //                click the line it shares an endpoint with.
  // Either way the apply replaces both legs with their trimmed selves + the
  // tangent arc in ONE undo step (sketchService.filletSketchCorner), and the tool
  // stays armed for the next corner. Hover ghosts the exact result at the current
  // chip radius. Esc cancels a half-made two-pick first, and only then falls
  // through to the global ladder (→ select), matching the dimension tool's ladder.

  /** The corner the cursor resolves to, honouring an armed two-pick. */
  private resolveFilletPick(at: Point2): { a: SketchEntity; b: SketchEntity } | null {
    const session = sketchStore.getState().session;
    if (!session) return null;
    const weldTol = this.weldTol();
    if (this.filletFirstPick) {
      const a = session.entities.find((e) => e.id === this.filletFirstPick);
      const hit = hitTestSketch(at, session.entities, this.pickTol());
      const b = hit ? session.entities.find((e) => e.id === hit.entityId) : undefined;
      if (!a || !b || a.id === b.id) return null;
      return sharedCorner(a, b, weldTol) ? { a, b } : null;
    }
    const corner = findFilletCorner(at, session.entities, { reach: this.pickTol(), weldTol });
    return corner ? { a: corner.a, b: corner.b } : null;
  }

  /** Ghost the trimmed legs + tangent arc the current hover would author. */
  private renderFilletGhost(clientX: number, clientY: number): void {
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    const pick = raw ? this.resolveFilletPick(raw) : null;
    if (!pick) {
      this.deps.engine.setSketchPreview([]);
      return;
    }
    const solved = filletGeometry(pick.a, pick.b, this.filletRadius, { tol: this.weldTol() });
    this.deps.engine.setSketchPreview(solved.ok ? [...solved.geom.drafts] : []);
  }

  private handleFilletClick(clientX: number, clientY: number): void {
    const session = sketchStore.getState().session;
    if (!session) return;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return;
    const pick = this.resolveFilletPick(raw);
    if (pick) {
      this.filletFirstPick = null;
      sketchSelectionStore.getState().clear();
      this.deps.engine.setSketchPreview([]);
      void this.applyFillet(pick.a.id, pick.b.id);
      return;
    }
    // No corner resolved: a LINE under the cursor arms (or re-arms) the two-pick
    // path; anything else drops it. Never clears geometry — a miss is inert.
    const hit = hitTestSketch(raw, session.entities, this.pickTol());
    const line = hit ? session.entities.find((e) => e.id === hit.entityId) : undefined;
    if (!line || line.type !== "Line") {
      this.filletFirstPick = null;
      sketchSelectionStore.getState().clear();
      this.updateFilletHint();
      return;
    }
    this.filletFirstPick = line.id;
    sketchSelectionStore.getState().set([{ entityId: line.id }]);
    this.updateFilletHint();
  }

  private async applyFillet(aId: string, bId: string): Promise<void> {
    const radius = this.filletRadius;
    const result = await filletSketchCorner(this.deps.client, aId, bId, radius, {
      tol: this.weldTol(),
    });
    if (this.disposed) return;
    if (result.ok) {
      viewportStore
        .getState()
        .setStatusHint(`Filleted at ${lengthHint(radius)} · click another corner or Esc to exit`);
      return;
    }
    // A locked-geometry refusal already published its own hint (and carries no
    // failure), so only a geometric refusal is phrased here.
    if (result.failure) {
      viewportStore.getState().setStatusHint(filletFailureHint(result.failure), { severity: "error" });
    }
  }

  /** Phase-appropriate fillet hint (one-click corner vs. armed two-pick). */
  private updateFilletHint(): void {
    viewportStore
      .getState()
      .setStatusHint(
        this.filletFirstPick
          ? "Fillet — click the line that shares a corner · Esc to cancel"
          : "Fillet — click a corner (or two lines) · Esc to exit",
        { sticky: true },
      );
  }

  // ── Sketch Offset tool ───────────────────────────────────────────────────────
  //
  // Hover any Line/Arc/Circle: the connected chain it belongs to is walked
  // (offsetMath.buildChain), the offset SIDE is taken from which side of that
  // chain the cursor is on, and the whole parallel copy is ghosted live. A click
  // authors it in ONE undo step. The tool stays armed, so a second click offsets
  // again (from whichever chain is now under the cursor).
  //
  // The controller sends the SIGNED distance rather than a side token: the
  // service rebuilds the same chain from the same seed id, so the traversal — and
  // therefore what "+" means — is identical on both sides of the queue.

  /** Chain + signed distance the cursor currently resolves to, or null. */
  private resolveOffsetPick(at: Point2): { seedId: string; chain: Chain; signed: number } | null {
    const session = sketchStore.getState().session;
    if (!session) return null;
    const hit = hitTestSketch(at, session.entities, this.pickTol());
    const seed = hit ? session.entities.find((e) => e.id === hit.entityId) : undefined;
    if (!seed) return null;
    const chain = buildChain(seed, session.entities, this.weldTol());
    if (!chain) return null;
    return { seedId: seed.id, chain, signed: chainSideSign(chain, at) * this.offsetDistance };
  }

  /** Ghost the parallel chain the current hover would author. */
  private renderOffsetGhost(clientX: number, clientY: number): void {
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    const pick = raw ? this.resolveOffsetPick(raw) : null;
    if (!pick) {
      this.deps.engine.setSketchPreview([]);
      return;
    }
    const solved = offsetChain(pick.chain, pick.signed, { minSize: this.stepCtx().minSize });
    this.deps.engine.setSketchPreview(solved.ok ? solved.entities : []);
  }

  private handleOffsetClick(clientX: number, clientY: number): void {
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return;
    const pick = this.resolveOffsetPick(raw);
    if (!pick) return; // a miss is inert
    this.deps.engine.setSketchPreview([]);
    void this.applyOffset(pick.seedId, pick.signed);
  }

  private async applyOffset(seedId: string, signed: number): Promise<void> {
    const result = await offsetSketchChain(this.deps.client, seedId, signed, {
      tol: this.weldTol(),
      minSize: this.stepCtx().minSize,
    });
    if (this.disposed) return;
    if (result.ok) {
      viewportStore
        .getState()
        .setStatusHint(
          `Offset by ${lengthHint(Math.abs(signed))} · click another chain or Esc to exit`,
        );
      return;
    }
    if (result.failure) {
      viewportStore.getState().setStatusHint(offsetFailureHint(result.failure), { severity: "error" });
    }
  }

  // ── Select tool ─────────────────────────────────────────────────────────────
  //
  // FLOW. pointerdown resolves a hit; a DRAGGABLE handle arms a drag (and suppresses
  // LMB orbit); anything else stays a candidate click. On pointerup:
  //   - a real click (< DRAG_PX, no move) → click-select (Shift/Meta toggles, plain
  //     replaces, a miss clears);
  //   - a drag past DRAG_PX on an armed handle → beginGesture → solveDrag (fire-and-
  //     forget, latest-wins by seq) → endGesture (ONE undo) + the selection stays;
  //   - a drag past DRAG_PX from EMPTY space → marquee / box select (W2-D, below).
  // Esc mid-drag ends the gesture at the ORIGINAL point + restores pre-drag geometry.

  /** hitTest the current session at a client point (same 8px reach the dimension tool uses). */
  private hitAt(clientX: number, clientY: number): SketchSel | null {
    const session = sketchStore.getState().session;
    if (!session) return null;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return null;
    const tol = SNAP_PX * this.deps.engine.planePixelWorld();
    return hitTestSketch(raw, session.entities, tol);
  }

  private onSelectPointerDown = (e: PointerEvent): void => {
    if (e.button !== 0) return;
    // Self-heal: a previous gesture whose release we never saw (shouldn't happen —
    // the window pointerup net covers it — but a stuck overlay is worse than a
    // redundant reset).
    this.cancelMarquee();
    const session = sketchStore.getState().session;
    const hit = this.hitAt(e.clientX, e.clientY);
    const intent = dragIntent(hit, session?.entities ?? []);
    // LOCKED GEOMETRY (W2 L3): a projected boundary point is a legitimate snap +
    // selection target, but dragging it is a `SetEntityPositions` the backend
    // refuses. Refuse to ARM (rather than letting the drag run and fail at
    // endGesture), and say why — a handle that silently ignores the pointer reads
    // as a broken app. The click path below is untouched: the point stays
    // selectable.
    if (intent && session && lockedEntityIds(session.entities).has(intent.sel.entityId)) {
      this.dragArmed = null;
      viewportStore.getState().setStatusHint(`${LOCKED_GEOMETRY_HINT} — it cannot be dragged`);
      return;
    }
    this.dragArmed = intent;
    // Suppress LMB orbit for a potential handle drag; a plain click restores it on up.
    if (intent) {
      this.deps.engine.setSketchDrawingActive(true);
      return;
    }
    // Empty space: arm a marquee. This DELIBERATELY claims the empty-space LMB drag
    // from orbit (RMB pan + Shift/two-finger orbit are untouched). A sub-threshold
    // release collapses back to the ordinary click-clear path.
    if (hit === null && session) {
      this.marqueeArmed = true;
      this.deps.engine.setSketchDrawingActive(true);
    }
  };

  private onSelectPointerMove = (e: PointerEvent): void => {
    // Idle move (no primary button): live hover feedback — the engine hover
    // recolor is driven from sketchSelectionStore via the ViewportRoot bridge.
    // Coalesced to one raycast per frame (P2).
    if ((e.buttons & 1) === 0) {
      this.scheduleHoverHit(e.clientX, e.clientY);
      return;
    }
    // Only care about a primary-button drag (LMB held) initiated in the viewport.
    if (this.downButton !== 0) return;
    const far =
      Math.abs(e.clientX - this.downX) > DRAG_PX || Math.abs(e.clientY - this.downY) > DRAG_PX;
    if (far) this.moved = true;
    if (this.dragging || this.dragStarting) {
      this.scheduleSelectSolve(e.clientX, e.clientY);
      return;
    }
    if (this.dragArmed && far) {
      void this.beginSelectDrag(e.clientX, e.clientY);
      return;
    }
    if (this.marqueeActive || (this.marqueeArmed && far)) {
      this.marqueeActive = true;
      this.scheduleMarquee(e.clientX, e.clientY);
    }
  };

  /** Coalesce idle-hover hit-tests to ONE raycast per frame (P2). The engine hover
   *  recolor is driven from sketchSelectionStore via the ViewportRoot bridge. */
  private scheduleHoverHit(clientX: number, clientY: number): void {
    this.pendingHover = { x: clientX, y: clientY };
    if (this.hoverScheduled) return;
    this.hoverScheduled = true;
    requestAnimationFrame(() => {
      this.hoverScheduled = false;
      if (this.disposed) return;
      const pt = this.pendingHover;
      this.pendingHover = null;
      if (!pt) return;
      const hit = this.hitAt(pt.x, pt.y);
      const store = sketchSelectionStore.getState();
      const same =
        (hit === null && store.hover === null) ||
        (hit !== null && store.hover !== null && sameSketchSel(hit, store.hover));
      if (!same) store.setHover(hit);
      // Trim tool: overlay the destructive doomed-piece ghost for the hovered entity
      // (a miss / off-entity hover clears it).
      if (this.trimActive) this.renderTrimGhost(pt.x, pt.y, hit);
      // Fillet / Offset: overlay the ADDITIVE ghost of what the click would author,
      // on the ordinary preview lane (nothing is destroyed, so the trim ghost's
      // destructive styling would read wrong).
      if (this.filletActive) this.renderFilletGhost(pt.x, pt.y);
      if (this.offsetActive) this.renderOffsetGhost(pt.x, pt.y);
    });
  }

  /** Draw the destructive trim ghost for the entity under the cursor (Trim hover):
   *  the doomed sub-piece the click would remove, or the whole entity when there is
   *  nothing to trim (no qualifying crossings). Cleared on a miss. */
  private renderTrimGhost(clientX: number, clientY: number, hit: SketchSel | null): void {
    if (!hit) {
      this.deps.engine.setSketchTrimGhost(null);
      return;
    }
    const session = sketchStore.getState().session;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    const target = session?.entities.find((e) => e.id === hit.entityId);
    if (!session || !raw || !target) {
      this.deps.engine.setSketchTrimGhost(null);
      return;
    }
    const others = session.entities.filter((e) => e.id !== target.id);
    const doomed = trimPreview(target, raw, others, { minSize: 4 * this.deps.engine.planePixelWorld() });
    // null ⇒ the whole entity is doomed: ghost its own polyline.
    this.deps.engine.setSketchTrimGhost(doomed ?? entityToDraft(target));
  }

  private onSelectPointerUp = (e: PointerEvent): void => {
    const button = this.downButton;
    this.downButton = -1;
    if (this.dragging) {
      const pt = this.deps.engine.screenToPlane(e.clientX, e.clientY);
      void this.finishDrag(pt ? [pt.x, pt.y] : undefined, false);
      return;
    }
    if (this.dragStarting) {
      // beginGesture still in flight: defer the commit until it resolves.
      const pt = this.deps.engine.screenToPlane(e.clientX, e.clientY);
      this.dragEndPending = { target: pt ? [pt.x, pt.y] : undefined, restore: false };
      return;
    }
    // Marquee past DRAG_PX → resolve the box; sub-threshold → tear it down and fall
    // through to the unchanged click path (which clears / toggles as before).
    if (this.marqueeActive) {
      this.finishMarquee(e);
      return;
    }
    if (this.marqueeArmed) this.cancelMarquee();
    // Not a drag: restore orbit if a handle armed it, then click-select if it was a click.
    if (this.dragArmed) {
      this.deps.engine.setSketchDrawingActive(false);
      this.dragArmed = null;
    }
    const wasClick =
      button === 0 &&
      e.button === 0 &&
      !this.moved &&
      Math.abs(e.clientX - this.downX) <= DRAG_PX &&
      Math.abs(e.clientY - this.downY) <= DRAG_PX;
    if (!wasClick) return;
    const hit = this.hitAt(e.clientX, e.clientY);
    const additive = e.shiftKey || e.metaKey;
    const current = sketchSelectionStore.getState().selected;
    sketchSelectionStore.getState().set(clickSelection(current, hit, additive));
  };

  /** First move past DRAG_PX on an armed handle: open the backend gesture. */
  private async beginSelectDrag(clientX: number, clientY: number): Promise<void> {
    const session = sketchStore.getState().session;
    const armed = this.dragArmed;
    if (!session || !armed || this.dragStarting || this.dragging) return;
    this.dragStarting = true;
    this.dragBase = session.entities;
    this.dragPlane = session.plane;
    this.dragStatus = session.status;
    this.dragLastSeq = 0;
    this.dragAccum = {};
    try {
      await this.deps.client.beginGesture(session.sketchId, armed.pointRef);
    } catch (err) {
      viewportStore.getState().setStatusHint(`Drag failed: ${sketchErr(err)}`, { severity: "error", sticky: true });
      this.resetDrag();
      this.deps.engine.setSketchDrawingActive(false);
      return;
    }
    // A tool/mode change during the await tore the gesture down: close it cleanly.
    if (!this.selectActive || this.dragArmed !== armed) {
      void this.deps.client.endGesture().catch(() => {});
      return;
    }
    this.dragStarting = false;
    this.dragging = true;
    // A pointerup / Esc that landed before begin resolved deferred its end to here.
    const pending = this.dragEndPending;
    if (pending) {
      this.dragEndPending = null;
      void this.finishDrag(pending.target, pending.restore);
      return;
    }
    this.scheduleSelectSolve(clientX, clientY); // seed the first solve at the current pointer
  }

  /** Coalesce the latest drag target and fire one solveDrag per frame (latest-wins). */
  private scheduleSelectSolve(clientX: number, clientY: number): void {
    const pt = this.deps.engine.screenToPlane(clientX, clientY);
    if (!pt) return;
    this.pendingTarget = [pt.x, pt.y];
    if (this.solveScheduled) return;
    this.solveScheduled = true;
    requestAnimationFrame(() => {
      this.solveScheduled = false;
      if (this.disposed) return;
      const target = this.pendingTarget;
      this.pendingTarget = null;
      if (!target || !this.dragging) return;
      void this.fireSolve(target);
    });
  }

  /** One incremental drag solve; drop null/stale-seq responses, else preview the delta. */
  private async fireSolve(target: [number, number]): Promise<void> {
    let res;
    try {
      res = await this.deps.client.solveDrag(target);
    } catch {
      return;
    }
    if (!this.dragging || !shouldApplyDrag(this.dragLastSeq, res)) return;
    this.dragLastSeq = res!.seq;
    // Live conflict tint during the drag (store short-circuits when unchanged).
    sketchStore.getState().setConflicting(res!.conflicting ?? []);
    if (this.dragPlane) {
      this.dragAccum = { ...this.dragAccum, ...res!.positions };
      const moved = applySolvedPositions(this.dragBase, this.dragAccum);
      this.deps.engine.updateSketchSession(this.dragPlane, moved, this.dragStatus);
    }
  }

  /** Esc mid-drag: end the gesture at the ORIGINAL position (no explicit cancel verb
   *  on the wire — endGesture always commits ONE step) and restore pre-drag geometry. */
  private cancelSelectDrag(): void {
    const orig = this.draggedPointCoord();
    if (this.dragging) {
      void this.finishDrag(orig, true);
    } else if (this.dragStarting) {
      this.dragEndPending = { target: orig, restore: true };
    }
  }

  /** Plane coord of the dragged point in the pre-drag base (for the Esc restore target). */
  private draggedPointCoord(): [number, number] | undefined {
    const sel = this.dragArmed?.sel;
    if (!sel || !sel.point) return undefined;
    const e = this.dragBase.find((x) => x.id === sel.entityId);
    if (!e) return undefined;
    return entityPoints(e).find((p) => p.position === sel.point)?.coord;
  }

  /** Commit (or cancel-restore) the drag: endGesture → apply final positions to the
   *  session + engine + DOF badge; the selection stays on the dragged entity. */
  private async finishDrag(finalTarget: [number, number] | undefined, restore: boolean): Promise<void> {
    const gen = sketchStore.getState().sessionGeneration;
    const startSession = sketchStore.getState().session;
    // Pre-drag snapshot for undo (the store still holds the pre-drag arrays — the drag
    // preview only moved the engine, never setSession). A cancel (Esc) does NOT push.
    const before: SketchSnapshot | null = startSession
      ? { entities: startSession.entities, constraints: startSession.constraints }
      : null;
    const sel = this.dragArmed?.sel ?? null;
    const base = this.dragBase;
    const plane = this.dragPlane;
    this.resetDrag();
    this.deps.engine.setSketchDrawingActive(false); // restore LMB orbit

    let result;
    try {
      result = await this.deps.client.endGesture(finalTarget);
    } catch (err) {
      viewportStore.getState().setStatusHint(`Drag failed: ${sketchErr(err)}`, { severity: "error", sticky: true });
    }
    if (this.sessionStale(gen)) return; // exited / superseded / disposed during await
    const session = sketchStore.getState().session;
    if (!session || !plane) return;

    // On a cancel (Esc) restore the pre-drag geometry; else apply the committed delta.
    const positions = restore ? {} : result?.solvedPositions ?? {};
    const entities = applySolvedPositions(base, positions);
    const next: SketchSession = {
      ...session,
      entities,
      dof: result?.dof ?? session.dof,
      status: result?.status ?? session.status,
    };
    sketchStore.getState().setSession(next);
    // Unconditional: a failed endGesture reverts geometry to the pre-drag base, so a
    // conflict set left over from fireSolve's live frames would tint constraints that
    // are not conflicting in the DISPLAYED state.
    sketchStore.getState().setConflicting(result?.conflicting ?? []);
    this.deps.engine.updateSketchSession(next.plane, entities, next.status);
    if (result) this.pushSolve(session.sketchId, result.dof, result.status);
    if (sel) sketchSelectionStore.getState().set([sel]);
    if (!restore && before) sketchStore.getState().pushUndoSnapshot(before, { kind: "drag" });
  }

  /** Drop all in-flight drag state (idempotent). A pending solve rAF no-ops on `!dragging`. */
  private resetDrag(): void {
    this.dragArmed = null;
    this.dragStarting = false;
    this.dragging = false;
    this.dragLastSeq = 0;
    this.dragBase = [];
    this.dragPlane = null;
    this.pendingTarget = null;
    this.dragEndPending = null;
  }

  // ── Marquee / box select (W2-D) ─────────────────────────────────────────────
  //
  // Rightward drag = WINDOW (full containment, solid border); leftward = CROSSING
  // (touch, dashed border). See marqueeSelect.ts for the geometry + the legacy
  // divergence note.
  //
  // TEARDOWN. `setSketchDrawingActive` is a SHARED LMB-orbit suppression flag (armed
  // draw tools + modal model drags use it too), so every exit path below funnels
  // through `cancelMarquee` — pointerup (in-viewport and the window net), Esc,
  // pointercancel, a tool switch (`selectMachine`), session teardown, and dispose.
  // Missing one restores neither orbit nor the overlay, silently.

  /** rAF-coalesced overlay update: one DOM write per frame, mode re-derived live. */
  private scheduleMarquee(clientX: number, clientY: number): void {
    this.pendingMarquee = { x: clientX, y: clientY };
    if (this.marqueeScheduled) return;
    this.marqueeScheduled = true;
    requestAnimationFrame(() => {
      this.marqueeScheduled = false;
      if (this.disposed) return;
      const pt = this.pendingMarquee;
      this.pendingMarquee = null;
      if (!pt || !this.marqueeActive) return;
      this.marqueeMode = marqueeModeFor(this.downX, pt.x);
      this.drawMarquee(pt.x, pt.y);
    });
  }

  /** Position + restyle the overlay div for the current screen rect. */
  private drawMarquee(clientX: number, clientY: number): void {
    const el = this.ensureMarqueeOverlay();
    const box = this.deps.container.getBoundingClientRect();
    const left = Math.min(this.downX, clientX) - box.left;
    const top = Math.min(this.downY, clientY) - box.top;
    const s = el.style;
    s.left = `${left}px`;
    s.top = `${top}px`;
    s.width = `${Math.abs(clientX - this.downX)}px`;
    s.height = `${Math.abs(clientY - this.downY)}px`;
    s.borderStyle = this.marqueeMode === "crossing" ? "dashed" : "solid";
    el.dataset.marqueeMode = this.marqueeMode;
  }

  /** Lazily create the marquee div. Styling is tokens-only (no raw hex, no new
   *  tokens): accent border + an accent-derived translucent fill via `color-mix`. */
  private ensureMarqueeOverlay(): HTMLDivElement {
    if (this.marqueeEl) return this.marqueeEl;
    const el = document.createElement("div");
    el.dataset.sketchMarquee = "1";
    const s = el.style;
    s.position = "absolute";
    s.pointerEvents = "none";
    s.zIndex = "2"; // above the engine canvas (z0) and the HTML overlay layer (z1)
    s.boxSizing = "border-box";
    s.borderWidth = "1px";
    s.borderStyle = "solid";
    s.borderColor = "var(--color-accent)";
    s.backgroundColor = "color-mix(in srgb, var(--color-accent) 12%, transparent)";
    this.deps.container.appendChild(el);
    this.marqueeEl = el;
    return el;
  }

  /**
   * Map the drag's four SCREEN corners onto the sketch plane and take their AABB.
   *
   * APPROXIMATION. In the normal sketch pose (ortho, camera down the plane normal,
   * up = plane +Y — what `enterSketch`'s `viewAlongNormal` establishes) a screen rect
   * maps to an exactly axis-aligned plane rect, so the AABB is exact. If the user
   * orbits off-normal the rect maps to a general quadrilateral and the AABB is its
   * CONSERVATIVE superset — the box can then select slightly more than it visually
   * covers. Deliberate: a superset is recoverable (Shift-click off / re-drag), a
   * subset silently loses picks. Returns null when a corner does not hit the plane
   * (grazing ray / no live plane).
   */
  private planeMarqueeRect(upX: number, upY: number): MarqueeRect | null {
    const corners: Array<[number, number]> = [
      [this.downX, this.downY],
      [upX, this.downY],
      [upX, upY],
      [this.downX, upY],
    ];
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const [cx, cy] of corners) {
      const p = this.deps.engine.screenToPlane(cx, cy);
      if (!p) return null;
      minX = Math.min(minX, p.x);
      minY = Math.min(minY, p.y);
      maxX = Math.max(maxX, p.x);
      maxY = Math.max(maxY, p.y);
    }
    return marqueeRectFrom(minX, minY, maxX, maxY);
  }

  /** Release past DRAG_PX: resolve the box → selection (plain replaces, Shift/Meta
   *  UNIONs — never toggles, a box is an additive gesture), then tear down. */
  private finishMarquee(e: { clientX: number; clientY: number; shiftKey: boolean; metaKey: boolean }): void {
    const additive = e.shiftKey || e.metaKey;
    const session = sketchStore.getState().session;
    const rect = this.planeMarqueeRect(e.clientX, e.clientY);
    const mode = marqueeModeFor(this.downX, e.clientX);
    this.cancelMarquee(); // overlay gone + orbit restored before any store write
    if (!session || !rect) return;
    const picked: SketchSel[] = marqueeHits(session.entities, rect, mode).map((entityId) => ({ entityId }));
    const store = sketchSelectionStore.getState();
    if (!additive) {
      store.set(picked);
      return;
    }
    const current = store.selected;
    const added = picked.filter((p) => !current.some((s) => sameSketchSel(s, p)));
    if (added.length > 0) store.set([...current, ...added]);
  }

  /** Drop the marquee (idempotent): overlay removed + LMB orbit restored. */
  private cancelMarquee(): void {
    if (!this.marqueeArmed && !this.marqueeActive) return;
    this.marqueeArmed = false;
    this.marqueeActive = false;
    this.pendingMarquee = null;
    this.marqueeEl?.remove();
    this.marqueeEl = null;
    this.deps.engine.setSketchDrawingActive(false);
  }

  /** Window-level release net — see the constructor comment. No-ops when the
   *  container's own pointerup already finished the marquee. */
  private onWindowPointerUp = (e: PointerEvent): void => {
    if (!this.marqueeArmed && !this.marqueeActive) return;
    this.downButton = -1;
    if (this.marqueeActive) this.finishMarquee(e);
    else this.cancelMarquee();
  };

  /** OS/browser-level pointer loss (touch cancel, capture stolen): drop the box. */
  private onWindowPointerCancel = (): void => {
    this.cancelMarquee();
  };

  /**
   * W1-B: apply the sticky construction draw modifier to a tool machine's output.
   * The machines are mode-blind (they already mark their OWN helper drafts, e.g. the
   * arc radius rubber-band), so the flag is OR-ed on here — the single place drafts
   * cross from the FSM into the engine / the commit path. Off ⇒ the same array back
   * (no churn). Applied to preview + committed so what the user sees dashed is
   * exactly what gets authored.
   */
  private decorate(drafts: DraftEntity[]): DraftEntity[] {
    if (!sketchStore.getState().constructionMode) return drafts;
    return drafts.map((d) => (d.construction ? d : { ...d, construction: true }));
  }

  private updateGhost(preview: DraftEntity[], cursor: Point2): void {
    const line = preview.find((d) => d.type === "Line" && !d.construction && d.p0 && d.p1);
    if (!line || !line.p0 || !line.p1) {
      this.deps.engine.setSketchGhost(null, null);
      return;
    }
    const hv = inferHVDraft(line.p0, line.p1);
    this.deps.engine.setSketchGhost(hv, hv ? cursor : null);
  }

  // ── keyboard (Alt suppress + Enter/Esc end chain) ─────────────────────────

  private onKeyDown = (e: KeyboardEvent): void => {
    // DOM FOCUS IS THE PRIORITY RULE (SP-1 §7.5), and this listener is
    // capture-phase on window — so a key aimed at a text field (a live dimension
    // chip, the dimension chip, an inspector input) has to be handed back before
    // ANY branch below can swallow it. Enter, Escape, Tab and every digit then
    // belong to that field, with zero keymap changes.
    if (isEditableKeyTarget(e.target)) return;
    // A digit over the viewport OPENS a chip. Claimed before the polygon
    // side-count branch, which now only owns the IDLE phase.
    if (this.tryStartLiveDimTyping(e)) return;
    if (e.key === "Escape" && this.planePicking) {
      // Cancel plane pick → back to model mode (exit() hides the picker).
      toolStore.getState().setMode("model");
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Alt") this.altHeld = true;
    // Polygon side count: 3-9 belong to the armed polygon machine, so they are
    // claimed HERE (capture phase) before the global keymap ladder ever sees them.
    // ARMED + live dimensions on, a digit is the RADIUS chip's (already claimed
    // above) — so this branch owns the idle phase only. With the pref off it
    // still owns both, because then there is no chip to hand the key to.
    const sidesPhase = this.machineState?.anchors.length === 0 || !this.liveDimsEnabled();
    if (this.machine?.id === "polygon" && this.machineState && sidesPhase && POLYGON_SIDES_KEY.test(e.key)) {
      const stepped = this.machine.step(this.machineState, { kind: "sides", n: Number(e.key) }, this.stepCtx());
      logSketchStep(this.machine.id, "sides", stepped);
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview(this.decorate(stepped.preview));
      this.updatePolygonHint();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && this.selectActive && (this.marqueeActive || this.marqueeArmed)) {
      // Cancel the in-flight box select here (overlay + orbit restored); don't let the
      // global Esc ladder also switch tools / leave sketch mode.
      this.cancelMarquee();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && this.selectActive && (this.dragging || this.dragStarting)) {
      // Cancel the in-flight drag here; don't let the global Esc ladder run.
      this.cancelSelectDrag();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && this.filletActive && this.filletFirstPick) {
      // Cancel the half-made two-pick here; the SECOND Esc falls through to the
      // global ladder and leaves the tool (same rung order as the dimension tool).
      this.filletFirstPick = null;
      sketchSelectionStore.getState().clear();
      this.deps.engine.setSketchPreview([]);
      this.updateFilletHint();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && this.dimensionActive && (this.dimState.ready || this.dimState.pending)) {
      // Cancel the in-flight dimension here; don't let the global Esc ladder run.
      this.cancelDimension();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Enter" && this.machine && this.machineState && this.machineState.anchors.length > 0) {
      // (A focused text input — the dimension chip, a live dimension chip — has
      // already been handed its Enter by the editable-target guard at the top.)
      // A chain is in progress: Enter ends it here (mirrors the Esc-chain branch
      // below). No anchors ⇒ this branch is skipped and the global finishSketch
      // shortcut (useShortcuts) handles Enter instead.
      const stepped = this.machine.step(this.machineState, { kind: "esc" }, this.stepCtx());
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview([]);
      this.deps.engine.setSketchGhost(null, null);
      this.clearLiveDimGesture();
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && this.machine && this.machineState && this.machineState.anchors.length > 0) {
      // A gesture is in progress: end the chain here, and DON'T let the global
      // Esc ladder also switch tools (capture-phase intercept).
      const stepped = this.machine.step(this.machineState, { kind: "esc" }, this.stepCtx());
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview([]);
      this.deps.engine.setSketchGhost(null, null);
      this.clearLiveDimGesture();
      e.stopPropagation();
      e.preventDefault();
    }
  };

  private onKeyUp = (e: KeyboardEvent): void => {
    if (e.key === "Alt") this.altHeld = false;
  };

  dispose(): void {
    this.disposed = true; // set FIRST: guards every pending rAF + queued write-back
    this.snapCache = null;
    this.snapCacheKey = null;
    this.clearLiveDimGesture();
    this.pendingSwitchId = null;
    this.endPlanePick();
    sketchSelectionStore.getState().clear();
    this.endSketchEditTool();
    if (this.dimensionActive) this.cancelDimension();
    // An in-flight drag gesture must be closed on the wire (endGesture always commits
    // ONE step; there is no cancel verb) so the worker doesn't leak an open gesture.
    if (this.dragging || this.dragStarting) void this.deps.client.endGesture().catch(() => {});
    this.resetDrag();
    this.cancelMarquee();
    this.marqueeEl?.remove(); // belt-and-braces: an overlay with no live marquee
    this.marqueeEl = null;
    sketchStore.getState().clearSketchUndo();
    const session = sketchStore.getState().session;
    if (session) {
      void this.deps.client.cancelSketch(session.sketchId);
      sketchStore.getState().setSession(null);
    }
    const c = this.deps.container;
    c.style.cursor = "";
    c.removeEventListener("pointerdown", this.onPointerDown);
    c.removeEventListener("pointermove", this.onPointerMove);
    c.removeEventListener("pointerup", this.onPointerUp);
    window.removeEventListener("pointerup", this.onWindowPointerUp);
    window.removeEventListener("pointercancel", this.onWindowPointerCancel);
    window.removeEventListener("keydown", this.onKeyDown, true);
    window.removeEventListener("keyup", this.onKeyUp, true);
    for (const u of this.unsubs) u();
    this.unsubs.length = 0;
  }
}

/** A key event aimed at a text field. The controller's keydown is CAPTURE-phase on
 *  window, so it would otherwise swallow keys before the input's own handler runs.
 *  Mirrors `isEditableTarget` in useShortcuts.ts. */
function isEditableKeyTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
  );
}

/** A raw engine pick → a {@link FacePickTarget}, or null when it is not a face.
 *  Typed structurally so the controller never imports Three.js (`worldPos` is a
 *  `THREE.Vector3` on the engine side). */
function facePickFrom(
  hit: { bodyId: string; kind: string; topoKey: string; elementId?: string; worldPos: { x: number; y: number; z: number } } | null,
): FacePickTarget | null {
  if (!hit || hit.kind !== "face") return null;
  return {
    bodyId: hit.bodyId,
    topoKey: hit.topoKey,
    elementId: hit.elementId,
    worldPoint: [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z],
  };
}

/** Human message from a rejected backend sketch call. */
function sketchErr(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** A document-mm length rendered in the user's DISPLAY unit, for a hint. */
function lengthHint(mm: number): string {
  const unit = settingsStore.getState().displayUnit;
  return `${formatLength(mm, unit)} ${lengthSuffix(unit)}`;
}

/** Phrase a typed fillet refusal (WP-C T2b). Every arm NAMES what went wrong —
 *  a silent no-op reads as a broken tool. */
function filletFailureHint(f: FilletFailure): string {
  switch (f.reason) {
    case "notACorner":
      return "Fillet needs two lines that share a corner";
    case "collinear":
      return "Those lines are collinear — there is no corner to round";
    case "degenerate":
      return "That line is too short to fillet";
    case "badRadius":
      return "Fillet radius must be greater than zero";
    case "radiusTooLarge":
      return `Radius too large — this corner takes at most ${lengthHint(f.maxRadius)}`;
  }
}

/** Phrase a typed offset refusal (WP-C T2b). */
function offsetFailureHint(f: OffsetFailure): string {
  switch (f.reason) {
    case "unsupported":
      return "Offset works on lines, arcs and circles";
    case "badDistance":
      return "Offset distance must be greater than zero";
    case "arcCollapsed":
      return `Offset too large — an arc collapses beyond ${lengthHint(f.maxDistance)}`;
    case "segmentVanished":
      return "Offset too large — a segment of the chain would vanish";
  }
}

/** H/V inference for the ghost glyph — delegates to the shared `inferHV` (one source
 *  of truth for the ±5° tolerance) and maps its tokens to the glyph's short codes. */
function inferHVDraft(p0: Point2, p1: Point2): "H" | "V" | null {
  const hv = inferHV([p0.x, p0.y], [p1.x, p1.y]);
  return hv === "Horizontal" ? "H" : hv === "Vertical" ? "V" : null;
}
