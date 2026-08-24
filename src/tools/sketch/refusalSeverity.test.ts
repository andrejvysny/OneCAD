/*
 * Sketch REFUSALS carry error severity — one pin per refusal path.
 *
 * Severity is not decoration here: it is the switch both feedback surfaces read.
 * `StatusBar` renders an error red and replays its one-shot pulse, and
 * `SketchErrorPulse` (audit item #10's near-action residual) shows a refusal at
 * the cursor ONLY for `severity === "error"`. These paths used to publish their
 * refusal as an ordinary info hint, so the most common refusals — a conflicting
 * constraint, a rejected dimension, an edit on locked reference geometry —
 * announced themselves in exactly the same grey as "click a line".
 *
 * TEXTS ARE UNCHANGED by that pass; these tests assert both, so a later reword
 * cannot quietly drop the severity with it.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  applyConstraint,
  deleteEntities,
  extendEntity,
  filletSketchCorner,
  flushSketchMutations,
} from "./sketchService";
import { evaluateApplicability } from "./constraintApplicability";
import { toConstraintTarget } from "./constraintTarget";
import { SketchController } from "./SketchController";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { CadClient } from "@/ipc/client";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { SketchConstraint, SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function seed(entities: SketchEntity[], constraints: SketchConstraint[] = []): void {
  sketchStore.getState().setSession({
    sketchId: "sk-refuse",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  });
}

/** A client whose FIRST upsert reports the set as Conflicting (blaming `blame`)
 *  and whose later ones (the revert) come back clean — the shape the reject
 *  branches are written against. */
function conflictingClient(blame: string[]): CadClient {
  let n = 0;
  return {
    sketchUpsert: () => {
      n += 1;
      return Promise.resolve(
        n === 1
          ? {
              sketchId: "sk-refuse",
              sketchRevision: 2,
              dof: 0,
              status: "Conflicting" as const,
              conflicting: blame,
              solvedPositions: {},
            }
          : {
              sketchId: "sk-refuse",
              sketchRevision: 3,
              dof: 0,
              status: "FullyConstrained" as const,
              conflicting: [],
              solvedPositions: {},
            },
      );
    },
  } as unknown as CadClient;
}

const hint = () => viewportStore.getState().statusHint;

describe("sketch refusals publish at error severity", () => {
  beforeEach(() => {
    resetStores();
    resetMockSketches();
    sketchStore.getState().reset();
    toolChipStore.getState().clear();
  });

  it("locked geometry: delete", async () => {
    seed([{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0], referenceLocked: true }]);
    await deleteEntities(mockClient, ["e1"]);
    expect(hint()?.message).toMatch(/Reference geometry is locked.*cannot be deleted/);
    expect(hint()?.severity).toBe("error");
  });

  it("locked geometry: extend", async () => {
    seed([
      { id: "e1", type: "Line", p0: [0, 0], p1: [10, 0], referenceLocked: true },
      { id: "w", type: "Line", p0: [20, -50], p1: [20, 50] },
    ]);
    await extendEntity(mockClient, "e1", [9, 0], { minSize: 0.1 });
    expect(hint()?.message).toMatch(/Reference geometry is locked.*cannot be extended/);
    expect(hint()?.severity).toBe("error");
  });

  it("locked geometry: fillet", async () => {
    seed([
      { id: "e1", type: "Line", p0: [0, 0], p1: [100, 0], referenceLocked: true },
      { id: "e2", type: "Line", p0: [0, 0], p1: [0, 80] },
    ]);
    await filletSketchCorner(mockClient, "e1", "e2", 5, { tol: 0.25 });
    expect(hint()?.message).toMatch(/Reference geometry is locked.*cannot be filleted/);
    expect(hint()?.severity).toBe("error");
  });

  it("a geometric constraint rejected on conflict", async () => {
    // One line (4 DOF) already consumed by two Coincidents; +Horizontal is over.
    const entities: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];
    seed(entities, [
      { id: "k1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] },
      { id: "k2", type: "Coincident", entities: ["e1", "e1"], positions: ["End", "Start"] },
    ]);
    const horiz = evaluateApplicability(
      [toConstraintTarget({ entityId: "e1" }, entities)!],
      entities,
    ).find((a) => a.type === "Horizontal")!;

    const { rejected } = await applyConstraint(mockClient, horiz);
    expect(rejected).toBe(true);
    expect(hint()?.message).toMatch(/Constraint removed/);
    expect(hint()?.severity).toBe("error");
  });

  it("a dimension rejected on conflict from the applied-dimension chip", async () => {
    const entities: SketchEntity[] = [{ id: "e1", type: "Circle", center: [0, 0], radius: 5 }];
    seed(entities, []);
    const client = conflictingClient([]);
    const radius = evaluateApplicability(
      [toConstraintTarget({ entityId: "e1" }, entities)!],
      entities,
    ).find((a) => a.type === "Radius")!;

    // The dimensional route ARMS a chip; the refusal happens on its Enter.
    await applyConstraint(client, radius);
    expect(toolChipStore.getState().kind).toBe("dimension");
    toolChipStore.getState().onValue!(8);
    await flushSketchMutations();

    expect(hint()?.message).toMatch(/Dimension removed/);
    expect(hint()?.severity).toBe("error");
  });

  it("an ACCEPTED dimension still just clears the prompt (no phantom error)", async () => {
    const entities: SketchEntity[] = [{ id: "e1", type: "Circle", center: [0, 0], radius: 5 }];
    seed(entities, []);
    const radius = evaluateApplicability(
      [toConstraintTarget({ entityId: "e1" }, entities)!],
      entities,
    ).find((a) => a.type === "Radius")!;

    await applyConstraint(mockClient, radius);
    toolChipStore.getState().onValue!(8);
    await flushSketchMutations();

    expect(hint()).toBeNull();
  });
});

// ── the Dimension TOOL's own chip (SketchController, not the service) ──────────

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
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
  };
}

describe("the Dimension tool's chip refuses at error severity", () => {
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    // Drain whatever the previous test left queued: the sketch mutation chain is
    // a module singleton, and a late upsert write would land in THIS test's
    // session (it did — the pick was hitting a leftover circle).
    await flushSketchMutations();
    resetStores();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    const session: SketchSession = {
      sketchId: "sk-refuse",
      plane: PLANE,
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [100, 0] }],
      constraints: [],
      dof: 4,
      status: "UnderConstrained",
    };
    const client = {
      ...conflictingClient([]),
      enterSketch: vi.fn(() => Promise.resolve(session)),
      cancelSketch: vi.fn(() => Promise.resolve()),
      deleteSketch: vi.fn(() => Promise.resolve()),
    } as unknown as CadClient;
    controller = new SketchController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client,
      container,
    });
    toolStore.getState().setMode("sketch", "sk-refuse");
    await flush();
    toolStore.getState().setTool("dimension");
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  it("a rejected dimension is an error, not a status note", async () => {
    // Pick the line (screenToPlane is 1:1 in this harness), then commit a value.
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 50, clientY: 0, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: 50, clientY: 0, button: 0, buttons: 0, bubbles: true }),
    );
    await flush();
    expect(toolChipStore.getState().kind).toBe("dimension");

    toolChipStore.getState().onValue!(70);
    await flush();

    expect(hint()?.message).toMatch(/Dimension removed/);
    expect(hint()?.severity).toBe("error");
  });
});
