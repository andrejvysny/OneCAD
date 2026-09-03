/*
 * Revolve re-edit honesty + the commit-boundary profile-record guarantee
 * (jsdom, REVOLVE-REGION-PARITY). Before this, `editRevolveFeature` guessed the
 * sketch (`lastArmedSketch ?? Object.keys(sketches)[0]`), the arm took
 * `regions[0]`, the re-edit rendered `revolveAxisCandidates[0]` regardless of the
 * stored axis, and arming called the SESSION-OPENING `finishSketch` (whose record
 * mint was the only thing making a revolve commit resolvable at all).
 *
 * These specs pin the fixed contract:
 *   (a) the re-edit reads the STORED profile.sketchId, not lastArmedSketch/first key,
 *   (b) it binds the STORED regionId, never regions[0],
 *   (c) a stale stored regionId refuses LOUDLY naming the available ids,
 *   (d) finishSketch is NEVER called while arming and EXACTLY ONCE at confirm
 *       (the postmortem's "profile sketch not found in plan" tripwire),
 *   (e) the stored axis lineId is restored; a vanished line refuses (no substitute),
 *   (f) a superseded re-edit never resurrects (armGen),
 *   (g) a geometryToken bump under an armed revolve cancels it with a hint.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/**
 * Three disjoint squares with DISTINCT extents, so the ring handed to
 * `showRevolvePreview` identifies WHICH region was bound (the only observable the
 * controller exposes for the private binding).
 */
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [100, 100, 140, 100, 140, 140, 100, 140],
    indices: [0, 1, 2, 0, 2, 3],
  },
};
const R2: SketchRegion = {
  regionId: "r2",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [200, 200, 260, 200, 260, 260, 200, 260],
    indices: [0, 1, 2, 0, 2, 3],
  },
};

// Two axis candidates far to the left of every region (neither splits a profile).
const AXIS_A = {
  id: "line-a",
  type: "Line" as const,
  p0: [-50, -50] as [number, number],
  p1: [-50, 400] as [number, number],
};
const AXIS_B = {
  id: "line-b",
  type: "Line" as const,
  p0: [-80, -50] as [number, number],
  p1: [-80, 400] as [number, number],
};

const ok = (bodyId: string): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId, meshKey: `${bodyId}#0` }],
  removedBodies: [],
});

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
  };
}

interface ClientOpts {
  /** Regions per sketch id — the pure-read arm source. */
  regions: Record<string, SketchRegion[]>;
  entities?: SketchSession["entities"];
  storedParams?: Record<string, unknown> | (() => Promise<Record<string, unknown>>);
}

function makeClientMock(opts: ClientOpts) {
  let apply = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    finishSketch: vi.fn(
      (sketchId: string): Promise<FinishSketchResult> =>
        Promise.resolve({ regions: opts.regions[sketchId] ?? [] }),
    ),
    getSketchRegions: vi.fn(
      (sketchId: string): Promise<FinishSketchResult> =>
        Promise.resolve({ regions: opts.regions[sketchId] ?? [] }),
    ),
    getSketch: vi.fn(
      (sketchId: string): Promise<SketchSession> =>
        Promise.resolve({
          sketchId,
          plane: PLANE,
          entities: opts.entities ?? [],
          constraints: [],
          dof: 0,
          status: "FullyConstrained",
        }),
    ),
    beginPreview: vi.fn(() => Promise.resolve({ sessionId: "pv-1", previewBodyId: "pb-1" })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(ok("b0"))),
    applyOperation: vi.fn(() => Promise.resolve(ok(`rv${++apply}`))),
    applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
    getOperationParams: vi.fn(
      (): Promise<Record<string, unknown>> =>
        typeof opts.storedParams === "function"
          ? opts.storedParams()
          : Promise.resolve(opts.storedParams ?? {}),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** The ring `showRevolvePreview` was last handed (identifies the bound region). */
function previewRing(engine: ReturnType<typeof makeEngineMock>): [number, number][] | null {
  const calls = engine.showRevolvePreview.mock.calls;
  if (calls.length === 0) return null;
  return (calls[calls.length - 1] as unknown[])[1] as [number, number][];
}

function ringMaxU(ring: [number, number][] | null): number {
  return ring ? Math.max(...ring.map(([u]) => u)) : Number.NaN;
}

describe("ModelToolController revolve re-edit (REVOLVE-REGION-PARITY)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(opts: ClientOpts): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(opts);
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  /** A two-sketch document; "sk-1" is the WRONG guess, "sk-2" carries the profile. */
  function twoSketchDoc(): void {
    documentStore.setState({
      sketches: {
        "sk-1": { id: "sk-1", name: "Sketch 1", visible: true, dof: 0, status: "ok", geometryToken: "t1" },
        "sk-2": { id: "sk-2", name: "Sketch 2", visible: true, dof: 0, status: "ok", geometryToken: "t2" },
      },
      features: [{ id: "rev-1", kind: "revolve", label: "Revolve", valueText: "90°", status: "ok" }],
    });
  }

  beforeEach(() => {
    resetStores();
    // No preview results are delivered here (onPreviewResult is a bare stub), so
    // collapse the fresh-commit exact-preview barrier to one macrotask.
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([]); // no selection → setTool('revolve') can't fresh-arm
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  // ── (a) the stored sketch wins over lastArmedSketch / the first document key ──

  it("re-edit reads the STORED profile.sketchId, not lastArmedSketch or the first sketch key", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-1": [R0], "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    // Prime lastArmedSketch with the WRONG sketch: a fresh revolve arm on sk-1.
    selectionStore.getState().set([{ kind: "sketch", id: "sk-1" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk-1");
    clientMock.getSketchRegions.mockClear();
    selectionStore.getState().set([]);

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    // "sk-1" is BOTH lastArmedSketch and the first key of documentStore.sketches —
    // the old guess would have re-armed it. The stored id must win.
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk-2");
    expect(clientMock.getSketchRegions).not.toHaveBeenCalledWith("sk-1");
    expect(engineMock.showRevolvePreview).toHaveBeenCalled();
  });

  // ── (b) the stored regionId wins over regions[0] ──────────────────────────────

  it("binds the EXACT stored regionId out of a 3-region sketch (never regions[0])", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R0, R1, R2] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r2" }, // the THIRD region
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    // r2 spans u ∈ [200,260]; r0 (the old regions[0] guess) spans u ∈ [0,20].
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(ringMaxU(previewRing(engineMock))).toBeCloseTo(260, 6);
    expect(engineMock.showRegionPick).not.toHaveBeenCalled(); // a re-edit never re-picks
  });

  // ── (b') the anchor array stays length-matched to revolveRegionIds on re-edit ──

  it("re-edit's revolveRegionAnchors array is length-matched to revolveRegionIds, anchor at index 0", async () => {
    twoSketchDoc();
    const storedAnchor: [number, number] = [1.5, -2.5];
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1", regionAnchor: storedAnchor },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    const priv = controller as unknown as {
      revolveRegionIds: string[];
      revolveRegionAnchors: Array<[number, number] | undefined>;
    };
    expect(priv.revolveRegionAnchors).toHaveLength(priv.revolveRegionIds.length);
    expect(priv.revolveRegionAnchors[0]).toEqual(storedAnchor);
  });

  // ── (c) a stale stored regionId refuses loudly ────────────────────────────────

  it("refuses a stale stored regionId with the available ids — nothing armed, nothing committed", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R0, R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r-gone" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("r-gone");
    expect(hint?.message).toContain("r0, r1"); // the available ids, verbatim
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
  });

  // ── (d) THE tripwire: finishSketch never at arm, exactly once at confirm ──────

  it("never calls finishSketch while arming, and calls it EXACTLY once at confirm (record guarantee)", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    // Arm is a PURE read: regions came from getSketchRegions, no session-opening
    // finish (which would also mint the timeline record as a side effect).
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk-2");
    expect(clientMock.finishSketch).not.toHaveBeenCalled();

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    // The commit boundary guarantees the `Sketch` record — without it the regen
    // planner fails every revolve with "profile sketch not found in plan".
    expect(clientMock.finishSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.finishSketch).toHaveBeenCalledWith("sk-2");
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1); // re-edit deep-merge
  });

  it("a FRESH revolve also guarantees the record exactly once at confirm", async () => {
    documentStore.setState({
      sketches: { "sk-2": { id: "sk-2", name: "Sketch 2", visible: true, dof: 0, status: "ok", geometryToken: "t2" } },
    });
    build({ regions: { "sk-2": [R1] }, entities: [AXIS_A] });
    selectionStore.getState().set([{ kind: "sketch", id: "sk-2" }]);

    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    expect(clientMock.finishSketch).not.toHaveBeenCalled(); // axis pick, pure read only

    // Click the single axis candidate (identity screen→plane mapping) → armed.
    container.dispatchEvent(
      new MouseEvent("pointerdown", { button: 0, clientX: -50, clientY: 100, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { button: 0, clientX: -50, clientY: 100, bubbles: true }),
    );
    await flush();
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(clientMock.finishSketch).not.toHaveBeenCalled();
    // Axis chosen ⇒ the op is well-formed ⇒ the kernel-preview session opens (and
    // arming STILL authors no timeline record — beginPreview is not finishSketch).
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    await flush();
    await flush();

    expect(clientMock.finishSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.finishSketch).toHaveBeenCalledWith("sk-2");
    // The commit materializes the PREVIEWED candidate, not a second ad-hoc op.
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
  });

  it("a failed record guarantee returns to armed with a named hint and commits nothing", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });
    clientMock.finishSketch.mockRejectedValue(new Error("worker gone"));

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("cannot record profile sketch");
    expect(hint?.message).toContain("worker gone");
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
    expect(toolStore.getState().phase).toBe("armed"); // retryable
  });

  // ── (e) the stored axis is restored; a vanished line refuses ──────────────────

  it("restores the STORED axis lineId instead of the first candidate", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A, AXIS_B], // candidates[0] is line-a
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-b" }, // the SECOND
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    const axis = (engineMock.showRevolvePreview.mock.calls[0] as unknown[])[2] as {
      a: [number, number];
      b: [number, number];
    };
    expect(axis.a[0]).toBeCloseTo(-80, 6); // line-b, not line-a (-50)

    // The commit re-targets by id — the deep-merge must carry the STORED axis through
    // untouched (angle-only patch), never a substituted one.
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    const cmd = (clientMock.applyEditCommand.mock.calls as unknown[][])[0][0] as {
      op: { params: { axis?: { lineId?: string }; angleDeg?: { value?: number } } };
    };
    expect(cmd.op.params.axis?.lineId).toBe("line-b");
    expect(cmd.op.params.angleDeg?.value).toBe(90);
  });

  it("refuses when the stored axis line no longer exists (never substitutes another line)", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A], // line-b is GONE
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-b" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("line-b");
    expect(hint?.message).toContain("no longer exists");
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("select");
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
  });

  it("refuses a feature whose stored profile is missing or needs repair", async () => {
    twoSketchDoc();
    build({ regions: { "sk-2": [R1] }, entities: [AXIS_A], storedParams: { angleDeg: { value: 90 } } });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toBe("Revolve profile is missing or needs repair");
    expect(clientMock.getSketchRegions).not.toHaveBeenCalled();
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
  });

  // ── (f) a superseded re-edit never resurrects ─────────────────────────────────

  it("a tool switch during getOperationParams drops the re-edit (armGen)", async () => {
    twoSketchDoc();
    let resolveParams!: (v: Record<string, unknown>) => void;
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: () => new Promise((r) => (resolveParams = r)),
    });

    controller.editRevolveFeature("rev-1");
    await flush(); // paused at the getOperationParams await (BEFORE setTool("revolve"))
    // Supersede with a real tool switch. NOTE: setTool("select") would be a no-op
    // here (the tool is still "select" — the re-edit sets "revolve" only after the
    // params land), so it would not bump armGen at all.
    toolStore.getState().setTool("fillet"); // onToolChange → invalidateArm()
    await flush();
    resolveParams({
      profile: { sketchId: "sk-2", regionId: "r1" },
      angleDeg: { value: 90 },
      axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
    });
    await flush();
    await flush();

    expect(clientMock.getSketchRegions).not.toHaveBeenCalled(); // never even read
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("fillet"); // never flipped to revolve
  });

  // ── (g) a geometryToken bump under an armed revolve cancels it ────────────────

  it("a sketch geometryToken bump cancels an armed revolve with an info hint", async () => {
    twoSketchDoc();
    build({
      regions: { "sk-2": [R1] },
      entities: [AXIS_A],
      storedParams: {
        profile: { sketchId: "sk-2", regionId: "r1" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk-2", lineId: "line-a" },
      },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(toolStore.getState().modelTool).toBe("revolve");

    // The sketch is edited underneath the armed revolve.
    documentStore.setState({
      sketches: {
        ...documentStore.getState().sketches,
        "sk-2": { id: "sk-2", name: "Sketch 2", visible: true, dof: 0, status: "ok", geometryToken: "t2-edited" },
      },
    });
    await flush();

    expect(engineMock.hideRevolvePreview).toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("select");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("info");
    expect(hint?.message).toBe("Sketch changed — revolve preview cancelled");

    // Nothing can be committed off the dropped arm.
    toolChipStore.getState().onConfirm?.();
    await flush();
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
  });
});
