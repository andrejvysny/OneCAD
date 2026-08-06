/*
 * Align face-to-face wiring (WP-B W2.5) — the pointer → registry → solve path.
 *
 * The math is pinned in `tools/preview/alignSolve`; what is only checkable HERE
 * is everything between a click and a placement, and each part fails silently:
 *
 *   - PICK VALIDATION. Pick 1 must land on a body being MOVED and pick 2 on one
 *     that is not. Both refusals are loud and both KEEP the phase — a two-pick
 *     flow that swallowed a bad click would leave the user reading an unchanged
 *     prompt with no idea why.
 *   - ONE HOVER WRITER. The face tint goes through `selectionStore.setHover`,
 *     the same store the select tool writes, and is cleared by whoever set it.
 *   - THE SOURCES STAY VISIBLE. The ghost hides the bodies being placed; during
 *     the picks those bodies are exactly what has to be clickable.
 *   - THE RECORD'S OWN FRAME. On a fold the visible mesh already carries the
 *     stored motion, so a frame read straight off the registry would compose
 *     onto a placement the align is supposed to replace.
 *
 * Geometry is REAL: two `mockMeshes` boxes in the real mesh registry, so the
 * face ordinals, ids and vertex data are the ones the app itself would resolve.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickHit } from "@/viewport/engine/Picker";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, OperationOp, TransformBodyParams } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { buildBodyObjects, swap as swapMesh, __resetRegistryForTests } from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh, makeCylinderMesh } from "@/ipc/mockMeshes";

/** The seed box: 80×60×30 centred on the origin (pivot (0,0,0), +X face at 40). */
const MOVER = "body1";
/** A 40³ box at (110,0,0) — the mock's STEP-import stand-in. Its −Y face is at −20. */
const DEST = "body2";
/** A tessellated cylinder: `f:0` is the curved wall, `f:1`/`f:2` the planar caps. */
const ROUND = "body3";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: MOVER, meshKey: `${MOVER}#0` }],
  removedBodies: [],
});

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function makeEngineMock() {
  return {
    /** What the next `probePick` reports — the test's stand-in for the raycast. */
    hit: null as PickHit | null,
    showGhostPreview: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    probePick(this: { hit: PickHit | null }): PickHit | null {
      return this.hit;
    },
    hideExtrudePreview: vi.fn(),
    clearPreviewBody: vi.fn(),
    setPreviewTint: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideRegionPick: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    hitExtrudeHandle: vi.fn(() => false),
    showTransformGizmo: vi.fn(),
    hideTransformGizmo: vi.fn(),
    setTransformGizmoHover: vi.fn(),
    setTransformGizmoActive: vi.fn(),
    hitTransformGizmo: vi.fn(() => null),
    screenRay: (x: number, y: number) => ({ origin: [x, -y, 1000], dir: [0, 0, -1] }),
  };
}

function makeClientMock() {
  return {
    onPreviewResult: vi.fn(() => () => {}),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve<Record<string, unknown>>({})),
    canFoldTransform: vi.fn(() => Promise.resolve<string | null>(null)),
  };
}

/** A face hit as `resolvePick` would build it (only these fields are read). */
const faceHit = (bodyId: string, topoKey: string): PickHit =>
  ({
    bodyId,
    kind: "face",
    topoKey,
    distance: 10,
    worldPos: { x: 0, y: 0, z: 0 },
  }) as unknown as PickHit;

const hintText = (): string => {
  const h = viewportStore.getState().statusHint as { message?: string } | string | null;
  return (typeof h === "string" ? h : (h?.message ?? "")) ?? "";
};

describe("ModelToolController align (face-to-face)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  beforeEach(() => {
    resetStores();
    __resetRegistryForTests();
    swapMesh(MOVER, buildBodyObjects(parseMeshPayload(makeBoxMesh()), MOVER, 1));
    swapMesh(DEST, buildBodyObjects(parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [110, 0, 0])), DEST, 1));
    swapMesh(ROUND, buildBodyObjects(parseMeshPayload(makeCylinderMesh()), ROUND, 1));
    container = document.createElement("div");
    document.body.appendChild(container);
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      // The align phase + the solved FREE axis have no other surface at all —
      // no chip can show a rotation about (0,0,1)·37.5° — so the assertions read
      // the same `?vpdebug` publication the e2e lane reads.
      debug: true,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    resetStores();
    __resetRegistryForTests();
  });

  async function armOn(...bodyIds: string[]): Promise<void> {
    selectionStore.getState().set(bodyIds.map((id) => ({ kind: "body", id })));
    toolStore.getState().setTool("transform");
    await flush();
  }

  const debug = (): Record<string, unknown> =>
    ((window as unknown as { __extrudePreview?: Record<string, unknown> }).__extrudePreview ?? {});

  const pointer = (type: string, x = 5, y = 5): void => {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button: 0, bubbles: true }));
  };

  /** A genuine CLICK (press + release within the drag threshold) on `hit`. */
  const clickFace = (hit: PickHit | null): void => {
    engineMock.hit = hit;
    pointer("pointerdown");
    pointer("pointerup");
    engineMock.hit = null;
  };

  /** The [Align] segment — a TOGGLE: pressed once starts the flow, again exits. */
  const beginAlign = (): void => toolChipStore.getState().onAlign?.();

  const escape = (): void => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  };

  /** Arm on the mover and run both picks of the canonical +X → −Y align. */
  async function alignMoverOntoDest(): Promise<void> {
    await armOn(MOVER);
    beginAlign();
    clickFace(faceHit(MOVER, "f:0")); // +X, centre (40,0,0)
    clickFace(faceHit(DEST, "f:3")); // −Y, centre (110,−20,0)
  }

  it("the [Align] chip starts the flow and asks for the face to move", async () => {
    await armOn(MOVER);
    expect(toolChipStore.getState().alignPhase).toBeNull();
    beginAlign();
    expect(toolChipStore.getState().alignPhase).toBe("pickMoving");
    expect(debug().transformAlignPhase).toBe("pickMoving");
    expect(hintText()).toMatch(/face to move/i);
  });

  it("drops the ghost + gizmo so the bodies being placed stay clickable", async () => {
    await armOn(MOVER);
    // A nudge first: the ghost is up and the sources are HIDDEN…
    toolChipStore.getState().onValue?.(30);
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenLastCalledWith([MOVER]);
    engineMock.hideTransformGizmo.mockClear();

    beginAlign();
    // …and entering the align puts them back, because they are what gets picked.
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenLastCalledWith([]);
    expect(engineMock.hideGhostPreview).toHaveBeenCalled();
    expect(engineMock.hideTransformGizmo).toHaveBeenCalled();
    expect(debug().transformGizmo).toBe(false);
  });

  it("solves the placement from two REAL faces and lands it in the FSM", async () => {
    await alignMoverOntoDest();

    expect(debug().transformAlignPhase).toBeNull();
    // R = +90° about Z; R·(40,0,0) = (0,40,0); (110,−20,0) − (0,40,0) = (110,−60,0).
    const t = debug().transformTranslate as number[];
    expect(t[0]).toBeCloseTo(110, 9);
    expect(t[1]).toBeCloseTo(-60, 9);
    expect(t[2]).toBeCloseTo(0, 9);
    expect(debug().transformAngleDeg as number).toBeCloseTo(90, 9);
    expect((debug().transformRotAxisVec as number[])[2]).toBeCloseTo(1, 9);
    // Reads out as the MOVE it is, on the component it leaned on hardest.
    expect(debug().transformMode).toBe("move");
    expect(debug().transformAxis).toBe("X");
    expect(toolChipStore.getState().value).toBeCloseTo(110, 9);
  });

  it("re-raises the ghost at the solved placement once the flow ends", async () => {
    await armOn(MOVER);
    beginAlign();
    engineMock.showGhostPreviewMulti.mockClear();
    clickFace(faceHit(MOVER, "f:0"));
    expect(engineMock.showGhostPreviewMulti).not.toHaveBeenCalled(); // still picking
    clickFace(faceHit(DEST, "f:3"));
    expect(engineMock.showGhostPreviewMulti).toHaveBeenCalled();
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenLastCalledWith([MOVER]);
    expect(debug().transformGizmo).toBe(true); // the gizmo is back to nudge with
  });

  it("commits the FREE rotation axis verbatim — no snap to a world axis", async () => {
    await alignMoverOntoDest();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();

    const op = (clientMock.applyOperation.mock.calls as unknown as OperationOp[][])[0][0];
    expect(op.opType).toBe("TransformBody");
    const params = op.params as TransformBodyParams;
    expect(params.rotate.angleDeg).toBeCloseTo(90, 9);
    expect(params.rotate.axis[2]).toBeCloseTo(1, 9);
    expect(params.rotate.center).toEqual([0, 0, 0]); // the FROZEN pivot
    expect(params.translate[0]).toBeCloseTo(110, 9);
    expect(params.translate[1]).toBeCloseTo(-60, 9);
  });

  it("Enter is held back while a pick is still outstanding", async () => {
    await armOn(MOVER);
    beginAlign();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(debug().transformAlignPhase).toBe("pickMoving");
  });

  // ── pick validation ──

  it("REFUSES a first pick that is not on a body being moved, and stays in phase", async () => {
    await armOn(MOVER);
    beginAlign();
    clickFace(faceHit(DEST, "f:1"));
    expect(debug().transformAlignPhase).toBe("pickMoving");
    expect(hintText()).toMatch(/not on a body being moved/i);
  });

  it("REFUSES a self-align: the destination may not be a body being moved", async () => {
    await armOn(MOVER);
    beginAlign();
    clickFace(faceHit(MOVER, "f:0"));
    expect(debug().transformAlignPhase).toBe("pickDest");
    clickFace(faceHit(MOVER, "f:1")); // another face of the SAME body
    expect(debug().transformAlignPhase).toBe("pickDest");
    expect(hintText()).toMatch(/cannot be aligned to itself/i);
    expect(debug().transformTranslate).toEqual([0, 0, 0]); // nothing was written
  });

  it("REFUSES a curved face loudly and stays in the phase", async () => {
    await armOn(ROUND);
    beginAlign();
    clickFace(faceHit(ROUND, "f:0")); // the 24-segment cylinder wall
    expect(debug().transformAlignPhase).toBe("pickMoving");
    expect(hintText()).toMatch(/not flat/i);
    // The planar CAP of the same body is accepted — the refusal is about
    // curvature, not about the body.
    clickFace(faceHit(ROUND, "f:1"));
    expect(debug().transformAlignPhase).toBe("pickDest");
  });

  it("REFUSES a miss (empty space / an edge) without leaving the phase", async () => {
    await armOn(MOVER);
    beginAlign();
    clickFace(null);
    expect(debug().transformAlignPhase).toBe("pickMoving");
    expect(hintText()).toMatch(/flat FACE/i);
    clickFace({ ...faceHit(MOVER, "e:0"), kind: "edge" } as PickHit);
    expect(debug().transformAlignPhase).toBe("pickMoving");
  });

  it("a DRAG is an orbit, not a pick", async () => {
    await armOn(MOVER);
    beginAlign();
    engineMock.hit = faceHit(MOVER, "f:0");
    pointer("pointerdown", 5, 5);
    pointer("pointermove", 90, 90);
    pointer("pointerup", 90, 90);
    engineMock.hit = null;
    expect(debug().transformAlignPhase).toBe("pickMoving");
  });

  // ── hover ──

  it("hover-tints the face under the pointer through selectionStore (one writer)", async () => {
    await armOn(MOVER);
    beginAlign();
    engineMock.hit = faceHit(MOVER, "f:4");
    pointer("pointermove", 20, 20);
    const hover = selectionStore.getState().hover;
    expect(hover?.kind).toBe("face");
    expect(hover?.bodyId).toBe(MOVER);
    expect(hover?.topoKey).toBe("f:4");
    expect(hover?.id).toBe(`${MOVER}#f:4`);
  });

  it("does not tint a face the OUTSTANDING pick would refuse", async () => {
    await armOn(MOVER);
    beginAlign();
    engineMock.hit = faceHit(DEST, "f:1"); // wrong side for pick 1
    pointer("pointermove", 20, 20);
    expect(selectionStore.getState().hover).toBeNull();
    // …and after the first pick the allowed side flips.
    clickFace(faceHit(MOVER, "f:0"));
    engineMock.hit = faceHit(DEST, "f:1");
    pointer("pointermove", 21, 21);
    expect(selectionStore.getState().hover?.bodyId).toBe(DEST);
  });

  it("clears its OWN hover when the flow ends, and leaves a foreign one alone", async () => {
    await armOn(MOVER);
    beginAlign();
    engineMock.hit = faceHit(MOVER, "f:4");
    pointer("pointermove", 20, 20);
    expect(selectionStore.getState().hover).not.toBeNull();
    escape();
    expect(selectionStore.getState().hover).toBeNull();

    // Nothing of ours is set now, so a hover somebody else owns survives a
    // second Esc through the same teardown.
    const foreign = { kind: "face" as const, id: "other#f:1", bodyId: "other", topoKey: "f:1" };
    selectionStore.getState().setHover(foreign);
    beginAlign();
    escape();
    expect(selectionStore.getState().hover).toEqual(foreign);
  });

  // ── Esc ladder ──

  it("Esc walks back one pick at a time and leaves the placement ARMED", async () => {
    await armOn(MOVER);
    toolChipStore.getState().onValue?.(30);
    beginAlign();
    clickFace(faceHit(MOVER, "f:0"));
    expect(debug().transformAlignPhase).toBe("pickDest");

    escape();
    expect(debug().transformAlignPhase).toBe("pickMoving");
    escape();
    expect(debug().transformAlignPhase).toBeNull();
    expect(debug().transformPhase).toBe("armed");
    // The pre-align placement is untouched — backing out is not cancelling.
    expect(debug().transformTranslate).toEqual([30, 0, 0]);
    expect(debug().transformGizmo).toBe(true);
  });

  it("a re-entered flow does not reuse the abandoned first pick", async () => {
    await armOn(MOVER);
    beginAlign();
    clickFace(faceHit(MOVER, "f:0")); // +X
    escape(); // → pickMoving; the +X frame is dropped
    clickFace(faceHit(MOVER, "f:4")); // +Z, centre (0,0,15)
    clickFace(faceHit(DEST, "f:3")); // −Y, centre (110,−20,0)

    // +Z onto −Y turns about X, not about Z — so the ROTATION AXIS alone proves
    // which first pick was used, independently of any translate arithmetic.
    const t = debug().transformTranslate as number[];
    expect(t[0]).toBeCloseTo(110, 9);
    expect(Math.abs(debug().transformAngleDeg as number)).toBeCloseTo(90, 9);
    expect((debug().transformRotAxisVec as number[])[0]).toBeCloseTo(-1, 9); // about X, not Z
  });

  // ── multi-target ──

  it("moves the WHOLE selection by the one solved motion", async () => {
    await armOn(MOVER, ROUND);
    beginAlign();
    // The moving face may belong to ANY target — here the second one's cap.
    clickFace(faceHit(ROUND, "f:1"));
    clickFace(faceHit(DEST, "f:3"));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();

    const params = (clientMock.applyOperation.mock.calls as unknown as OperationOp[][])[0][0]
      .params as TransformBodyParams;
    expect(params.targets).toEqual([MOVER, ROUND]);
    // One record, one motion — the pivot is the COMBINED bbox centre, frozen at
    // arm, not the picked body's own.
    expect(params.rotate.center[0]).toBeCloseTo(0, 9);
  });

  it("the [Align] segment is a TOGGLE: a second press leaves the flow", async () => {
    await armOn(MOVER);
    beginAlign();
    clickFace(faceHit(MOVER, "f:0"));
    expect(debug().transformAlignPhase).toBe("pickDest");
    beginAlign(); // the control that SAYS it is running must be able to stop it
    expect(debug().transformAlignPhase).toBeNull();
    expect(toolChipStore.getState().alignPhase).toBeNull();
    expect(debug().transformPhase).toBe("armed");
  });

  // ── the record's own frame (fold) ──

  it("solves against the record's INPUT geometry on a fold, not the moved mesh", async () => {
    // The mesh in the registry is the body AS COMMITTED: already +30 along X.
    swapMesh(MOVER, buildBodyObjects(parseMeshPayload(makeBoxMesh(80, 60, 30, 0, [30, 0, 0])), MOVER, 2));
    clientMock.canFoldTransform.mockResolvedValue("f7");
    clientMock.getOperationParams.mockResolvedValue({
      targets: [MOVER],
      translate: [30, 0, 0],
      rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
      copy: false,
    });
    await armOn(MOVER);
    expect(debug().transformFold).toBe("f7");
    expect(debug().transformTranslate).toEqual([30, 0, 0]);

    beginAlign();
    clickFace(faceHit(MOVER, "f:0")); // on screen at x = 70; in the RECORD's frame, 40
    clickFace(faceHit(DEST, "f:1")); // −X face of the far box, centre (90,0,0)

    // Already anti-parallel ⇒ pure translation. The record's own frame puts the
    // face at 40, so the answer is 50 — reading the moved mesh would say 20.
    expect(debug().transformAngleDeg).toBe(0);
    expect((debug().transformTranslate as number[])[0]).toBeCloseTo(50, 9);
  });

  /**
   * The same frame distinction, on the GHOST. The registry mesh is the body as
   * COMMITTED, so a ghost drawn at the raw placement would apply the record's
   * stored motion twice: a re-edit of a 30mm move would preview at 60 and then
   * commit at 30. The ghost matrix therefore carries `M_live ∘ M_armed⁻¹`.
   */
  it("ghosts a folded re-edit at the EDITED placement, not on top of the stored one", async () => {
    swapMesh(MOVER, buildBodyObjects(parseMeshPayload(makeBoxMesh(80, 60, 30, 0, [30, 0, 0])), MOVER, 2));
    clientMock.canFoldTransform.mockResolvedValue("f7");
    clientMock.getOperationParams.mockResolvedValue({
      targets: [MOVER],
      translate: [30, 0, 0],
      rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
      copy: false,
    });
    await armOn(MOVER);
    toolChipStore.getState().onValue?.(50); // re-edit 30 → 50

    const calls = engineMock.showGhostPreviewMulti.mock.calls;
    const items = calls[calls.length - 1][0] as { transforms: { kind: string; m: number[] }[] }[];
    const m = items[0].transforms[0].m;
    // The on-screen +X face sits at x = 70; the edited record puts it at 90.
    const x = m[0] * 70 + m[1] * 0 + m[2] * 0 + m[3];
    expect(x).toBeCloseTo(90, 9); // NOT 120, which the un-corrected matrix gives
  });

  it("ghosts a FRESH arm at the plain placement (the correction is the identity)", async () => {
    await armOn(MOVER);
    toolChipStore.getState().onValue?.(30);
    const calls = engineMock.showGhostPreviewMulti.mock.calls;
    const items = calls[calls.length - 1][0] as { transforms: { kind: string; m: number[] }[] }[];
    const m = items[0].transforms[0].m;
    expect(m[0] * 40 + m[3]).toBeCloseTo(70, 9);
  });
});
