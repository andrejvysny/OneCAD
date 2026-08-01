/*
 * SketchController — the two W3 sketch-on-face entry triggers (jsdom).
 *
 * (b) The plane-pick phase accepts a body FACE, after the datum and the three
 *     world quads. That ORDER is the load-bearing bit and is pinned with all
 *     three stacked under one pointer: the datum and the quads are chrome the
 *     picker itself raised, so model geometry underneath must never steal them.
 *
 * (c) A double-click on a face in model mode. Re-entry wins over creation — a
 *     face that already hosts a sketch opens THAT sketch (the newest) instead of
 *     stacking a second identical projected boundary on the same plane — and a
 *     face the backend refuses reports without flipping the mode at all.
 *
 * There is deliberately NO hover-time planarity check anywhere here: it would
 * cost a backend round-trip per pointer move. The CLICK validates, which is why
 * the refusal paths below matter.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController, newestSketchOnFace, type FacePickTarget } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { CadClient } from "@/ipc/client";
import type { EnterSketchTarget, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

/** The backend-resolved frame — NOT the canonical XY basis, so any re-derivation
 *  on the frontend would show up as a different plane. */
const FACE_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 25],
  xAxis: [0, 1, 0],
  yAxis: [-1, 0, 0],
  normal: [0, 0, 1],
};

/** What `engine.probePick` hands back for the box's top cap. */
function faceHit(topoKey = "f:22") {
  return {
    bodyId: "body1",
    kind: "face" as const,
    topoKey,
    elementId: undefined as string | undefined,
    distance: 4,
    worldPos: { x: 1, y: 2, z: 25 },
  };
}

function makeSession(target: EnterSketchTarget): SketchSession {
  const sketchId = typeof target === "string" ? target : (target.sketchId ?? "sketchNew");
  const plane = typeof target !== "string" && "plane" in target ? target.plane : FACE_PLANE;
  return { sketchId, plane, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

function sketchOnFace(id: string, elementId: string): SketchMeta {
  return {
    id,
    name: id,
    visible: true,
    dof: 0,
    status: "ok",
    geometryToken: `t:${id}`,
    hostFace: { bodyId: "body1", elementId },
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — sketch-on-face entry triggers (W3)", () => {
  let container: HTMLDivElement;
  let controller: SketchController;
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let datumHit: string | null;
  let quadHit: PickablePlane | null;
  let bodyHit: ReturnType<typeof faceHit> | null;

  function makeEngineMock() {
    return {
      setPlanePickerVisible: vi.fn(),
      planePickerHover: vi.fn(() => quadHit),
      planePickerHitTest: vi.fn(() => quadHit),
      clearPlanePickerHover: vi.fn(),
      datumHitTest: vi.fn(() => datumHit),
      setDatumHover: vi.fn(),
      probePick: vi.fn(() => bodyHit),
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
      finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
      deleteSketch: vi.fn(() => Promise.resolve()),
      promoteSelection: vi.fn((bodyId: string, picks: Array<{ topoKey: string }>) =>
        // Deterministic, exactly like the mock client: the SAME pick promotes to
        // the SAME id, which is what makes re-entry by elementId possible at all.
        Promise.resolve(picks.map((p) => ({ topoKey: p.topoKey, elementId: `el_${bodyId}_${p.topoKey}`, kind: "face", bodyId }))),
      ),
      faceSketchPlane: vi.fn(() => Promise.resolve(FACE_PLANE)),
    };
  }

  function pointer(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  /** A clean tap (no movement, so the controller classifies it as a click). */
  function tap(x = 100, y = 100): void {
    pointer("pointerdown", x, y, 0, 1);
    pointer("pointerup", x, y, 0, 0);
  }

  beforeEach(() => {
    resetStores();
    selectionStore.getState().set([]); // no face/datum preselection: bare entry
    datumHit = null;
    quadHit = null;
    bodyHit = null;
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

  // ── (b) plane-pick phase accepts a face ────────────────────────────────────

  describe("plane-pick phase", () => {
    async function armPicker(): Promise<void> {
      toolStore.getState().setMode("sketch");
      await flush();
      expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
    }

    it("RESOLVE ORDER with all three stacked: datum beats quad beats face", async () => {
      await armPicker();
      datumHit = "d1";
      quadHit = "XZ";
      bodyHit = faceHit();

      tap();
      await flush();
      // The datum won: no world-plane sketch, no face sketch. (The seeded datum
      // does not exist in the store, so confirmDatumPick bails — which is itself
      // the proof that the datum rung claimed the click.)
      expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
      expect(clientMock.enterSketch).not.toHaveBeenCalled();

      // Drop the datum: the QUAD is next, still ahead of the face under it.
      datumHit = null;
      tap();
      await flush();
      expect(clientMock.enterSketch).toHaveBeenCalledWith({ newOnPlane: "XZ" });
      expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
    });

    it("a click on a body face (no datum, no quad) opens a sketch ON it", async () => {
      await armPicker();
      bodyHit = faceHit();

      tap();
      await flush();

      expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
        { topoKey: "f:22", anchor: { worldPoint: [1, 2, 25] } },
      ]);
      expect(clientMock.faceSketchPlane).toHaveBeenCalledWith("body1", "el_body1_f:22", "f:22");
      expect(clientMock.enterSketch).toHaveBeenCalledWith({
        // The topoKey rides through to `add_sketch_on_face` — it is the rung that
        // resolves a just-promoted, never-consumed ElementId.
        newOnFace: {
          bodyId: "body1",
          elementId: "el_body1_f:22",
          topoKey: "f:22",
          worldPoint: [1, 2, 25],
        },
        plane: FACE_PLANE,
      });
      // The picker is down and the session is live on the BACKEND's frame.
      expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
      expect(viewportStore.getState().activeSketchId).toBe("sketchNew");
      // The fresh row records its host, so a later double-click re-enters it.
      expect(documentStore.getState().sketches["sketchNew"]?.hostFace).toEqual({
        bodyId: "body1",
        elementId: "el_body1_f:22",
      });
    });

    it("hovering a face highlights it and NAMES it in the prompt; un-hovering restores the prompt", async () => {
      await armPicker();
      const prompt = viewportStore.getState().statusHint?.message;

      bodyHit = faceHit();
      container.dispatchEvent(new MouseEvent("pointermove", { clientX: 5, clientY: 5, bubbles: true }));
      expect(selectionStore.getState().hover).toMatchObject({ kind: "face", bodyId: "body1", topoKey: "f:22" });
      expect(viewportStore.getState().statusHint?.message).toBe("Face of Body 1 — click to sketch on it");

      bodyHit = null;
      container.dispatchEvent(new MouseEvent("pointermove", { clientX: 6, clientY: 6, bubbles: true }));
      expect(selectionStore.getState().hover).toBeNull();
      expect(viewportStore.getState().statusHint?.message).toBe(prompt);
    });

    it("a hovered face is NOT probed while a datum or a quad owns the pointer", async () => {
      await armPicker();
      bodyHit = faceHit();

      datumHit = "d1";
      container.dispatchEvent(new MouseEvent("pointermove", { clientX: 5, clientY: 5, bubbles: true }));
      datumHit = null;
      quadHit = "XY";
      container.dispatchEvent(new MouseEvent("pointermove", { clientX: 6, clientY: 6, bubbles: true }));

      expect(engineMock.probePick).not.toHaveBeenCalled();
      expect(selectionStore.getState().hover).toBeNull();
    });

    it("a REFUSED face keeps the picker up with the reason in the prompt", async () => {
      await armPicker();
      bodyHit = faceHit();
      clientMock.faceSketchPlane.mockRejectedValueOnce(new Error("only a planar face can host a sketch"));

      tap();
      await flush();

      expect(clientMock.enterSketch).not.toHaveBeenCalled();
      expect(toolStore.getState().mode).toBe("sketch");
      // Never hidden: the picker was not torn down, so the quads are still there.
      expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(true);
      const hint = viewportStore.getState().statusHint;
      expect(hint?.message).toMatch(/Cannot sketch on that face: only a planar face can host a sketch/);
      expect(hint?.message).toMatch(/Select a plane to start the sketch/);
      expect(hint?.severity).toBe("error");
      expect(hint?.sticky).toBe(true);

      // Still usable: a quad click after the refusal still works.
      quadHit = "XY";
      tap();
      await flush();
      expect(clientMock.enterSketch).toHaveBeenCalledWith({ newOnPlane: "XY" });
    });

    it("the `entering` guard makes a double-click create exactly ONE sketch", async () => {
      await armPicker();
      bodyHit = faceHit();
      let resolvePlane!: (p: SketchPlane) => void;
      clientMock.faceSketchPlane.mockImplementationOnce(
        () => new Promise<SketchPlane>((r) => { resolvePlane = r; }),
      );

      tap();
      tap(); // the second half of the double-click, while the first is in flight
      await flush();
      resolvePlane(FACE_PLANE);
      await flush();

      expect(clientMock.faceSketchPlane).toHaveBeenCalledTimes(1);
      expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    });

    it("Esc during the pick clears the face hover highlight", async () => {
      await armPicker();
      bodyHit = faceHit();
      container.dispatchEvent(new MouseEvent("pointermove", { clientX: 5, clientY: 5, bubbles: true }));
      expect(selectionStore.getState().hover).not.toBeNull();

      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
      await flush();

      expect(toolStore.getState().mode).toBe("model");
      expect(selectionStore.getState().hover).toBeNull();
    });
  });

  // ── (c) double-click a face in model mode ──────────────────────────────────

  describe("double-click entry (enterOnFace)", () => {
    const pick = (): FacePickTarget => ({
      bodyId: "body1",
      topoKey: "f:22",
      worldPoint: [1, 2, 25],
    });

    it("no sketch hosted there ⇒ a NEW one is created on the face", async () => {
      await controller.enterOnFace(pick());
      await flush();

      expect(clientMock.faceSketchPlane).toHaveBeenCalledWith("body1", "el_body1_f:22", "f:22");
      expect(clientMock.enterSketch).toHaveBeenCalledWith({
        newOnFace: { bodyId: "body1", elementId: "el_body1_f:22", topoKey: "f:22", worldPoint: [1, 2, 25] },
        plane: FACE_PLANE,
      });
      expect(toolStore.getState().mode).toBe("sketch");
      expect(viewportStore.getState().activeSketchId).toBe("sketchNew");
    });

    it("an EXISTING host sketch is re-entered — the NEWEST (last) of several", async () => {
      const doc = documentStore.getState();
      doc.addSketch(sketchOnFace("onFaceOld", "el_body1_f:22"));
      doc.addSketch(sketchOnFace("otherFace", "el_body1_f:99"));
      doc.addSketch(sketchOnFace("onFaceNew", "el_body1_f:22"));

      await controller.enterOnFace(pick());
      await flush();

      // Re-entry, not creation: no plane preflight, no `newOnFace` target.
      expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
      expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
      expect(clientMock.enterSketch).toHaveBeenCalledWith("onFaceNew");
      expect(toolStore.getState().mode).toBe("sketch");
      expect(viewportStore.getState().activeSketchId).toBe("onFaceNew");
    });

    it("a sketch on ANOTHER face is not a match — a new sketch is created", async () => {
      documentStore.getState().addSketch(sketchOnFace("otherFace", "el_body1_f:99"));

      await controller.enterOnFace(pick());
      await flush();

      expect(clientMock.enterSketch).toHaveBeenCalledWith(
        expect.objectContaining({ newOnFace: expect.objectContaining({ elementId: "el_body1_f:22" }) }),
      );
    });

    it("a NON-PLANAR face hints and leaves the mode alone (no picker, no chrome)", async () => {
      clientMock.faceSketchPlane.mockRejectedValueOnce(new Error("only a planar face can host a sketch"));

      await controller.enterOnFace(pick());
      await flush();

      expect(clientMock.enterSketch).not.toHaveBeenCalled();
      expect(toolStore.getState().mode).toBe("model");
      expect(engineMock.setPlanePickerVisible).not.toHaveBeenCalled();
      const hint = viewportStore.getState().statusHint;
      expect(hint?.message).toMatch(/Cannot sketch on that face: only a planar face can host a sketch/);
      expect(hint?.severity).toBe("error");
    });

    it("a pick that cannot be identified hints instead of guessing", async () => {
      clientMock.promoteSelection.mockResolvedValueOnce([]);

      await controller.enterOnFace(pick());
      await flush();

      expect(clientMock.faceSketchPlane).not.toHaveBeenCalled();
      expect(toolStore.getState().mode).toBe("model");
      expect(viewportStore.getState().statusHint?.message).toMatch(/could not be identified/);
    });

    it("is a MODEL-MODE gesture: a call while already sketching is a no-op", async () => {
      toolStore.getState().setMode("sketch", "sketch2");
      await flush();
      clientMock.enterSketch.mockClear();
      clientMock.promoteSelection.mockClear();

      await controller.enterOnFace(pick());
      await flush();

      expect(clientMock.promoteSelection).not.toHaveBeenCalled();
      expect(clientMock.enterSketch).not.toHaveBeenCalled();
      expect(viewportStore.getState().activeSketchId).toBe("sketch2");
    });

    it("two overlapping double-clicks create exactly ONE sketch", async () => {
      let resolvePlane!: (p: SketchPlane) => void;
      clientMock.faceSketchPlane.mockImplementationOnce(
        () => new Promise<SketchPlane>((r) => { resolvePlane = r; }),
      );

      const first = controller.enterOnFace(pick());
      await flush();
      await controller.enterOnFace(pick()); // in-flight ⇒ dropped
      resolvePlane(FACE_PLANE);
      await first;
      await flush();

      expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    });
  });

  // ── the newest-host rule, directly ─────────────────────────────────────────

  describe("newestSketchOnFace", () => {
    it("returns the LAST matching entry in projection order", () => {
      const sketches = {
        a: sketchOnFace("a", "el_1"),
        b: sketchOnFace("b", "el_2"),
        c: sketchOnFace("c", "el_1"),
      };
      expect(newestSketchOnFace(sketches, "el_1")).toBe("c");
      expect(newestSketchOnFace(sketches, "el_2")).toBe("b");
    });

    it("world/datum sketches carry no host and never match", () => {
      const world: SketchMeta = { id: "w", name: "w", visible: true, dof: 3, status: "under", geometryToken: "t" };
      expect(newestSketchOnFace({ w: world }, "el_1")).toBeNull();
    });
  });
});
