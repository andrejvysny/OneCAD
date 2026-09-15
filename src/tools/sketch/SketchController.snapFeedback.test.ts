/*
 * SketchController — the ONE WRITER of snap feedback (decision D8, UX review
 * S1/S4).
 *
 * Two things this pins:
 *
 *   1. THE INVARIANT. A scripted run of every event that changes what a pointer
 *      sample MEANS, asserting after each one that what the indicator was told
 *      and what `viewportStore.snapFeedback` holds are the same decision — or
 *      that both are cleared. Before this, `bumpInteraction` reset the latch
 *      without touching the feedback, so the settings subscriber and Alt (its
 *      only solo callers) left a marker and a hint chip on screen describing a
 *      decision taken under rules that no longer applied, and the status-bar
 *      readout was a SEPARATE raycast that never agreed with either.
 *
 *      `engine.setSketchSnap` stands in for the indicator: `ViewportEngine`
 *      forwards its argument verbatim to `SnapIndicator.show`/`hide`, so the
 *      last call IS the indicator's visible state.
 *
 *   2. GRID SNAPPING SNAPS, after the first click of a gesture as well as
 *      before it (S1 / the session-31 follow-up). Measured cause: once a
 *      gesture has an anchor there is a live dimension frame, and the
 *      cursor-rounding candidates (a rounded length AND a rounded angle) resolve
 *      both plane degrees of freedom for a fraction of a pixel — so by score
 *      alone they outbid a grid crossing the cursor is practically sitting on,
 *      and the point lands off-grid with `snapped: false` and no badge.
 *
 * Harness mirrors SketchController.snapClear.test.ts (real pointer dispatch,
 * inline rAF), with snapping left ON and a seeded line so there is a real
 * endpoint to snap to.
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
import type { SnapDecision } from "./snapTypes";
import { toolStore } from "@/stores/toolStore";
import { settingsStore } from "@/stores/settingsStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "./sketchService";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** A committed leg from (0,0) to (100,0) — its End is the endpoint under test. */
const SEED: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [100, 0] }];

/**
 * `pxPerUnit` is the plane→screen scale: the metric AND `screenToPlane` are
 * derived from it together, so a client pixel and a plane unit never disagree.
 */
function makeEngineMock(opts: { pxPerUnit?: number; cameraDistance?: number } = {}) {
  const s = opts.pxPerUnit ?? 1;
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
    setSketchProjectedIds: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x: x / s, y: y / s })),
    planePixelWorld: vi.fn(() => 1 / s),
    planeScreenMetric: vi.fn(() => ({ m00: s, m01: 0, m10: 0, m11: s })),
    getCameraDistance: vi.fn(() => opts.cameraDistance ?? 100),
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

function makeClientMock(entities: SketchEntity[]) {
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: entities.map((e) => ({ ...e })),
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

interface Harness {
  engine: ReturnType<typeof makeEngineMock>;
  container: HTMLDivElement;
  controller: SketchController;
}

async function mount(
  opts: { pxPerUnit?: number; cameraDistance?: number; entities?: SketchEntity[] } = {},
): Promise<Harness> {
  const engine = makeEngineMock(opts);
  const client = makeClientMock(opts.entities ?? []);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const controller = new SketchController({
    engine: engine as unknown as ViewportEngine,
    client: client as unknown as CadClient,
    container,
  });
  toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
  await flush();
  return { engine, container, controller };
}

const click = (c: HTMLElement, x: number, y: number): void => {
  c.dispatchEvent(
    new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
  );
  c.dispatchEvent(
    new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
  );
};

const move = (c: HTMLElement, x: number, y: number): void => {
  c.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }));
};

describe("SketchController — snap feedback has ONE writer (D8)", () => {
  beforeEach(() => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    resetStores();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("indicator ≡ store after every event that changes a pointer sample's meaning", async () => {
    const h = await mount({ entities: SEED });
    const { engine, container, controller } = h;

    /** What the indicator was last told (null ⇒ hidden). */
    const indicator = (): SnapDecision | null => {
      const calls = engine.setSketchSnap.mock.calls;
      return (calls[calls.length - 1]?.[0] ?? null) as SnapDecision | null;
    };

    /** The invariant, checked after EVERY scripted event. */
    const expectAgreement = (why: string): void => {
      const shown = indicator();
      const stored = viewportStore.getState().snapFeedback;
      if (shown === null || !shown.snapped) {
        expect(stored, `${why}: indicator hidden but the store still holds a decision`).toBeNull();
        return;
      }
      expect(stored, `${why}: indicator visible but the store holds nothing`).not.toBeNull();
      expect(stored!.point, why).toEqual(shown.point);
      expect(stored!.primaryKind, why).toBe(shown.primaryKind);
      expect(stored!.label, why).toBe(shown.label);
      expect(stored!.guides, why).toEqual(shown.guides);
    };

    // 1. Near the seeded endpoint — a live decision.
    move(container, 99, 0);
    expect(viewportStore.getState().snapFeedback?.primaryKind).toBe("endpoint");
    expect(viewportStore.getState().snapFeedback?.point).toEqual({ x: 100, y: 0 });
    expectAgreement("hover on endpoint");

    // 2. Away from everything.
    move(container, 43, 37);
    expectAgreement("hover in open space");

    // 3. A click (arms the chain — the readout keeps naming the placed point).
    click(container, 99, 0);
    await flushSketchMutations();
    expectAgreement("arming click");

    // 4. Escape ends the chain.
    move(container, 60, 20);
    expectAgreement("hover mid-chain");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("escape");

    // 5. Alt down suppresses snapping — the feedback must clear in the SAME
    //    frame, not at the next pointer move.
    move(container, 99, 0);
    expect(viewportStore.getState().snapFeedback).not.toBeNull();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", bubbles: true }));
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("alt down");
    window.dispatchEvent(new KeyboardEvent("keyup", { key: "Alt", bubbles: true }));
    expectAgreement("alt up");

    // 6. A snap-SOURCE toggle re-poses the whole candidate field.
    move(container, 99, 0);
    expect(viewportStore.getState().snapFeedback).not.toBeNull();
    settingsStore.getState().setSnap("grid", false);
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("settings toggle");

    // 7. Tool switch.
    move(container, 99, 0);
    toolStore.getState().setTool("circle");
    await flush();
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("tool switch");

    // 8. Window blur.
    move(container, 99, 0);
    window.dispatchEvent(new Event("blur"));
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("blur");

    // 9. Pointer leaves the canvas.
    move(container, 99, 0);
    container.dispatchEvent(new MouseEvent("pointerout", { bubbles: true }));
    container.dispatchEvent(new MouseEvent("pointerleave", { bubbles: true }));
    expect(viewportStore.getState().snapFeedback).toBeNull();
    expectAgreement("pointerleave");

    controller.dispose();
    container.remove();
  });

  /*
   * S3 — closing a loop must be visible BEFORE the click. The chain's open
   * start is an ordinary endpoint snap; what changes is the name.
   */
  it("names the chain's open start 'Close loop'", async () => {
    const h = await mount();
    const { container, controller } = h;

    // A two-anchor chain, well away from the origin (whose own snap would
    // outrank an endpoint sitting on it).
    click(container, 200, 200);
    await flushSketchMutations();
    click(container, 200, 260);
    await flushSketchMutations();

    // Back over the chain's START — an endpoint snap that would close the loop.
    move(container, 201, 201);
    const closing = viewportStore.getState().snapFeedback;
    expect(closing?.primaryKind).toBe("endpoint");
    expect(closing?.point).toEqual({ x: 200, y: 200 });
    expect(closing?.label).toBe("Close loop");
    // …and the hint CHIP shows it even though the live-dim chips are open,
    // which normally suppresses the label.
    expect(h.engine.setSketchSnap).toHaveBeenLastCalledWith(expect.anything(), true);

    // The chain's CURRENT end is an ordinary endpoint — clicking there ends the
    // chain, it does not close a loop.
    move(container, 201, 259);
    const elsewhere = viewportStore.getState().snapFeedback;
    expect(elsewhere?.primaryKind).toBe("endpoint");
    expect(elsewhere?.label).toBe("Endpoint");

    controller.dispose();
    container.remove();
  });

  /*
   * The review's numbers: a 20 mm grid (camera distance 500 ⇒
   * `chooseGridStep().minor === 20`), snap radius M (8px) and a zoom of 2 px per
   * mm, so one cell projects 40px across and the grid's own reach is
   * min(8 × 1.25, 0.42 × 40) = 10px.
   */
  it("snaps to a 20 mm grid crossing 3px away, before and after the first click", async () => {
    const h = await mount({ pxPerUnit: 2, cameraDistance: 500 });
    const { container, controller } = h;
    // `resetStores()` forces cursor rounding OFF for every vitest (the pointer
    // specs pin raw click coords). It is ON by default in the app, and it is
    // the source that competes with the grid — so the whole point of these two
    // tests is lost without it.
    settingsStore.getState().setSnap("dimensionRound", true);

    // Hover 3px (1.5 mm) off the crossing at plane (20, 0) ⇒ client (40, 0).
    move(container, 43, 0);
    expect(viewportStore.getState().snapFeedback?.primaryKind).toBe("grid");
    expect(viewportStore.getState().snapFeedback?.point).toEqual({ x: 20, y: 0 });

    click(container, 2, 6); // first anchor, well away from that crossing
    await flushSketchMutations();
    move(container, 43, 0);

    const after = viewportStore.getState().snapFeedback;
    expect(after?.primaryKind).toBe("grid");
    expect(after?.point).toEqual({ x: 20, y: 0 });

    controller.dispose();
    container.remove();
  });

  /*
   * THE MEASURED FAILURE (S1's own configuration: "Grid snap is ON by default,
   * grid scale 5 mm"). Camera distance 100 ⇒ a 5 mm minor cell, at 2 px per mm
   * that is a 10px cell, so the grid's reach is min(8 × 1.25, 0.42 × 10) =
   * 4.2px and the rounding quantum is 0.5 mm (1px).
   *
   * Before the fix this hover — ONE pixel off a crossing, comfortably inside
   * the grid's reach — resolved to (20.26, −0.05) with kind `none`: the
   * length-rounding candidate cost 0.46px and the angle-rounding candidate
   * 0.17px, 0.88px in total against the crossing's 1.0px, so the invisible pair
   * won and the cursor tracked continuously through the crossing with no badge
   * and no lock. The 20 mm case above does NOT reproduce it — the coarser grid
   * has a 2 mm quantum, whose rounding costs more than the crossing does.
   */
  it("snaps to a 5 mm grid crossing 1px away once the gesture has an anchor", async () => {
    const h = await mount({ pxPerUnit: 2, cameraDistance: 100 });
    const { container, controller } = h;
    settingsStore.getState().setSnap("dimensionRound", true); // app default

    click(container, 2, 6); // first anchor ⇒ a live dimension frame exists
    await flushSketchMutations();
    move(container, 41, 0); // plane (20.5, 0) — 1px off the crossing at (20, 0)

    const at = viewportStore.getState().snapFeedback;
    expect(at?.primaryKind).toBe("grid");
    expect(at?.point).toEqual({ x: 20, y: 0 });

    controller.dispose();
    container.remove();
  });
});
