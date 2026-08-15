/*
 * SketchController × live dimensions, Wave 2 (jsdom): the CONTROLLER wiring, not
 * the pure core (liveDimension/liveDimFrames/liveDimConstraints/liveToolMachines
 * own their own suites). What is under test here is only what the controller
 * decides:
 *   - WHETHER to round (pref · Alt · a geometry snap already won) and at WHICH
 *     quantum (camera distance → chooseGridStep → dimQuantum),
 *   - that a LOCK projects into the committed geometry AND authors its driving
 *     constraint in the SAME sketchUpsert as the entity (one undo step),
 *   - that a conflicting dimension is dropped while the ENTITY survives, still
 *     for exactly one undo step.
 * Chips and keyboard capture land in Wave 3 — locks are set directly on the
 * controller's `liveDim` state here, which is precisely the seam the chip FSM
 * will write through.
 *
 * Harness mirrors SketchController.draw.test.ts: faked engine + client, 1:1
 * screenToPlane so client coords ARE plane coords, every frontend snap type off
 * unless a test turns one on, commits drained through `flushSketchMutations`.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchSession,
  SketchUpsertResult,
} from "@/ipc/types";
import { toolStore, type SketchTool } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "./sketchService";
import { liveDimInit, type LiveDimState } from "./liveDimension";
import { liveDimStore } from "@/stores/liveDimStore";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/**
 * chooseGridStep(100).minor = 5 mm ⇒ dimQuantum = 0.5 mm (a tenth of the minor
 * cell, floored onto the 1/2/5 ladder). Pinned here because every rounding
 * expectation below is stated in it.
 */
const CAMERA_DISTANCE = 100;
const QUANTUM_MM = 0.5;

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    clearPlanePickerHover: vi.fn(),
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
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    getCameraDistance: vi.fn(() => CAMERA_DISTANCE),
  };
}

function okResult(sketchId: string): SketchUpsertResult {
  return {
    sketchId,
    sketchRevision: 1,
    dof: 0,
    status: "UnderConstrained",
    conflicting: [],
    solvedPositions: {},
  };
}

function makeClientMock() {
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    sketchUpsert: vi.fn(
      (
        sketchId: string,
        _entities: SketchEntity[],
        _constraints: SketchConstraint[],
      ): Promise<SketchUpsertResult> => Promise.resolve(okResult(sketchId)),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** Every frontend snap type off — a test that wants one turns it back on. */
function disableSnapping(): void {
  const s = settingsStore.getState();
  for (const key of [
    "grid",
    "sketchGuideLines",
    "sketchGuidePoints",
    "quadrant",
    "intersection",
    "onCurve",
  ] as const) {
    s.setSnap(key, false);
  }
}

describe("SketchController live dimensions (Wave 2 wiring)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores(); // dimensionRound: false — each test opts in explicitly
    disableSnapping();
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1"); // default tool = line
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  async function setTool(tool: SketchTool): Promise<void> {
    toolStore.getState().setTool(tool);
    await flush();
  }

  const session = (): SketchSession => sketchStore.getState().session!;
  const entities = (): SketchEntity[] => session().entities;

  type Internals = { liveDim: LiveDimState; lastChainLineId: string | null };
  const internals = (): Internals => controller as unknown as Internals;

  /** Pin a field exactly as the Wave-3 chip FSM will. */
  function lock(field: "length" | "radius" | "width", value: number): void {
    internals().liveDim = { ...liveDimInit(), locks: { [field]: value } };
  }

  const lengthOf = (e: SketchEntity): number => Math.hypot(e.p1![0] - e.p0![0], e.p1![1] - e.p0![1]);
  const angleDegOf = (e: SketchEntity): number =>
    (Math.atan2(e.p1![1] - e.p0![1], e.p1![0] - e.p0![0]) * 180) / Math.PI;

  const enable = (): void => settingsStore.getState().setSnap("dimensionRound", true);

  // ── cursor-placed rounding ──────────────────────────────────────────────────

  it("rounds a cursor-placed segment to the zoom quantum (length AND whole degrees)", async () => {
    enable();
    click(0, 0);
    click(10, 3); // raw: 10.4403 mm @ 16.699° — neither on a ladder rung
    await flushSketchMutations();

    const line = entities()[0];
    expect(lengthOf(line)).toBeCloseTo(10.5, 9); // 10.4403 → 21 × 0.5
    expect(angleDegOf(line)).toBeCloseTo(17, 9); // 16.699 → 17 (1° quantum)
    // The anchor is untouched — only the moving end is re-placed.
    expect(line.p0).toEqual([0, 0]);
  });

  it("the quantum follows the CAMERA, not a constant (zoomed out ⇒ coarser)", async () => {
    enable();
    // chooseGridStep(1000).minor = 50 mm ⇒ quantum 5 mm.
    engineMock.getCameraDistance.mockReturnValue(1000);
    click(0, 0);
    click(10, 3);
    await flushSketchMutations();

    expect(lengthOf(entities()[0])).toBeCloseTo(10, 9); // 10.4403 → 2 × 5
  });

  it("pref OFF ⇒ the raw click coordinate commits verbatim", async () => {
    click(0, 0);
    click(10, 3);
    await flushSketchMutations();

    expect(entities()[0]).toMatchObject({ p0: [0, 0], p1: [10, 3] });
  });

  it("Alt suppresses rounding for as long as it is held", async () => {
    enable();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", bubbles: true }));
    click(0, 0);
    click(10, 3);
    await flushSketchMutations();
    expect(entities()[0]).toMatchObject({ p1: [10, 3] });

    window.dispatchEvent(new KeyboardEvent("keyup", { key: "Alt", bubbles: true }));
    click(20, 6); // fresh chain leg off the previous end — rounding is back
    click(30, 9);
    await flushSketchMutations();
    const len = lengthOf(entities()[1]);
    expect(Math.round(len / QUANTUM_MM) * QUANTUM_MM).toBeCloseTo(len, 9);
  });

  it("a GEOMETRY snap beats rounding — the snapped coordinate is bit-identical", async () => {
    // An endpoint at a deliberately un-round place, drawn with rounding off.
    click(0, 0);
    click(10, 3);
    await flushSketchMutations();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    enable();
    settingsStore.getState().setSnap("sketchGuidePoints", true); // endpoint snapping
    click(100, 100); // far from every candidate — plain first anchor
    click(10.2, 3.1); // within the 8px reach of the endpoint at (10,3)
    await flushSketchMutations();

    // Rounding the 130-ish mm length would have dragged the end off the endpoint.
    expect(entities()[1].p1).toEqual([10, 3]);
  });

  it("grid beats rounding for the SECOND point too, when the cursor is close to an intersection", async () => {
    // Regression: grid used to be disabled outright for any point past the
    // first whenever dimensionRound was on, so a 2nd+ vertex could never land
    // on a grid line even with the cursor right on top of one.
    engineMock.getCameraDistance.mockReturnValue(1000); // chooseGridStep(1000).minor = 50
    enable();
    settingsStore.getState().setSnap("grid", true);
    click(0, 0); // first anchor, trivially on-grid
    click(52, 2); // 2.8 units from grid intersection (50,0) — well inside the 8px reach
    await flushSketchMutations();

    expect(entities()[0].p1).toEqual([50, 0]);
  });

  it("rounding is the fallback when the cursor is nowhere near a grid intersection", async () => {
    engineMock.getCameraDistance.mockReturnValue(1000); // gridStep 50, quantum 5mm
    enable();
    settingsStore.getState().setSnap("grid", true);
    click(0, 0);
    click(25, 25); // cell-center, 35.4 units from the nearest intersection — outside reach
    await flushSketchMutations();

    const line = entities()[0];
    expect(lengthOf(line)).toBeCloseTo(35, 9); // 35.355 → 7 × 5mm quantum, not a grid vertex
    expect(line.p1).not.toEqual([50, 50]);
    expect(line.p1).not.toEqual([0, 0]);
  });

  // ── locks project into geometry AND into constraints ────────────────────────

  it("a locked length pins the segment exactly and rides the SAME upsert as the entity", async () => {
    click(0, 0);
    lock("length", 50);
    click(10, 0); // steering the direction only — the length is pinned

    await flushSketchMutations();
    expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);

    const line = entities()[0];
    expect(line).toMatchObject({ type: "Line", p0: [0, 0], p1: [50, 0] });

    const dim = session().constraints.find((c) => c.type === "Distance");
    expect(dim).toBeDefined();
    expect(dim!.value).toBe(50);
    expect(dim!.entities).toEqual([line.id, line.id]);
    expect(dim!.positions).toEqual(["Start", "End"]);

    // ONE undo step for the whole click — entity and dimension together.
    expect(sketchStore.getState().undoStack).toHaveLength(1);
  });

  it("a lock is exact, never re-rounded by the quantum", async () => {
    enable(); // quantum 0.5 mm — 50.3 is not on it
    click(0, 0);
    lock("length", 50.3);
    click(10, 0);
    await flushSketchMutations();

    expect(lengthOf(entities()[0])).toBeCloseTo(50.3, 9);
    expect(session().constraints.find((c) => c.type === "Distance")!.value).toBe(50.3);
  });

  it("committing consumes the locks (the next segment is free again)", async () => {
    click(0, 0);
    lock("length", 50);
    click(10, 0);
    expect(internals().liveDim.locks).toEqual({}); // cleared synchronously, at the click

    click(10, 40); // next chain leg — unpinned, so the raw click stands
    await flushSketchMutations();
    expect(entities()[1]).toMatchObject({ p0: [50, 0], p1: [10, 40] });
  });

  it("a polygon radius lock binds the CONSTRUCTION circumcircle (index = side count)", async () => {
    await setTool("polygon");
    click(0, 0); // centre
    lock("radius", 40);
    click(50, 0); // vertex direction only — the radius is pinned
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(7); // 6 ring lines + circumcircle
    expect(ents[6]).toMatchObject({ type: "Circle", construction: true });
    expect(ents[6].radius).toBeCloseTo(40, 9);

    const radius = session().constraints.find((c) => c.type === "Radius");
    expect(radius).toBeDefined();
    expect(radius!.value).toBe(40);
    expect(radius!.entities).toEqual([ents[6].id]);
  });

  it("locks do NOT survive a tool change", async () => {
    click(0, 0);
    lock("length", 50);
    await setTool("rect");
    expect(internals().liveDim.locks).toEqual({});
    expect(internals().lastChainLineId).toBeNull();
  });

  // ── conflict fallback ───────────────────────────────────────────────────────

  it("a conflicting dimension is dropped, the ENTITY survives, and undo stays at one step", async () => {
    let call = 0;
    let firstBatch: SketchConstraint[] = [];
    let secondBatch: SketchConstraint[] = [];
    clientMock.sketchUpsert.mockImplementation(
      (sketchId: string, _e: SketchEntity[], constraints: SketchConstraint[]) => {
        call += 1;
        if (call === 1) {
          firstBatch = constraints;
          // Name the inferred Horizontal as the clashing party (what a real solve
          // reports; the mock lane reports none and falls back to generic text).
          return Promise.resolve({
            ...okResult(sketchId),
            status: "OverConstrained" as const,
            conflicting: [constraints[0].id],
          });
        }
        secondBatch = constraints;
        return Promise.resolve(okResult(sketchId));
      },
    );

    click(0, 0);
    lock("length", 50);
    click(10, 0);
    await flushSketchMutations();

    expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(2);
    expect(firstBatch.some((c) => c.type === "Distance")).toBe(true);
    expect(secondBatch.some((c) => c.type === "Distance")).toBe(false);

    // The user DREW the segment — it stays, at the length they typed.
    const ents = entities();
    expect(ents).toHaveLength(1);
    expect(ents[0]).toMatchObject({ type: "Line", p0: [0, 0], p1: [50, 0] });
    expect(session().constraints.some((c) => c.type === "Distance")).toBe(false);
    expect(session().constraints.some((c) => c.type === "Horizontal")).toBe(true);

    // ONE snapshot for the whole click, not one per upsert.
    expect(sketchStore.getState().undoStack).toHaveLength(1);
    // …and the hint NAMES what it clashed with.
    expect(viewportStore.getState().statusHint?.message).toContain("conflicts with Horizontal");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("no locks ⇒ no second upsert even when the solve reports a conflict", async () => {
    clientMock.sketchUpsert.mockImplementation((sketchId: string) =>
      Promise.resolve({ ...okResult(sketchId), status: "OverConstrained" as const }),
    );
    click(0, 0);
    click(50, 0);
    await flushSketchMutations();

    // The dim fallback is for TYPED dimensions only — an over-constrained solve
    // from the ordinary draw path is reported by the badge, not re-upserted.
    expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
    expect(entities()).toHaveLength(1);
  });
});

/*
 * Wave 3 — the CHIPS and the keys that reach them. Everything here goes through
 * the two seams the UI actually uses: the capture-phase keydown (a digit over the
 * viewport), and the handlers the controller injects into `liveDimStore` (which
 * is exactly what `LiveDimField` calls once it has parsed + validated its text).
 * The field component itself is covered by LiveDimChips.test.tsx.
 */
describe("SketchController live dimensions (Wave 3 chips + typing)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    disableSnapping();
    // The pointer-move lane is rAF-coalesced; run it inline so a move is
    // observable on the next line.
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
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  function move(x: number, y: number): void {
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }));
  }

  /** A key typed over the VIEWPORT (target = window, never an input). */
  function type(key: string): void {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  }

  async function setTool(tool: SketchTool): Promise<void> {
    toolStore.getState().setTool(tool);
    await flush();
  }

  const chips = (): ReturnType<typeof liveDimStore.getState> => liveDimStore.getState();
  const handlers = () => chips().handlers!;
  const fieldValue = (field: string): number =>
    chips().fields.find((f) => f.field === field)!.value;
  const session = (): SketchSession => sketchStore.getState().session!;
  const entities = (): SketchEntity[] => session().entities;
  const hint = (): string | undefined => viewportStore.getState().statusHint?.message;

  type Internals = { liveDim: LiveDimState };
  const internals = (): Internals => controller as unknown as Internals;

  // ── the chips appear, and a digit opens one ────────────────────────────────

  it("an armed gesture publishes chips; a digit over the viewport opens the first", () => {
    click(0, 0);
    move(30, 0);
    expect(chips().open).toBe(true);
    // A FIRST leg has no line to be angled against — only the length chip lives.
    expect(chips().fields.map((f) => f.field)).toEqual(["length"]);
    expect(fieldValue("length")).toBeCloseTo(30, 9);

    type("5");
    expect(chips().focus).toBe("length");
    expect(chips().text).toBe("5"); // the opening character is already in the field
    expect(internals().liveDim.locks).toEqual({}); // …but nothing is pinned yet
  });

  it("suppresses the snap hint label while a live-dim chip set is open", () => {
    // Before any chips are open, the hint preference is honored as-is.
    move(5, 0);
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(expect.anything(), true);

    click(0, 0);
    move(30, 0); // opens the chip set
    move(31, 0); // one frame later `liveDimsOpen` is reflected in `showHints`
    expect(chips().open).toBe(true);
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(expect.anything(), false);

    // Closing the gesture (Escape) restores the hint.
    type("Escape");
    move(32, 0);
    expect(chips().open).toBe(false);
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(expect.anything(), true);
  });

  it("a chained SECOND leg gains the angle chip (sketching against a line)", () => {
    click(0, 0);
    click(10, 0); // commits leg 1 and chains
    move(30, 0);
    expect(chips().fields.map((f) => f.field)).toEqual(["length", "angle"]);
  });

  it("a chained leg highlights its reference segment and previews the angle arc", async () => {
    click(0, 0);
    click(10, 0); // commits leg 1 (the reference) and chains
    await flushSketchMutations();
    const refId = entities()[0].id;
    move(30, 0);

    expect(engineMock.setSketchAngleReference).toHaveBeenLastCalledWith(refId);
    // fromDeg is the OUTGOING ray of the reference segment (180° from its
    // incoming/authoring direction, 0°) — the visible half of the old line
    // extending from the vertex, per `arcPreviewSweep`.
    expect(engineMock.setSketchAnglePreview).toHaveBeenLastCalledWith(
      expect.objectContaining({ center: { x: 10, y: 0 }, fromDeg: 180 }),
    );

    // Escape drops the gesture — both clear.
    type("Escape");
    expect(engineMock.setSketchAngleReference).toHaveBeenLastCalledWith(null);
    expect(engineMock.setSketchAnglePreview).toHaveBeenLastCalledWith(null);
  });

  it("the angle chip's anchor moves to the arc's bisector, not segmentFrame's own anchor", async () => {
    click(0, 0);
    click(10, 0); // commits leg 1 (reference), vertex at (10, 0)
    await flushSketchMutations();
    move(30, 0); // new leg heads +x (0°); reference's outgoing ray is 180° ⇒ sweep 180→360

    // chooseGridStep(100).minor = 5 mm (pinned by this file's CAMERA_DISTANCE
    // comment) ⇒ arc radius = 5 × 3 = 15, label radius = 15 × 0.85 = 12.75;
    // bisector of [180°, 360°] = 270° (straight down in plane coords) ⇒
    // label at (10, 0) + 12.75 × (0, −1).
    const angleCalls = engineMock.moveChip.mock.calls.filter((c) => c[0] === "__live_dim_angle");
    const [, lastAt] = angleCalls[angleCalls.length - 1];
    expect(lastAt[0]).toBeCloseTo(10, 6);
    expect(lastAt[1]).toBeCloseTo(-12.75, 6);
  });

  it("a FIRST leg (no reference yet) never highlights or previews", () => {
    click(0, 0);
    move(30, 0);
    expect(chips().fields.map((f) => f.field)).toEqual(["length"]); // no angle chip
    expect(engineMock.setSketchAngleReference).toHaveBeenLastCalledWith(null);
    expect(engineMock.setSketchAnglePreview).toHaveBeenLastCalledWith(null);
  });

  it("with no anchor placed there is no dimension, so no chip and no capture", () => {
    move(30, 0);
    expect(chips().open).toBe(false);

    type("5");
    expect(chips().focus).toBeNull();
    expect(chips().open).toBe(false);
  });

  it("the chips follow the cursor without React: one moveChip per field per move", () => {
    click(0, 0);
    engineMock.moveChip.mockClear();
    move(30, 0);
    // One field on a first leg — the angle chip is withheld until a reference.
    expect(engineMock.moveChip).toHaveBeenCalledTimes(1);
    expect(engineMock.moveChip.mock.calls.map((c: unknown[]) => c[0])).toEqual([
      "__live_dim_length",
    ]);
  });

  it("the pref gates the whole lane — no chips, and digits stay unclaimed", () => {
    settingsStore.getState().setShow("liveDimensions", false);
    click(0, 0);
    move(30, 0);
    expect(chips().open).toBe(false);

    type("5");
    expect(chips().focus).toBeNull();
  });

  // ── polygon: the same digit means two different things by phase ────────────

  it("polygon: an IDLE digit is still the side count", async () => {
    await setTool("polygon");
    expect(hint()).toBe("Polygon — 6 sides · 3–9 to change");

    type("5");
    expect(hint()).toBe("Polygon — 5 sides · 3–9 to change");
    expect(chips().open).toBe(false);
  });

  it("polygon: an ARMED digit types the radius and leaves the side count alone", async () => {
    await setTool("polygon");
    type("5"); // 5 sides, while idle
    click(0, 0);
    move(40, 0);
    expect(hint()).toBe("Polygon — 5 sides · type to set radius · Tab for sides");

    type("3");
    expect(chips().focus).toBe("radius");
    expect(chips().text).toBe("3");
    expect(fieldValue("sides")).toBe(5); // untouched by the keystroke
  });

  it("polygon: locking the sides chip runs the MACHINE verb, not a geometry lock", async () => {
    await setTool("polygon");
    click(0, 0);
    move(40, 0);
    handlers().onFocus("sides");
    handlers().onTab(false, 5);

    expect(fieldValue("sides")).toBe(5);
    expect(fieldValue("radius")).toBeCloseTo(40, 9); // the count never moves the vertex
    expect(hint()).toContain("5 sides");
  });

  // ── Tab / Enter / Esc ──────────────────────────────────────────────────────

  it("Tab locks the field it leaves and cycles to the next", () => {
    click(0, 0);
    click(10, 0); // a chained second leg → a reference exists, so the ∠ chip returns
    move(30, 0);
    type("5");
    handlers().onTab(false, 50);

    expect(chips().focus).toBe("angle");
    expect(internals().liveDim.locks).toEqual({ length: 50 });
    // The lock is already in the geometry: the chip re-reads it off the re-stepped
    // machine, not off a separate copy.
    expect(fieldValue("length")).toBeCloseTo(50, 9);
    expect(chips().fields.find((f) => f.field === "length")!.locked).toBe(true);
  });

  it("Enter locks AND commits through the same tail as a mouse click", async () => {
    click(0, 0);
    move(30, 0);
    type("5");
    handlers().onText("50");
    handlers().onEnter(50);
    await flushSketchMutations();

    expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
    const line = entities()[0];
    expect(line).toMatchObject({ type: "Line", p0: [0, 0], p1: [50, 0] });

    const dim = session().constraints.find((c) => c.type === "Distance");
    expect(dim!.value).toBe(50);
    expect(dim!.entities).toEqual([line.id, line.id]);
    // ONE undo step — the entity and its typed dimension rode the same upsert.
    expect(sketchStore.getState().undoStack).toHaveLength(1);
    // The chain continues, so the chips re-open on the next leg with no locks.
    expect(internals().liveDim.locks).toEqual({});
    expect(chips().focus).toBeNull();
  });

  it("Esc on a chip drops THAT field's lock and leaves the gesture armed", () => {
    click(0, 0);
    move(30, 0);
    type("5");
    handlers().onTab(false, 50); // lock length, move to angle
    handlers().onFocus("length"); // click back onto the length chip
    handlers().onEscape();

    expect(internals().liveDim.locks).toEqual({});
    expect(chips().focus).toBeNull();
    expect(chips().open).toBe(true); // the gesture itself survives
    expect(entities()).toHaveLength(0);
  });

  it("a viewport click while a chip is focused LOCKS the value without committing", () => {
    click(0, 0);
    move(30, 0);
    type("5");
    handlers().onBlur("length", 50); // what LiveDimField's onBlur reports

    expect(internals().liveDim.locks).toEqual({ length: 50 });
    expect(chips().focus).toBeNull();
    expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
  });

  // e2e-found regression: Tab to the next chip makes React's focus effect
  // synchronously blur the DEPARTED input, whose onBlur fires AFTER the FSM has
  // already moved to the new field. That stale blur (field ≠ FSM focus) must be
  // dropped — processing it nulled the focus and swallowed every keystroke into
  // the second field, so the typed gesture never committed.
  it("a stale blur from the field Tab just left does NOT steal the new field's focus", () => {
    click(0, 0);
    click(10, 0); // chain a second leg so BOTH chips live (oblique, against a line)
    move(30, 20);
    type("5");
    handlers().onTab(false, 50); // lock length, FSM focus → angle
    expect(internals().liveDim.focus).toBe("angle");

    handlers().onBlur("length", null); // the departed input's DOM blur, empty text

    expect(internals().liveDim.focus).toBe("angle"); // survives the stale blur
    expect(internals().liveDim.locks).toEqual({ length: 50 });
  });

  // ── teardown ───────────────────────────────────────────────────────────────

  it("a tool change closes the chips", async () => {
    click(0, 0);
    move(30, 0);
    expect(chips().open).toBe(true);

    await setTool("rect");
    expect(chips().open).toBe(false);
    expect(chips().fields).toEqual([]);
    expect(chips().focus).toBeNull();
  });

  it("ending the chain with Escape closes the chips", () => {
    click(0, 0);
    move(30, 0);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(chips().open).toBe(false);
  });

  it("dispose closes the chips (React then unmounts every host)", () => {
    click(0, 0);
    move(30, 0);
    controller.dispose();
    expect(chips().open).toBe(false);
    expect(chips().fields).toEqual([]);
  });
});
