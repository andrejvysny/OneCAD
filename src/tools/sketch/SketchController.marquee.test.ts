/*
 * SketchController marquee / box select (jsdom, W2-D).
 *
 * Covers the controller lane only — the geometry itself is marqueeSelect.test.ts.
 * Engine + client are faked; `screenToPlane` is 1:1 so client coords ARE plane
 * coords, which makes the screen-corner → plane-AABB mapping the identity.
 *
 * The load-bearing assertions are the TEARDOWN ones: `setSketchDrawingActive` is a
 * SHARED LMB-orbit suppression flag (armed draw tools + modal model drags use it),
 * so a marquee that exits without restoring it silently kills orbit for everything.
 * Every exit path is asserted: release, sub-threshold release, Esc, tool switch,
 * a release that lands OUTSIDE the viewport, and dispose.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// Two well-separated horizontal lines (plane == client coords under the 1:1 mock).
const ENTITIES: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
  { id: "e2", type: "Line", p0: [0, 60], p1: [40, 60] },
];

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    clearPlanePickerHover: vi.fn(),
    // DATUM W1: the plane-pick path consults the datum layer first.
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
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
  };
}

function makeClientMock() {
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: ENTITIES.map((e) => ({ ...e })),
          constraints: [],
          dof: 4,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    finishSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    endGesture: vi.fn(() => Promise.resolve(undefined)),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController marquee select", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
    engineMock.setSketchDrawingActive.mockClear();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(
    type: string,
    x: number,
    y: number,
    button: number,
    buttons: number,
    mods?: { shiftKey?: boolean; metaKey?: boolean },
  ): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true, ...mods }),
    );
  }

  const selected = () => sketchSelectionStore.getState().selected;
  const overlay = () => container.querySelector<HTMLElement>("[data-sketch-marquee]");
  const orbitCalls = () => engineMock.setSketchDrawingActive.mock.calls.map((c) => c[0]);

  it("an empty-space LMB down arms the marquee and suppresses LMB orbit", () => {
    mouse("pointerdown", 200, 200, 0, 1);
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(true);
    expect(overlay()).toBeNull(); // not drawn until it passes DRAG_PX
  });

  it("a down ON a draggable handle arms a drag, not a marquee", () => {
    mouse("pointerdown", 0, 0, 0, 1); // e1.Start
    mouse("pointermove", 20, 20, 0, 1);
    expect(overlay()).toBeNull();
  });

  it("past DRAG_PX the overlay appears; rightward is solid (window), leftward dashed (crossing)", () => {
    mouse("pointerdown", 100, 100, 0, 1);
    mouse("pointermove", 160, 140, 0, 1); // rightward
    const el = overlay();
    expect(el).not.toBeNull();
    expect(el!.style.borderStyle).toBe("solid");
    expect(el!.dataset.marqueeMode).toBe("window");
    expect(el!.style.width).toBe("60px");
    expect(el!.style.height).toBe("40px");

    mouse("pointermove", 40, 140, 0, 1); // now leftward — mode re-evaluated live
    expect(overlay()!.style.borderStyle).toBe("dashed");
    expect(overlay()!.dataset.marqueeMode).toBe("crossing");
  });

  it("a sub-threshold release collapses to the existing click-clear path", () => {
    sketchSelectionStore.getState().set([{ entityId: "e1" }]);
    mouse("pointerdown", 200, 200, 0, 1);
    mouse("pointerup", 200, 201, 0, 0);
    expect(selected()).toEqual([]); // click-on-empty clears, as before
    expect(overlay()).toBeNull();
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false); // orbit back
  });

  it("a rightward WINDOW drag selects only fully-enclosed entities", () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    mouse("pointerup", 50, 70, 0, 0);
    expect(selected()).toEqual([{ entityId: "e1" }, { entityId: "e2" }]);
    expect(overlay()).toBeNull();
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false);
  });

  it("a rightward WINDOW drag that only clips an entity selects nothing", () => {
    mouse("pointerdown", 20, -10, 0, 1);
    mouse("pointermove", 80, 70, 0, 1);
    mouse("pointerup", 80, 70, 0, 0); // e1/e2 start at x=0, outside the box
    expect(selected()).toEqual([]);
  });

  it("a leftward CROSSING drag selects only the entity it touches", () => {
    mouse("pointerdown", 50, 50, 0, 1);
    mouse("pointermove", 20, 70, 0, 1);
    mouse("pointerup", 20, 70, 0, 0); // box x∈[20,50] y∈[50,70] grazes e2 only
    expect(selected()).toEqual([{ entityId: "e2" }]);
  });

  it("a plain drag REPLACES the selection; Shift UNIONs (never toggles)", () => {
    sketchSelectionStore.getState().set([{ entityId: "e1", point: "Start" }]);
    // Plain: replaces.
    mouse("pointerdown", -10, 50, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    mouse("pointerup", 50, 70, 0, 0);
    expect(selected()).toEqual([{ entityId: "e2" }]);

    // Shift: unions e1 in, keeping e2.
    mouse("pointerdown", -10, -10, 0, 1, { shiftKey: true });
    mouse("pointermove", 50, 10, 0, 1, { shiftKey: true });
    mouse("pointerup", 50, 10, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([{ entityId: "e2" }, { entityId: "e1" }]);

    // Shift again over the same box does NOT toggle e1 back off.
    mouse("pointerdown", -10, -10, 0, 1, { shiftKey: true });
    mouse("pointermove", 50, 10, 0, 1, { shiftKey: true });
    mouse("pointerup", 50, 10, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([{ entityId: "e2" }, { entityId: "e1" }]);
  });

  it("Esc mid-marquee cancels: overlay removed, orbit restored, selection untouched", () => {
    sketchSelectionStore.getState().set([{ entityId: "e1" }]);
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    expect(overlay()).not.toBeNull();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(overlay()).toBeNull();
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false);
    expect(selected()).toEqual([{ entityId: "e1" }]);

    // The release that follows the cancel must not resurrect the box.
    mouse("pointerup", 50, 70, 0, 0);
    expect(selected()).toEqual([{ entityId: "e1" }]);
  });

  it("a tool switch mid-marquee tears the box down and gives LMB orbit back", () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    expect(overlay()).not.toBeNull();

    toolStore.getState().setTool("trim"); // a click tool: no machine ⇒ orbit restored
    expect(overlay()).toBeNull();
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false);
  });

  it("a release OUTSIDE the viewport still resolves the box (window pointerup net)", () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    // Released over a floating panel: the event never reaches the container.
    window.dispatchEvent(
      new MouseEvent("pointerup", { clientX: 50, clientY: 70, button: 0, buttons: 0 }),
    );
    expect(selected()).toEqual([{ entityId: "e1" }, { entityId: "e2" }]);
    expect(overlay()).toBeNull();
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false);
  });

  it("pointercancel drops the box without selecting", () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    window.dispatchEvent(new MouseEvent("pointercancel"));
    expect(overlay()).toBeNull();
    expect(selected()).toEqual([]);
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false);
  });

  it("dispose mid-marquee removes the overlay and restores orbit", () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    expect(overlay()).not.toBeNull();
    controller.dispose();
    expect(overlay()).toBeNull();
    expect(orbitCalls()).toContain(false);
  });

  it("leaving sketch mode mid-marquee tears the box down", async () => {
    mouse("pointerdown", -10, -10, 0, 1);
    mouse("pointermove", 50, 70, 0, 1);
    expect(overlay()).not.toBeNull();
    toolStore.getState().setMode("model");
    await flush();
    expect(overlay()).toBeNull();
    expect(orbitCalls()).toContain(false);
  });
});
