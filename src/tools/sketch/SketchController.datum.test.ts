/*
 * SketchController — sketch-on-datum entry (DATUM W1), jsdom.
 *
 * Two routes into a datum-hosted sketch, both exercised here:
 *   1. a SELECTED datum + `setMode("sketch")` (the tree row double-click) →
 *      `tryEnterOnSelectedDatum`, checked before the world-plane picker;
 *   2. a datum QUAD clicked while the plane picker is up → `confirmDatumPick`,
 *      which wins over the world quads under the same pointer.
 *
 * The load-bearing assertion in both: `enterSketch` is called with the datum's
 * BACKEND-RESOLVED plane read off the projection store, never a re-derived one.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { CadClient } from "@/ipc/client";
import type { EnterSketchTarget, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore, type DatumMeta } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

/** The datum's resolved frame — deliberately NOT the canonical XY basis, so a
 *  re-derivation anywhere on the frontend would show up as a different plane. */
const DATUM_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 10],
  xAxis: [0, 1, 0],
  yAxis: [-1, 0, 0],
  normal: [0, 0, 1],
};

function datum(overrides: Partial<DatumMeta> = {}): DatumMeta {
  return {
    id: "d1",
    name: "Datum 1",
    basePlaneId: "XY",
    offset: 10,
    plane: DATUM_PLANE,
    resolvedValid: true,
    ...overrides,
  };
}

function makeSession(target: EnterSketchTarget): SketchSession {
  const sketchId = typeof target === "string" ? target : "sketchNew";
  const plane =
    typeof target !== "string" && "plane" in target ? target.plane : DATUM_PLANE;
  return { sketchId, plane, entities: [], constraints: [], dof: 0, status: "UnderConstrained" };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — sketch on a datum plane", () => {
  let container: HTMLDivElement;
  let controller: SketchController;
  let planeHit: PickablePlane | null;
  let datumHit: string | null;
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;

  function makeEngineMock() {
    return {
      setPlanePickerVisible: vi.fn(),
      planePickerHover: vi.fn(),
      planePickerHitTest: vi.fn(() => planeHit),
      clearPlanePickerHover: vi.fn(),
      datumHitTest: vi.fn(() => datumHit),
      setDatumHover: vi.fn(),
      enterSketch: vi.fn(),
      exitSketch: vi.fn(),
      setSketchDrawingActive: vi.fn(),
      setSketchPreview: vi.fn(),
      setSketchGhost: vi.fn(),
      setSketchTrimGhost: vi.fn(),
      setSketchSnap: vi.fn(),
    };
  }

  function makeClientMock() {
    return {
      enterSketch: vi.fn((target: EnterSketchTarget) => Promise.resolve(makeSession(target))),
      cancelSketch: vi.fn(() => Promise.resolve()),
      deleteSketch: vi.fn(() => Promise.resolve()),
      faceSketchPlane: vi.fn(),
      promoteSelection: vi.fn(),
    };
  }

  beforeEach(() => {
    resetStores();
    planeHit = null;
    datumHit = null;
    documentStore.getState().addDatum(datum());
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
    documentStore.getState().applyChange({ datums: {} });
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  // ── route 1: a selected datum ───────────────────────────────────────────────

  it("a SELECTED datum enters directly on it — no plane picker, backend plane verbatim", async () => {
    selectionStore.getState().set([{ kind: "datum", id: "d1" }]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(engineMock.setPlanePickerVisible).not.toHaveBeenCalled();
    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.enterSketch).toHaveBeenCalledWith({
      newOnDatum: { datumId: "d1" },
      plane: DATUM_PLANE,
    });
    // Same object identity: the frame is carried through, never rebuilt.
    const target = clientMock.enterSketch.mock.calls[0][0];
    if (typeof target === "string" || !("plane" in target)) throw new Error("expected a hosted target");
    expect(target.plane).toBe(DATUM_PLANE);
    expect(viewportStore.getState().activeSketchId).toBe("sketchNew");
  });

  it("an UNRESOLVED datum is refused with a message and falls back to the plane picker", async () => {
    documentStore.getState().addDatum(datum({ resolvedValid: false }));
    selectionStore.getState().set([{ kind: "datum", id: "d1" }]);
    toolStore.getState().setMode("sketch");
    await flush();

    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    expect(viewportStore.getState().statusHint?.message).toContain("did not resolve");
  });

  it("a datum ref for an id that is not in the projection falls through to the picker", async () => {
    selectionStore.getState().set([{ kind: "datum", id: "ghost" }]);
    toolStore.getState().setMode("sketch");
    await flush();
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
  });

  it("TWO selected datums are ambiguous — the picker wins rather than a guess", async () => {
    documentStore.getState().addDatum(datum({ id: "d2", name: "Datum 2" }));
    selectionStore.getState().set([
      { kind: "datum", id: "d1" },
      { kind: "datum", id: "d2" },
    ]);
    toolStore.getState().setMode("sketch");
    await flush();
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
  });

  it("an explicit sketch id still wins over a selected datum (re-open, not a new sketch)", async () => {
    selectionStore.getState().set([{ kind: "datum", id: "d1" }]);
    toolStore.getState().setMode("sketch", "sketch2");
    await flush();
    expect(clientMock.enterSketch).toHaveBeenCalledWith("sketch2");
  });

  // ── route 2: a datum quad clicked during the plane pick ─────────────────────

  it("a datum quad WINS over the world quads under the same pointer", async () => {
    toolStore.getState().setMode("sketch");
    await flush();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);

    // The ray hits both the datum and a world quad — the datum is the deliberate one.
    datumHit = "d1";
    planeHit = "XY";
    click(50, 50);
    await flush();

    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.enterSketch).toHaveBeenCalledWith({
      newOnDatum: { datumId: "d1" },
      plane: DATUM_PLANE,
    });
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
  });

  it("with no datum under the pointer the world quad still resolves normally", async () => {
    toolStore.getState().setMode("sketch");
    await flush();
    datumHit = null;
    planeHit = "XZ";
    click(50, 50);
    await flush();
    expect(clientMock.enterSketch).toHaveBeenCalledWith({ newOnPlane: "XZ" });
  });

  it("hovering a datum during the pick tints it and drops the picker's own hover", async () => {
    toolStore.getState().setMode("sketch");
    await flush();

    datumHit = "d1";
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: 5, clientY: 5, bubbles: true }));
    expect(engineMock.setDatumHover).toHaveBeenLastCalledWith("d1");
    expect(engineMock.clearPlanePickerHover).toHaveBeenCalled();

    datumHit = null;
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: 6, clientY: 6, bubbles: true }));
    expect(engineMock.setDatumHover).toHaveBeenLastCalledWith(null);
    expect(engineMock.planePickerHover).toHaveBeenLastCalledWith(6, 6);
  });

  it("clicking an UNRESOLVED datum quad stays in the pick phase with an error hint", async () => {
    documentStore.getState().addDatum(datum({ resolvedValid: false }));
    toolStore.getState().setMode("sketch");
    await flush();

    datumHit = "d1";
    click(50, 50);
    await flush();

    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    // Still picking: the world quads were never hidden.
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(true);
  });

  it("leaving the pick phase clears the datum hover tint", async () => {
    toolStore.getState().setMode("sketch");
    await flush();
    engineMock.setDatumHover.mockClear();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();
    expect(engineMock.setDatumHover).toHaveBeenCalledWith(null);
  });
});
