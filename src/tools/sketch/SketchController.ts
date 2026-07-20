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
} from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { chooseGridStep } from "@/viewport/engine/GridPlane";
import { toolStore } from "@/stores/toolStore";
import { viewportStore, type Projection } from "@/stores/viewportStore";
import { documentStore, docSketchStatus, nextSketchName } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { sketchSelectionStore, type SketchSel } from "@/stores/sketchSelectionStore";
import { settingsStore } from "@/stores/settingsStore";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { applySolvedPositions } from "@/ipc/sketchWireMap";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { computeSnap, type SnapResult } from "./snapEngine";
import { inferConstraints, entityPoints } from "./autoConstrain";
import { commitDimensionConstraint, deleteEntities } from "./sketchService";
import { hitTestSketch } from "./sketchHitTest";
import { clickSelection, dragIntent, shouldApplyDrag, type DragIntent } from "./selectGesture";
import { mirrorEntities } from "./mirrorMath";
import {
  dimensionInit,
  dimensionStep,
  buildDimensionConstraint,
  dimensionSuffix,
  pickDimensionTarget,
  type DimState,
} from "./dimensionTool";
import {
  TOOL_MACHINES,
  draftToEntityFields,
  type DraftEntity,
  type ToolMachine,
  type ToolState,
} from "./toolMachine";

const DRAG_PX = 4;

export interface SketchControllerDeps {
  engine: ViewportEngine;
  client: CadClient;
  container: HTMLElement;
}

export class SketchController {
  private machine: ToolMachine | null = null;
  private machineState: ToolState | null = null;
  private lastSnap: SnapResult | null = null;
  private altHeld = false;

  // Dimension tool (non-drawing): a pick-accumulator FSM + its open chip.
  private dimensionActive = false;
  private dimState: DimState = dimensionInit();
  private priorProjection: Projection | null = null;
  private entering = false;
  // Plane-pick phase: bare sketch entry shows the plane picker; a click on a
  // quad creates the sketch on that plane and opens the normal session.
  private planePicking = false;

  private downX = 0;
  private downY = 0;
  private downButton = -1;
  private moved = false;
  private pendingMove: PointerEvent | null = null;
  private moveScheduled = false;

  // Trim / Mirror tools (non-drawing, click-only). Parallel lanes to `selectActive`,
  // each gated STRICTLY on its tool id so the select/draw/dimension paths are untouched.
  private trimActive = false;
  private mirrorActive = false;

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
  private pendingTarget: [number, number] | null = null; // coalesced latest drag target
  private solveScheduled = false;
  // A pointerup / Esc that arrived while beginGesture was still in flight — its end
  // is deferred here so a fast flick never double-commits the gesture.
  private dragEndPending: { target?: [number, number]; restore: boolean } | null = null;

  private readonly unsubs: Array<() => void> = [];

  constructor(private readonly deps: SketchControllerDeps) {
    const c = deps.container;
    c.addEventListener("pointerdown", this.onPointerDown);
    c.addEventListener("pointermove", this.onPointerMove);
    c.addEventListener("pointerup", this.onPointerUp);
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
      // No target ⇒ bare "new sketch" intent: pick a plane first.
      if (!activeId) {
        this.beginPlanePick();
        return;
      }
      const opened = await this.openSession(activeId);
      if (!opened && toolStore.getState().mode === "sketch") {
        // Existing-sketch entry failed: fall back to model mode instead of
        // stranding sketch chrome with no session. exit() clears the status
        // hint, so re-set the failure message after the mode flip.
        const hint = viewportStore.getState().statusHint;
        toolStore.getState().setMode("model");
        viewportStore.getState().setStatusHint(hint);
      }
    } finally {
      this.entering = false;
    }
  }

  /** Enter the client session for `target` and wire it into the stores + engine.
   *  Returns false when the client rejected (session not opened). */
  private async openSession(target: EnterSketchTarget): Promise<boolean> {
    let session: SketchSession;
    try {
      session = await this.deps.client.enterSketch(target);
    } catch (e) {
      console.error("[sketch] enterSketch failed", target, e);
      viewportStore.getState().setStatusHint(`Enter sketch failed: ${sketchErr(e)}`);
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
          .catch((e) => console.error("[sketch] orphan cleanup failed", e));
      }
      return true; // the session opened; the user just left
    }

    // A freshly created sketch (plane pick) isn't in the tree yet — register it,
    // then make it the active + selected sketch so the chrome + inspector bind.
    viewportStore.getState().setActiveSketch(session.sketchId);
    const sketches = documentStore.getState().sketches;
    if (!sketches[session.sketchId]) {
      documentStore.getState().addSketch({
        id: session.sketchId,
        name: nextSketchName(sketches),
        visible: true,
        dof: session.dof,
        status: docSketchStatus(session.status),
      });
    }
    selectionStore.getState().set([{ kind: "sketch", id: session.sketchId }]);

    sketchStore.getState().setSession(session);
    this.pushSolve(session.sketchId, session.dof, session.status);

    this.priorProjection = viewportStore.getState().projection;
    this.deps.engine.enterSketch(session.plane, session.entities, session.status);
    viewportStore.getState().setProjection("ortho");

    this.selectMachine(toolStore.getState().sketchTool);
    return true;
  }

  /** Show the plane picker and prompt; a quad click resolves via confirmPlanePick. */
  private beginPlanePick(): void {
    this.planePicking = true;
    this.deps.engine.setPlanePickerVisible(true);
    viewportStore.getState().setStatusHint("Select a plane to start the sketch — Esc to cancel");
  }

  /** Reverse beginPlanePick (idempotent): hide the picker + clear its hint. */
  private endPlanePick(): void {
    if (!this.planePicking) return;
    this.planePicking = false;
    this.deps.engine.setPlanePickerVisible(false);
    viewportStore.getState().setStatusHint(null);
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
    }
  }

  private exit(): void {
    sketchSelectionStore.getState().clear();
    this.endPlanePick();
    this.machine = null;
    this.machineState = null;
    this.lastSnap = null;
    if (this.dimensionActive) this.cancelDimension();
    this.dimensionActive = false;
    this.resetDrag();
    this.selectActive = false;
    this.trimActive = false;
    this.mirrorActive = false;
    this.deps.engine.setSketchDrawingActive(false);
    this.deps.engine.setSketchPreview([]);
    this.deps.engine.setSketchSnap(null, false);
    this.deps.engine.setSketchGhost(null, null);
    this.deps.engine.exitSketch();
    viewportStore.getState().setStatusHint(null);
    if (this.priorProjection) {
      viewportStore.getState().setProjection(this.priorProjection);
      this.priorProjection = null;
    }
    const session = sketchStore.getState().session;
    if (session) void this.deps.client.cancelSketch(session.sketchId);
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
      sketchSelectionStore.getState().clear();
    }
    // Leaving the mirror tool drops its in-progress pick set (the mirror phase is
    // derived from the sketch selection — a stale set would leak into the next tool).
    if (this.mirrorActive && tool !== "mirror") {
      sketchSelectionStore.getState().clear();
    }

    const m = TOOL_MACHINES[tool] ?? null;
    this.machine = m;
    this.machineState = m ? m.init() : null;
    this.dimensionActive = tool === "dimension";
    this.selectActive = tool === "select";
    this.trimActive = tool === "trim";
    this.mirrorActive = tool === "mirror";
    // The dimension tool owns the pointer (no orbit) so clicks pick entities.
    this.deps.engine.setSketchDrawingActive(!!m || this.dimensionActive);
    this.deps.engine.setSketchPreview([]);
    this.deps.engine.setSketchGhost(null, null);
    if (!m && !this.dimensionActive) this.deps.engine.setSketchSnap(null, false);

    if (this.dimensionActive) {
      this.dimState = dimensionInit();
      viewportStore.getState().setStatusHint("Dimension — click a line, circle, arc, or two points");
      return;
    }
    if (this.trimActive) {
      viewportStore.getState().setStatusHint("Click an entity to delete · Esc to exit");
      return;
    }
    if (this.mirrorActive) {
      this.updateMirrorHint();
      return;
    }
    viewportStore.getState().setStatusHint(null);
  }

  // ── pointer handling ────────────────────────────────────────────────────

  private snapAt(clientX: number, clientY: number): SnapResult | null {
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return null;
    const session = sketchStore.getState().session;
    const settings = settingsStore.getState();
    return computeSnap(raw, session?.entities ?? [], {
      gridStep: chooseGridStep(this.deps.engine.getCameraDistance()).minor,
      pixelWorld: this.deps.engine.planePixelWorld(),
      enableGrid: settings.snapTo.grid,
      enableGuideLines: settings.snapTo.sketchGuideLines,
      enableGuidePoints: settings.snapTo.sketchGuidePoints,
      enableQuadrant: settings.snapTo.quadrant,
      enableIntersection: settings.snapTo.intersection,
      enableOnCurve: settings.snapTo.onCurve,
      suppress: this.altHeld,
      recentPoints: this.machineState?.anchors ?? [],
    });
  }

  private onPointerMove = (e: PointerEvent): void => {
    // Plane-pick phase owns the pointer: highlight the plane under the cursor.
    if (this.planePicking) {
      this.deps.engine.planePickerHover(e.clientX, e.clientY);
      return;
    }
    if (this.selectActive) {
      this.onSelectPointerMove(e);
      return;
    }
    // Trim / Mirror are click tools; a move past DRAG_PX with LMB held is an orbit,
    // not a click — track it so pointerup doesn't fire a stray delete/pick.
    if (this.trimActive || this.mirrorActive) {
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
      const stepped = this.machine.step(this.machineState, { kind: "move", pt: snap.point });
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview(stepped.preview);
      this.updateGhost(stepped.preview, snap.point);
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
      const kind = this.deps.engine.planePickerHitTest(e.clientX, e.clientY);
      if (kind) void this.confirmPlanePick(kind);
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
    if (!this.machine || !this.machineState) return;
    const snap = this.snapAt(e.clientX, e.clientY) ?? this.lastSnap;
    if (!snap) return;
    const stepped = this.machine.step(this.machineState, { kind: "click", pt: snap.point });
    this.machineState = stepped.state;
    this.deps.engine.setSketchPreview(stepped.preview);
    if (stepped.committed && stepped.committed.length > 0) {
      void this.commit(stepped.committed);
    }
    if (stepped.done) this.deps.engine.setSketchGhost(null, null);
  };

  // ── commit round-trip ─────────────────────────────────────────────────────

  private async commit(committed: DraftEntity[]): Promise<void> {
    const store = sketchStore.getState();
    const session = store.session;
    if (!session) return;

    const newEntities: SketchEntity[] = committed.map((d) => ({
      id: store.nextEntityId(),
      ...draftToEntityFields(d),
    }));
    const newConstraints: SketchConstraint[] = inferConstraints(newEntities, session.entities, {
      nextConstraintId: () => sketchStore.getState().nextConstraintId(),
    });

    const entities = [...session.entities, ...newEntities];
    const constraints = [...session.constraints, ...newConstraints];

    let result;
    try {
      result = await this.deps.client.sketchUpsert(session.sketchId, entities, constraints);
    } catch (e) {
      viewportStore.getState().setStatusHint(`Sketch solve failed: ${sketchErr(e)}`);
      return;
    }
    // A late exit could have cleared the session mid-await.
    if (!sketchStore.getState().session) return;

    // F-WP9: the solver may have MOVED points (constraint-driven); write the
    // solved positions back into the geometry (backend point UUIDs were already
    // reverse-mapped to `entityId.Position` keys by the client). No-op when the
    // solve returned no movement (identity upsert) — same array reference.
    const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});

    const next: SketchSession = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
    sketchStore.getState().setSession(next);
    this.deps.engine.updateSketchSession(next.plane, solvedEntities, next.status);
    this.pushSolve(session.sketchId, result.dof, result.status);
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
    const tol = 8 * this.deps.engine.planePixelWorld(); // same 8px reach as snapping
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
      viewportStore.getState().setStatusHint("Dimension — pick a second point");
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
      const { rejected } = await commitDimensionConstraint(this.deps.client, constraint);
      viewportStore
        .getState()
        .setStatusHint(rejected ? "Dimension removed — it would over-constrain the sketch" : null);
    } catch (e) {
      viewportStore.getState().setStatusHint(`Dimension failed: ${sketchErr(e)}`);
    }
  }

  /** Esc / tool change / empty click: drop the in-flight dimension + chip. */
  private cancelDimension(): void {
    this.dimState = dimensionInit();
    toolChipStore.getState().clear();
    if (this.dimensionActive) {
      viewportStore.getState().setStatusHint("Dimension — click a line, circle, arc, or two points");
    }
  }

  // ── Trim tool ────────────────────────────────────────────────────────────────
  //
  // Click-to-delete (ports OneCAD-CPP TrimTool: "click to delete entire entity").
  // A click that hits an entity (body OR point pick) deletes that entity via the
  // shared `deleteEntities` service (which cascades its referencing constraints);
  // a miss is a no-op. Esc falls through to the global ladder → back to select.

  private handleTrimClick(clientX: number, clientY: number): void {
    const hit = this.hitAt(clientX, clientY);
    if (!hit) return; // miss → no-op
    void deleteEntities(this.deps.client, [hit.entityId]);
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
      );
  }

  /** Reflect `sources` across the axis line, then ONE sketchUpsert (session + copies,
   *  session constraints + the new Symmetric/Equal). Selection moves to the copies. */
  private async performMirror(sources: SketchEntity[], axis: SketchEntity): Promise<void> {
    const session = sketchStore.getState().session;
    if (!session || !axis.p0 || !axis.p1) return;
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
      viewportStore.getState().setStatusHint(`Mirror failed: ${sketchErr(e)}`);
      return;
    }
    if (!sketchStore.getState().session) return; // exited during await

    const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});
    const next: SketchSession = {
      ...session,
      entities: solvedEntities,
      constraints,
      dof: result.dof,
      status: result.status,
    };
    sketchStore.getState().setSession(next);
    this.deps.engine.updateSketchSession(next.plane, solvedEntities, next.status);
    this.pushSolve(session.sketchId, result.dof, result.status);
    // Selection switches to the mirrored copies (body picks) → back in Phase B, repeatable.
    sketchSelectionStore.getState().set(mirrored.map((e) => ({ entityId: e.id })));
    viewportStore
      .getState()
      .setStatusHint(
        `Mirrored ${sources.length} ${sources.length === 1 ? "entity" : "entities"} · click another axis or Esc to exit`,
      );
  }

  // ── Select tool ─────────────────────────────────────────────────────────────
  //
  // FLOW. pointerdown resolves a hit; a DRAGGABLE handle arms a drag (and suppresses
  // LMB orbit); anything else stays a candidate click. On pointerup:
  //   - a real click (< DRAG_PX, no move) → click-select (Shift/Meta toggles, plain
  //     replaces, a miss clears);
  //   - a drag past DRAG_PX on an armed handle → beginGesture → solveDrag (fire-and-
  //     forget, latest-wins by seq) → endGesture (ONE undo) + the selection stays.
  // Esc mid-drag ends the gesture at the ORIGINAL point + restores pre-drag geometry.

  /** hitTest the current session at a client point (same 8px reach the dimension tool uses). */
  private hitAt(clientX: number, clientY: number): SketchSel | null {
    const session = sketchStore.getState().session;
    if (!session) return null;
    const raw = this.deps.engine.screenToPlane(clientX, clientY);
    if (!raw) return null;
    const tol = 8 * this.deps.engine.planePixelWorld();
    return hitTestSketch(raw, session.entities, tol);
  }

  private onSelectPointerDown = (e: PointerEvent): void => {
    if (e.button !== 0) return;
    const session = sketchStore.getState().session;
    const hit = this.hitAt(e.clientX, e.clientY);
    const intent = dragIntent(hit, session?.entities ?? []);
    this.dragArmed = intent;
    // Suppress LMB orbit for a potential handle drag; a plain click restores it on up.
    if (intent) this.deps.engine.setSketchDrawingActive(true);
  };

  private onSelectPointerMove = (e: PointerEvent): void => {
    // Only care about a primary-button drag (LMB held) initiated in the viewport.
    if (this.downButton !== 0 || (e.buttons & 1) === 0) return;
    const far =
      Math.abs(e.clientX - this.downX) > DRAG_PX || Math.abs(e.clientY - this.downY) > DRAG_PX;
    if (far) this.moved = true;
    if (this.dragging || this.dragStarting) {
      this.scheduleSelectSolve(e.clientX, e.clientY);
      return;
    }
    if (this.dragArmed && far) void this.beginSelectDrag(e.clientX, e.clientY);
  };

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
    try {
      await this.deps.client.beginGesture(session.sketchId, armed.pointRef);
    } catch (err) {
      viewportStore.getState().setStatusHint(`Drag failed: ${sketchErr(err)}`);
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
    if (this.dragPlane) {
      const moved = applySolvedPositions(this.dragBase, res!.positions);
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
    const sel = this.dragArmed?.sel ?? null;
    const base = this.dragBase;
    const plane = this.dragPlane;
    this.resetDrag();
    this.deps.engine.setSketchDrawingActive(false); // restore LMB orbit

    let result;
    try {
      result = await this.deps.client.endGesture(finalTarget);
    } catch (err) {
      viewportStore.getState().setStatusHint(`Drag failed: ${sketchErr(err)}`);
    }
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
    this.deps.engine.updateSketchSession(next.plane, entities, next.status);
    if (result) this.pushSolve(session.sketchId, result.dof, result.status);
    if (sel) sketchSelectionStore.getState().set([sel]);
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

  private updateGhost(preview: DraftEntity[], cursor: Point2): void {
    const line = preview.find((d) => d.type === "Line" && !d.construction && d.p0 && d.p1);
    if (!line || !line.p0 || !line.p1) {
      this.deps.engine.setSketchGhost(null, null);
      return;
    }
    const hv = inferHVDraft(line.p0, line.p1);
    this.deps.engine.setSketchGhost(hv, hv ? cursor : null);
  }

  // ── keyboard (Alt suppress + Esc ends chain) ──────────────────────────────

  private onKeyDown = (e: KeyboardEvent): void => {
    if (e.key === "Escape" && this.planePicking) {
      // Cancel plane pick → back to model mode (exit() hides the picker).
      toolStore.getState().setMode("model");
      e.stopPropagation();
      e.preventDefault();
      return;
    }
    if (e.key === "Alt") this.altHeld = true;
    if (e.key === "Escape" && this.selectActive && (this.dragging || this.dragStarting)) {
      // Cancel the in-flight drag here; don't let the global Esc ladder run.
      this.cancelSelectDrag();
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
    if (e.key === "Escape" && this.machine && this.machineState && this.machineState.anchors.length > 0) {
      // A gesture is in progress: end the chain here, and DON'T let the global
      // Esc ladder also switch tools (capture-phase intercept).
      const stepped = this.machine.step(this.machineState, { kind: "esc" });
      this.machineState = stepped.state;
      this.deps.engine.setSketchPreview([]);
      this.deps.engine.setSketchGhost(null, null);
      e.stopPropagation();
      e.preventDefault();
    }
  };

  private onKeyUp = (e: KeyboardEvent): void => {
    if (e.key === "Alt") this.altHeld = false;
  };

  dispose(): void {
    this.endPlanePick();
    const c = this.deps.container;
    c.removeEventListener("pointerdown", this.onPointerDown);
    c.removeEventListener("pointermove", this.onPointerMove);
    c.removeEventListener("pointerup", this.onPointerUp);
    window.removeEventListener("keydown", this.onKeyDown, true);
    window.removeEventListener("keyup", this.onKeyUp, true);
    for (const u of this.unsubs) u();
    this.unsubs.length = 0;
  }
}

/** Human message from a rejected backend sketch call. */
function sketchErr(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Local H/V inference for the ghost glyph (mirrors autoConstrain thresholds). */
function inferHVDraft(p0: Point2, p1: Point2): "H" | "V" | null {
  const HV = (5 * Math.PI) / 180;
  if (p0.x === p1.x && p0.y === p1.y) return null;
  const a = Math.atan2(p1.y - p0.y, p1.x - p0.x);
  const hDev = Math.min(Math.abs(a), Math.abs(Math.abs(a) - Math.PI));
  if (hDev <= HV) return "H";
  const vDev = Math.abs(Math.abs(a) - Math.PI / 2);
  if (vDev <= HV) return "V";
  return null;
}
