/*
 * SketchController — sketch-on-face entry (the `face_sketch_plane` topoKey
 * ladder rung's frontend half), jsdom.
 *
 * A SELECTED model face is a sketch host too (MODEL-OPS W2): `tryEnterOnSelectedFace`
 * is checked BEFORE the world-plane picker. The load-bearing assertion: the
 * face's `topoKey` rides along to `faceSketchPlane` — a just-picked face's
 * ElementId is minted but not yet CONSUMED by any op, so it is genuinely absent
 * from the worker's on-demand element-map partition, and only the topoKey rung
 * resolves it (see `api::face_sketch_plane`, mirrors `element_info`'s ladder).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { CadClient } from "@/ipc/client";
import type { EnterSketchTarget, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

/** The backend-resolved frame for the picked face — deliberately NOT the
 *  canonical XY basis, so a re-derivation anywhere on the frontend would show
 *  up as a different plane. */
const FACE_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 25],
  xAxis: [0, 1, 0],
  yAxis: [-1, 0, 0],
  normal: [0, 0, 1],
};

function faceRef(overrides: Partial<EntityRef> = {}): EntityRef {
  return {
    kind: "face",
    id: "body1#f:22",
    bodyId: "body1",
    topoKey: "f:22",
    elementId: "el_1",
    anchor: { worldPoint: [1, 2, 25] },
    ...overrides,
  } as EntityRef;
}

function makeSession(target: EnterSketchTarget): SketchSession {
  const sketchId = typeof target === "string" ? target : "sketchNew";
  const plane = typeof target !== "string" && "plane" in target ? target.plane : FACE_PLANE;
  return { sketchId, plane, entities: [], constraints: [], dof: 3, status: "UnderConstrained" };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — sketch on a selected face", () => {
  let container: HTMLDivElement;
  let controller: SketchController;
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;

  function makeEngineMock() {
    return {
      setPlanePickerVisible: vi.fn(),
      planePickerHover: vi.fn(),
      planePickerHitTest: vi.fn(() => null as PickablePlane | null),
      clearPlanePickerHover: vi.fn(),
      // W3: the plane-pick phase falls through to a body FACE (probePick).
      probePick: vi.fn(() => null),
      datumHitTest: vi.fn(() => null),
      setDatumHover: vi.fn(),
      enterSketch: vi.fn(),
      exitSketch: vi.fn(),
      setSketchDrawingActive: vi.fn(),
      setSketchPreview: vi.fn(),
      moveChip: vi.fn(),
      setSketchGhost: vi.fn(),
      setSketchTrimGhost: vi.fn(),
    setSketchAngleReference: vi.fn(),
    setSketchAnglePreview: vi.fn(),
      setSketchSnap: vi.fn(),
    };
  }

  function makeClientMock() {
    return {
      enterSketch: vi.fn((target: EnterSketchTarget) => Promise.resolve(makeSession(target))),
      cancelSketch: vi.fn(() => Promise.resolve()),
      deleteSketch: vi.fn(() => Promise.resolve()),
      promoteSelection: vi.fn(),
      faceSketchPlane: vi.fn(() => Promise.resolve(FACE_PLANE)),
    };
  }

  beforeEach(() => {
    resetStores();
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  // (a) exactly one selected face ─────────────────────────────────────────────

  it("a SELECTED face enters directly on it — no plane picker, topoKey forwarded, backend plane verbatim", async () => {
    selectionStore.getState().set([faceRef()]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(engineMock.setPlanePickerVisible).not.toHaveBeenCalled();
    expect(clientMock.promoteSelection).not.toHaveBeenCalled(); // elementId already minted
    expect(clientMock.faceSketchPlane).toHaveBeenCalledWith("body1", "el_1", "f:22");
    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.enterSketch).toHaveBeenCalledWith({
      // W2: the topoKey rides through to `add_sketch_on_face` too — it is the
      // rung that resolves a just-promoted, never-consumed ElementId.
      newOnFace: { bodyId: "body1", elementId: "el_1", topoKey: "f:22", worldPoint: [1, 2, 25] },
      plane: FACE_PLANE,
    });
    // Same object identity: the frame is carried through, never rebuilt.
    const target = clientMock.enterSketch.mock.calls[0][0];
    if (typeof target === "string" || !("plane" in target)) throw new Error("expected a hosted target");
    expect(target.plane).toBe(FACE_PLANE);
    expect(viewportStore.getState().activeSketchId).toBe("sketchNew");
  });

  it("an unminted pick promotes via topoKey first, then forwards the resulting elementId", async () => {
    clientMock.promoteSelection.mockResolvedValueOnce([{ elementId: "el_fresh", topoKey: "f:9", kind: "face", bodyId: "body1" }]);
    selectionStore.getState().set([faceRef({ elementId: undefined })]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [{ topoKey: "f:22", anchor: { worldPoint: [1, 2, 25] } }]);
    expect(clientMock.faceSketchPlane).toHaveBeenCalledWith("body1", "el_fresh", "f:22");
    expect(clientMock.enterSketch).toHaveBeenCalledWith({
      newOnFace: { bodyId: "body1", elementId: "el_fresh", topoKey: "f:22", worldPoint: [1, 2, 25] },
      plane: FACE_PLANE,
    });
  });

  // (b) zero selected faces ────────────────────────────────────────────────────

  it("no selected face falls through to the world-plane picker", async () => {
    selectionStore.getState().set([]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
  });

  // (c) two selected faces ─────────────────────────────────────────────────────

  it("TWO selected faces are ambiguous — the picker wins rather than a guess", async () => {
    selectionStore.getState().set([
      faceRef({ id: "body1#f:22" }),
      faceRef({ id: "body1#f:23", topoKey: "f:23", elementId: "el_2" }),
    ]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
  });

  // (d) the backend rejects (e.g. non-planar face) ────────────────────────────

  it("a REJECTED face falls back to the picker instead of stranding the user", async () => {
    clientMock.faceSketchPlane.mockRejectedValueOnce(new Error("only a planar face can host a sketch"));
    selectionStore.getState().set([faceRef()]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
    // The picker's own prompt is showing (sticky) — the fallback did not strand
    // the user with no chrome and no explanation.
    expect(viewportStore.getState().statusHint?.sticky).toBe(true);
  });

  it("the refusal RIDES INTO the picker prompt (a separate hint would be overwritten)", async () => {
    // W2: `beginPlanePick` writes the status hint itself, so a refusal published
    // before it is silently clobbered — the user is told nothing about WHY the
    // face was rejected. The reason is threaded through instead.
    clientMock.faceSketchPlane.mockRejectedValueOnce(new Error("only a planar face can host a sketch"));
    selectionStore.getState().set([faceRef()]);
    toolStore.getState().setMode("sketch");
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.message).toMatch(/Cannot sketch on that face: only a planar face can host a sketch/);
    expect(hint?.message).toMatch(/Select a plane to start the sketch/);
    expect(hint?.severity).toBe("error");
  });
});
