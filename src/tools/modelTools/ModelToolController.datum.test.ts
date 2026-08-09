/*
 * ModelToolController — datum plane tool (DATUM W1), jsdom.
 *
 * Covers the whole two-phase gesture (arm → base pick → offset chip + ghost →
 * commit), Esc at EVERY phase, the chip ✕, and teardown idempotence across the
 * tool-switch sweep / mode change / dispose. The engine + client are fakes; the
 * debug surface (`deps.debug`) exposes the datum phase for assertions.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import { toolStore } from "@/stores/toolStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

// The datum layer is a viewport CONTRIBUTION now, so its imperative surface is
// published through `modules/modeling/datumViewport` rather than hung off the
// engine. Same fakes, one seam over.
import { getDatumVisuals } from "@/modules/modeling/datumViewport";

vi.mock("@/modules/modeling/datumViewport", () => ({ getDatumVisuals: vi.fn() }));

const datumVisualsMock = {
  sync: vi.fn(),
  hitTest: vi.fn((): string | null => null),
  setHover: vi.fn(),
  setSelected: vi.fn(),
  setGhost: vi.fn(),
  ghostVisible: false,
};

const okEdit: ApplyOperationResult = {
  revision: 7,
  features: [],
  changedBodies: [],
  removedBodies: [],
};

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function debug(): Record<string, unknown> {
  return (window as unknown as { __extrudePreview?: Record<string, unknown> }).__extrudePreview ?? {};
}

describe("ModelToolController — datum plane tool", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let planeHit: PickablePlane | null;
  let engineMock: {
    setPlanePickerVisible: ReturnType<typeof vi.fn>;
    planePickerHover: ReturnType<typeof vi.fn>;
    planePickerHitTest: ReturnType<typeof vi.fn>;
    [k: string]: unknown;
  };
  let clientMock: { applyEditCommand: ReturnType<typeof vi.fn>; [k: string]: unknown };

  function build(applyEditCommand = vi.fn(() => Promise.resolve(okEdit))): void {
    planeHit = null;
    engineMock = {
      setPlanePickerVisible: vi.fn(),
      planePickerHover: vi.fn(),
      planePickerHitTest: vi.fn(() => planeHit),
      // Everything the shared cancel sweep touches on any tool switch.
      setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
      hideRegionPick: vi.fn(),
      setRegionHover: vi.fn(),
      setRegionSelected: vi.fn(),
      hideExtrudePreview: vi.fn(),
      hideRevolvePreview: vi.fn(),
      hideGhostPreview: vi.fn(),
      hideValueHandle: vi.fn(),
      showValueHandle: vi.fn(),
      showGhostPreviewMulti: vi.fn(),
      setPreviewTint: vi.fn(),
      clearPreviewBody: vi.fn(),
      setExtrudeHandleHover: vi.fn(),
      hitExtrudeHandle: vi.fn(() => false),
      isExtrudePreviewVisible: vi.fn(() => false),
    };
    clientMock = { onPreviewResult: vi.fn(() => () => {}), applyEditCommand };
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  function key(k: string): void {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: k }));
  }

  /** Arm the tool and pick the XY base quad (the common prefix of most specs). */
  function armAndPickBase(kind: PickablePlane = "XY"): void {
    toolStore.getState().setTool("datum");
    planeHit = kind;
    click(10, 10);
  }

  beforeEach(() => {
    resetStores();
    datumVisualsMock.setGhost.mockClear();
    vi.mocked(getDatumVisuals).mockReturnValue(datumVisualsMock);
    documentStore.getState().applyChange({ datums: {} });
    container = document.createElement("div");
    document.body.appendChild(container);
    build();
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    documentStore.getState().applyChange({ datums: {} });
  });

  // ── arm → base pick → offset → commit ───────────────────────────────────────

  it("arming shows the plane picker and enters the base-pick phase (no chip yet)", () => {
    toolStore.getState().setTool("datum");
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(true);
    expect(debug().datumPhase).toBe("basePick");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(viewportStore.getState().statusHint?.message).toMatch(/Select a base plane/);
  });

  it("a pointer move during base pick drives the picker hover (and nothing else)", () => {
    toolStore.getState().setTool("datum");
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: 5, clientY: 6, bubbles: true }));
    expect(engineMock.planePickerHover).toHaveBeenCalledWith(5, 6);
  });

  it("a click OFF every quad stays in base pick (no accidental datum)", () => {
    toolStore.getState().setTool("datum");
    planeHit = null;
    click(10, 10);
    expect(debug().datumPhase).toBe("basePick");
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("picking a base hides the picker, shows the ghost and opens the offset chip", () => {
    armAndPickBase("XY");

    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
    expect(debug().datumPhase).toBe("offset");
    expect(debug().datumBase).toBe("XY");
    expect(debug().datumOffset).toBe(10);
    // Ghost at the seed offset, labelled with the GEOMETRIC plane name.
    expect(datumVisualsMock.setGhost).toHaveBeenLastCalledWith("XY", 10, "XY");

    const chip = toolChipStore.getState();
    expect(chip.kind).toBe("datumOffset");
    expect(chip.value).toBe(10);
    expect(chip.label).toBe("XY");
    // Anchored at the ghost origin (XY base, +10 along world +Z).
    expect(chip.worldPos).toEqual([0, 0, 10]);
  });

  it("the chip label uses the GEOMETRIC plane name, not the legacy-swapped repo kind", () => {
    // Repo kind "XZ" has world normal +X, i.e. it IS the YZ plane geometrically.
    armAndPickBase("XZ");
    expect(debug().datumBase).toBe("XZ"); // stored kind: the repo one
    expect(toolChipStore.getState().label).toBe("YZ"); // shown: the geometric one
    expect(datumVisualsMock.setGhost).toHaveBeenLastCalledWith("XZ", 10, "YZ");
  });

  it("editing the chip offset moves the ghost live", () => {
    armAndPickBase("XY");
    toolChipStore.getState().onValue?.(35);
    expect(debug().datumOffset).toBe(35);
    expect(toolChipStore.getState().value).toBe(35);
    expect(datumVisualsMock.setGhost).toHaveBeenLastCalledWith("XY", 35, "XY");
  });

  it("the chip ✓ commits AddDatumPlane with the picked base + offset, then resets to select", async () => {
    armAndPickBase("XY");
    toolChipStore.getState().onValue?.(25);
    toolChipStore.getState().onConfirm?.();
    await flush();

    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    const cmd = clientMock.applyEditCommand.mock.calls[0][0];
    expect(cmd).toEqual({
      cmd: "addDatumPlane",
      datum: expect.objectContaining({
        kind: "OffsetFromPlane",
        basePlaneId: "XY",
        offset: 25,
        name: "Datum 1",
        resolvedValid: false,
      }),
    });
    // A real UUID id, not a display name.
    expect(cmd.datum.id).toMatch(/^[0-9a-f-]{36}$/);

    expect(toolStore.getState().modelTool).toBe("select");
    expect(debug().datumPhase).toBe("idle");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(datumVisualsMock.setGhost).toHaveBeenLastCalledWith(null, 0);
    expect(viewportStore.getState().statusHint?.message).toBe("Datum 1 created");
  });

  it("Enter commits the armed datum too (same gesture as the chip ✓)", async () => {
    armAndPickBase("YZ");
    key("Enter");
    await flush();
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    expect(clientMock.applyEditCommand.mock.calls[0][0].datum.basePlaneId).toBe("YZ");
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("Enter during the BASE-PICK phase does not commit anything", async () => {
    toolStore.getState().setTool("datum");
    key("Enter");
    await flush();
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
    expect(debug().datumPhase).toBe("basePick");
  });

  it("the name is the next free 'Datum N' against the existing projection", async () => {
    documentStore.getState().addDatum({
      id: "d1",
      name: "Datum 3",
      basePlaneId: "XY",
      offset: 1,
      plane: { kind: "custom", origin: [0, 0, 1], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
      resolvedValid: true,
    });
    armAndPickBase("XY");
    toolChipStore.getState().onConfirm?.();
    await flush();
    expect(clientMock.applyEditCommand.mock.calls[0][0].datum.name).toBe("Datum 4");
  });

  it("a rejected commit surfaces the backend message and still leaves the tool clean", async () => {
    build(vi.fn(() => Promise.reject(new Error("datum id collides"))));
    armAndPickBase("XY");
    toolChipStore.getState().onConfirm?.();
    await flush();

    expect(viewportStore.getState().statusHint?.message).toBe(
      "Create datum plane failed: datum id collides",
    );
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    expect(debug().datumPhase).toBe("idle");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("the controller NEVER writes the datum into the store itself (the projection does)", async () => {
    armAndPickBase("XY");
    toolChipStore.getState().onConfirm?.();
    await flush();
    // The fake client publishes no projection, so the store must stay empty —
    // an optimistic local write here would diverge from the backend's frame.
    expect(documentStore.getState().datums).toEqual({});
  });

  // ── cancel / teardown at every phase ────────────────────────────────────────

  it("Esc during BASE PICK hides the picker and returns to select", () => {
    toolStore.getState().setTool("datum");
    key("Escape");
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
    expect(debug().datumPhase).toBe("idle");
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("Esc during the OFFSET phase drops the ghost + chip and returns to select", () => {
    armAndPickBase("XY");
    key("Escape");
    expect(datumVisualsMock.setGhost).toHaveBeenLastCalledWith(null, 0);
    expect(toolChipStore.getState().kind).toBe("none");
    expect(debug().datumPhase).toBe("idle");
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("the chip ✕ cancels exactly like Esc", () => {
    armAndPickBase("XY");
    toolChipStore.getState().onCancel?.();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(debug().datumPhase).toBe("idle");
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("switching to another model tool sweeps the datum arm away", () => {
    armAndPickBase("XY");
    toolStore.getState().setTool("select");
    expect(debug().datumPhase).toBe("idle");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
  });

  it("leaving model mode sweeps the datum arm away (cancelAll)", () => {
    armAndPickBase("XY");
    toolStore.getState().setMode("sketch");
    expect(debug().datumPhase).toBe("idle");
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("dispose tears the arm down (the chip store outlives the controller)", () => {
    armAndPickBase("XY");
    expect(toolChipStore.getState().kind).toBe("datumOffset");
    controller.dispose();
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("endDatumPick is idempotent — a second cancel is a silent no-op", () => {
    armAndPickBase("XY");
    key("Escape");
    const hides = engineMock.setPlanePickerVisible.mock.calls.length;
    key("Escape");
    key("Escape");
    // No further picker/ghost churn once the tool is already idle.
    expect(engineMock.setPlanePickerVisible.mock.calls.length).toBe(hides);
    expect(debug().datumPhase).toBe("idle");
  });

  it("a commit that is superseded mid-flight (tool switched) publishes no hint of its own", async () => {
    let resolveApply: (r: ApplyOperationResult) => void = () => {};
    build(vi.fn(() => new Promise<ApplyOperationResult>((r) => (resolveApply = r))));
    armAndPickBase("XY");
    toolChipStore.getState().onConfirm?.();

    // The user moves on while the edit is in flight (bumps commitGen).
    toolStore.getState().setTool("extrude");
    const hintBefore = viewportStore.getState().statusHint?.message ?? null;

    resolveApply(okEdit);
    await flush();

    expect(viewportStore.getState().statusHint?.message ?? null).toBe(hintBefore);
    expect(toolStore.getState().modelTool).toBe("extrude"); // NOT reset to select
  });
});
