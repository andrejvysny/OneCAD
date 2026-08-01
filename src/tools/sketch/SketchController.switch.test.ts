/*
 * SketchController sketch→sketch switch (W0.2, trust-class).
 *
 * Activating another sketch (tree row / chrome) while ALREADY in sketch mode only
 * moves `viewportStore.activeSketchId` — the mode never changes, so the controller's
 * mode subscription never fires. Before the fix that made the chrome show sketch B
 * while every commit silently kept landing in sketch A.
 *
 * The switch closes A on the wire BEFORE opening B (the backend holds ONE sketch-
 * session slot) with the same cancel→finish pair `exit()` uses, and must stay clear
 * of the paths that legitimately move activeSketchId themselves (setMode writes it
 * after enter() started; openSession echoes it) — hence the self-switch pins below.
 *
 * Engine + client are faked (no WebGL / no backend); screenToPlane is 1:1 and every
 * frontend snap type is disabled so click coords ARE plane coords.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { CadClient } from "@/ipc/client";
import type {
  EnterSketchTarget,
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchSession,
  SketchUpsertResult,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "./sketchService";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

function makeEngineMock(hit: () => PickablePlane | null) {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => hit()),
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
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    getCameraDistance: vi.fn(() => 100),
  };
}

/** Every wire call the switch touches, in order, so the close/open sequence is pinned. */
type Calls = string[];

function makeClientMock(calls: Calls) {
  return {
    enterSketch: vi.fn((target: EnterSketchTarget): Promise<SketchSession> => {
      const sketchId = typeof target === "string" ? target : "sketchNew";
      calls.push(`enterSketch:${sketchId}`);
      return Promise.resolve({
        sketchId,
        plane: PLANE,
        entities: [],
        constraints: [],
        dof: 0,
        status: "UnderConstrained",
      });
    }),
    cancelSketch: vi.fn((id: string) => {
      calls.push(`cancelSketch:${id}`);
      return Promise.resolve();
    }),
    finishSketch: vi.fn((id: string) => {
      calls.push(`finishSketch:${id}`);
      return Promise.resolve({ regions: [] });
    }),
    deleteSketch: vi.fn(() => Promise.resolve()),
    sketchUpsert: vi.fn(
      (
        sketchId: string,
        _entities: SketchEntity[],
        _constraints: SketchConstraint[],
      ): Promise<SketchUpsertResult> => {
        calls.push(`sketchUpsert:${sketchId}`);
        return Promise.resolve({
          sketchId,
          sketchRevision: 1,
          dof: 0,
          status: "UnderConstrained",
          conflicting: [],
          solvedPositions: {},
        });
      },
    ),
  };
}

/** A tree-registered sketch with a backend-shaped geometry token. */
function sketchMeta(id: string): SketchMeta {
  return { id, name: id, visible: true, dof: 0, status: "under", geometryToken: `wire:${id}:v1` };
}

const tick = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
/** Drain the mutation queue + every await hop the switch chains through. */
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await flushSketchMutations();
    await tick();
  }
}

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

describe("SketchController sketch→sketch switch", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;
  let calls: Calls;
  let hitKind: PickablePlane | null;

  beforeEach(() => {
    resetStores();
    disableSnapping();
    calls = [];
    hitKind = null;
    engineMock = makeEngineMock(() => hitKind);
    clientMock = makeClientMock(calls);
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

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  /** One committed line segment (2 clicks on an open chain). */
  async function drawSegment(y: number): Promise<void> {
    click(0, y);
    click(50, y);
    await flushSketchMutations();
  }

  const upserts = (): string[] => calls.filter((c) => c.startsWith("sketchUpsert:"));

  it("[a] a switch retargets the commits: geometry drawn after it lands in B, not A", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    await drawSegment(0);
    expect(upserts()).toEqual(["sketchUpsert:A"]);

    toolStore.getState().setMode("sketch", "B"); // tree activate while IN sketch mode
    await settle();
    expect(sketchStore.getState().session?.sketchId).toBe("B");

    await drawSegment(10);
    expect(upserts()).toEqual(["sketchUpsert:A", "sketchUpsert:B"]);
    // B started empty: the second segment is the ONLY entity in B's session.
    expect(sketchStore.getState().session?.entities).toHaveLength(1);
  });

  it("[b] closes A (cancel THEN finish) BEFORE opening B", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    calls.length = 0;

    toolStore.getState().setMode("sketch", "B");
    await settle();

    expect(calls).toEqual(["cancelSketch:A", "finishSketch:A", "enterSketch:B"]);
  });

  it("[c] rapid A→B→C is latest-wins: lands on C, and B is never opened", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    calls.length = 0;

    toolStore.getState().setMode("sketch", "B");
    toolStore.getState().setMode("sketch", "C"); // arrives while the B switch is in flight
    await settle();

    expect(sketchStore.getState().session?.sketchId).toBe("C");
    expect(calls.filter((c) => c.startsWith("enterSketch:"))).toEqual(["enterSketch:C"]);
    expect(calls.filter((c) => c.startsWith("cancelSketch:"))).toEqual(["cancelSketch:A"]);
    expect(calls.filter((c) => c.startsWith("finishSketch:"))).toEqual(["finishSketch:A"]);
  });

  it("[d] a switch during an in-flight ENTER defers and runs after it", async () => {
    let resolveEnter!: (s: SketchSession) => void;
    clientMock.enterSketch.mockImplementationOnce(() => {
      calls.push("enterSketch:A");
      return new Promise<SketchSession>((r) => {
        resolveEnter = (s) => r(s);
      });
    });

    toolStore.getState().setMode("sketch", "A");
    await tick(); // enter() is now parked on the deferred enterSketch
    toolStore.getState().setMode("sketch", "B"); // retarget mid-enter

    resolveEnter({
      sketchId: "A",
      plane: PLANE,
      entities: [],
      constraints: [],
      dof: 0,
      status: "UnderConstrained",
    });
    await settle();

    // A opened first, THEN was closed for B — never skipped, never interleaved.
    expect(calls).toEqual(["enterSketch:A", "cancelSketch:A", "finishSketch:A", "enterSketch:B"]);
    expect(sketchStore.getState().session?.sketchId).toBe("B");
  });

  it("[e] the plane-pick create path is NOT a switch: one enterSketch, zero cancel/finish", async () => {
    toolStore.getState().setMode("sketch"); // bare intent → plane picker
    await settle();

    hitKind = "XZ";
    click(100, 100);
    await settle();

    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.cancelSketch).not.toHaveBeenCalled();
    expect(clientMock.finishSketch).not.toHaveBeenCalled();
    expect(sketchStore.getState().session?.sketchId).toBe("sketchNew");
  });

  it("[e2] a normal existing-sketch entry is NOT a switch either", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();

    expect(calls).toEqual(["enterSketch:A"]);
    expect(sketchStore.getState().session?.sketchId).toBe("A");
  });

  it("[f] a mode exit during an in-flight switch bails cleanly (B never opens)", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    calls.length = 0;

    let resolveFinish!: () => void;
    clientMock.finishSketch.mockImplementationOnce((id: string) => {
      calls.push(`finishSketch:${id}`);
      return new Promise<{ regions: [] }>((r) => {
        resolveFinish = () => r({ regions: [] });
      });
    });

    toolStore.getState().setMode("sketch", "B"); // switch starts, parks on finish(A)
    await tick();
    toolStore.getState().setMode("model"); // user leaves sketch mode mid-switch
    await tick();
    resolveFinish();
    await settle();

    expect(calls).toEqual(["cancelSketch:A", "finishSketch:A"]); // B never opened
    expect(toolStore.getState().mode).toBe("model");
    expect(sketchStore.getState().session).toBeNull();
  });

  it("[g] exit after a switch restores the projection saved at the ORIGINAL entry", async () => {
    expect(viewportStore.getState().projection).toBe("persp");

    toolStore.getState().setMode("sketch", "A");
    await settle();
    expect(viewportStore.getState().projection).toBe("ortho");

    toolStore.getState().setMode("sketch", "B");
    await settle();
    expect(viewportStore.getState().projection).toBe("ortho"); // sketch→sketch stays ortho

    toolStore.getState().setMode("model");
    await settle();
    expect(viewportStore.getState().projection).toBe("persp"); // NOT the switch-time ortho
  });

  it("[i] a rejected switch-open falls back to model mode instead of stranding the chrome", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    clientMock.enterSketch.mockImplementationOnce(() => Promise.reject(new Error("worker gone")));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    toolStore.getState().setMode("sketch", "B");
    await settle();

    expect(toolStore.getState().mode).toBe("model");
    expect(sketchStore.getState().session).toBeNull();
    expect(viewportStore.getState().statusHint?.message).toMatch(/Enter sketch failed/);
    errorSpy.mockRestore();
  });

  // ── the closed sketch's static layer must be told its geometry moved ─────────
  //
  // A's always-visible SketchStaticLayer copy was fetched at ENTRY. The switch
  // commits A's edits on the wire and then re-shows that stale copy — the real lane
  // self-heals because a backend projection bumps A's geometryToken, but the mock
  // lane publishes nothing, so the switch stamps a LOCAL token itself.

  it("[j] the switch bumps the CLOSED sketch's geometryToken so its static layer refetches", async () => {
    documentStore.setState({ sketches: { A: sketchMeta("A"), B: sketchMeta("B") } });
    toolStore.getState().setMode("sketch", "A");
    await settle();
    const beforeA = documentStore.getState().sketches.A.geometryToken;
    const beforeB = documentStore.getState().sketches.B.geometryToken;

    toolStore.getState().setMode("sketch", "B");
    await settle();

    expect(documentStore.getState().sketches.A.geometryToken).not.toBe(beforeA);
    // Only the CLOSED sketch is stamped — B's copy is the one being edited.
    expect(documentStore.getState().sketches.B.geometryToken).toBe(beforeB);
  });

  it("[j2] a SUPERSEDED switch still bumps A — A really closed", async () => {
    documentStore.setState({
      sketches: { A: sketchMeta("A"), B: sketchMeta("B"), C: sketchMeta("C") },
    });
    toolStore.getState().setMode("sketch", "A");
    await settle();
    const beforeA = documentStore.getState().sketches.A.geometryToken;

    toolStore.getState().setMode("sketch", "B");
    toolStore.getState().setMode("sketch", "C"); // supersedes the B switch mid-close
    await settle();

    expect(sketchStore.getState().session?.sketchId).toBe("C");
    expect(documentStore.getState().sketches.A.geometryToken).not.toBe(beforeA);
  });

  it("[j3] a NON-switch entry never bumps anything", async () => {
    documentStore.setState({ sketches: { A: sketchMeta("A") } });
    const before = documentStore.getState().sketches.A.geometryToken;

    toolStore.getState().setMode("sketch", "A");
    await settle();

    expect(documentStore.getState().sketches.A.geometryToken).toBe(before);
  });

  it("[h] re-activating the SAME sketch is a no-op (no close/reopen churn)", async () => {
    toolStore.getState().setMode("sketch", "A");
    await settle();
    calls.length = 0;

    toolStore.getState().setMode("sketch", "A");
    await settle();

    expect(calls).toEqual([]);
    expect(sketchStore.getState().session?.sketchId).toBe("A");
  });
});
