/*
 * ModelToolController — FILLET-CHAMFER-UNIFY: the drag DIRECTION types the edge
 * op, and a type change swaps the kernel preview session (jsdom).
 *
 * Three things are pinned here and nowhere else:
 *
 *  1. TIER HONESTY. `auto` is armed ONLY when the outward direction came from the
 *     bisector tier. The bbox proxy is convex-only — on a pocket edge it points
 *     INTO material — so an arm that falls back to it (or has no mesh at all)
 *     must seed `auto:false` and let the chip segments own the type.
 *  2. THE FLIP IS A SESSION SWAP. `beginPreview` freezes `opType`, so a re-type
 *     has to `endPreview(id,false)` the old session and open a new one under the
 *     new op. A params patch would author a Chamfer through a Fillet's builder.
 *  3. NO STALE SESSION SURVIVES A RACE. Two flips inside one in-flight
 *     `beginPreview` must leave exactly ONE live session, carrying the FINAL op —
 *     which is what the `armGen` bump before the close+reopen buys.
 *
 * The projection here is a deliberate fake (`x = wx`, `y = -wz`), so the screen
 * direction of "away from the body" is exact and the drag deltas below are
 * arithmetic, not calibration.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, PreviewDraft, PreviewResult, PreviewSession } from "@/ipc/types";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import * as meshRegistry from "@/viewport/mesh/meshRegistry";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

/**
 * `e:0` of the mock box — the bottom-front edge, between faces (0,-1,0) and
 * (0,0,-1). Its bisector is EXACTLY (0,-√½,-√½) at midpoint (0,-30,-15)
 * (pinned by edgeDirection.test.ts), which is what makes the screen axis below
 * computable by hand.
 */
const EDGE: EntityRef = {
  kind: "edge",
  id: "body1#e:0",
  bodyId: "body1",
  topoKey: "e:0",
  elementId: "el-edge-0",
  anchor: { worldPoint: [0, -30, -15] },
};

/** World → screen: x unchanged, world −z downscreen. Orthographic, no w. */
const project = (w: readonly number[]): { x: number; y: number } => ({ x: w[0], y: -w[2] });

function makeEngineMock(opts: { projectPoint?: boolean } = {}) {
  const engine: Record<string, unknown> = {
    setOrbitSuppressed: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    probePick: vi.fn(() => null),
  };
  // Absent on purpose in the fallback case: real engine mocks predate
  // `projectPoint`, and the controller must degrade to the screen convention
  // rather than throw.
  if (opts.projectPoint !== false) engine.projectPoint = vi.fn(project);
  return engine;
}

/** A client whose `beginPreview` can be held open (the race case resolves them by hand). */
function makeClientMock(opts: { deferPreview?: boolean } = {}) {
  let seq = 0;
  const pending: Array<() => void> = [];
  const client = {
    onPreviewResult: vi.fn((_cb: (r: PreviewResult) => void) => () => {}),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    beginPreview: vi.fn((_d: PreviewDraft) => {
      const session: PreviewSession = { sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` };
      if (!opts.deferPreview) return Promise.resolve(session);
      return new Promise<PreviewSession>((resolve) => pending.push(() => resolve(session)));
    }),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
  return { client, releaseAll: () => pending.splice(0).forEach((r) => r()) };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

interface DebugSurface {
  filletPhase?: string;
  edgeOpKind?: string;
  edgeOpAuto?: boolean;
  edgeOpAxisSource?: string;
  previewSessionCount?: number;
  sessionId?: string | null;
  previewOwner?: string | null;
}
const debugState = (): DebugSurface =>
  (window as unknown as { __extrudePreview?: DebugSurface }).__extrudePreview ?? {};

describe("ModelToolController edge-op drag direction", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController | undefined;
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;

  function build(opts: { projectPoint?: boolean; deferPreview?: boolean } = {}): void {
    engineMock = makeEngineMock({ projectPoint: opts.projectPoint });
    clientMock = makeClientMock({ deferPreview: opts.deferPreview });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock.client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true, // the tier + auto assertions read the debug surface
    });
  }

  /** Publish the mock box as `body1` so the tier math has real MESH1 to read. */
  function registerBox(): void {
    const view = parseMeshPayload(makeBoxMesh());
    meshRegistry.swap("body1", meshRegistry.buildBodyObjects(view, "body1", 1));
  }

  async function armFillet(): Promise<void> {
    selectionStore.getState().set([EDGE]);
    toolStore.getState().setTool("fillet");
    await flush();
  }

  /** Press at (100,100) and move by (dx,dy) — the drag the controller measures. */
  function drag(dx: number, dy: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 100, clientY: 100, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointermove", {
        clientX: 100 + dx,
        clientY: 100 + dy,
        button: 0,
        buttons: 1,
        bubbles: true,
      }),
    );
  }

  const drafts = (): PreviewDraft[] =>
    clientMock.client.beginPreview.mock.calls.map((c) => c[0] as PreviewDraft);

  beforeEach(() => {
    resetStores();
    meshRegistry.disposeAll();
    meshRegistry.__resetRegistryForTests();
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    controller = undefined;
    container.remove();
    meshRegistry.disposeAll();
    meshRegistry.__resetRegistryForTests();
  });

  // ── tier 1: the bisector arm is the only one that may re-type ────────────────

  it("arms auto on the BISECTOR tier when the picked edge's mesh is present", async () => {
    build();
    registerBox();
    await armFillet();

    expect(debugState().edgeOpAxisSource).toBe("bisector");
    expect(debugState().edgeOpAuto).toBe(true);
    expect(debugState().edgeOpKind).toBe("Fillet");
  });

  it("a drag INTO the body re-types to Chamfer and SWAPS the preview session", async () => {
    build();
    registerBox();
    await armFillet();
    expect(drafts()[0].opType).toBe("Fillet");

    // Outward (0,−√½,−√½) projects to screen +y, so screen −y is INTO the body.
    // Seed radius 2 ⇒ signed = 2 − 10 = −8, past the flip band.
    drag(0, -10);
    await flush();

    expect(debugState().edgeOpKind).toBe("Chamfer");
    expect(toolChipStore.getState().edgeOp).toBe("Chamfer"); // the chip follows
    expect(toolChipStore.getState().value).toBe(8); // MAGNITUDE, not −8

    // The old session was released and a NEW one opened under the new opType —
    // `beginPreview` freezes opType, so a params patch would be a lie.
    expect(clientMock.client.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(2);
    expect(drafts()[1].opType).toBe("Chamfer");
    expect(drafts()[1].params.mode).toBe("Chamfer");
    expect(drafts()[1].params.radius).toBe(8);
    expect(debugState().previewSessionCount).toBe(1);
    expect(debugState().sessionId).toBe("pv-2");
  });

  it("a drag AWAY keeps the Fillet and its ONE session", async () => {
    build();
    registerBox();
    await armFillet();

    drag(0, 40); // screen +y = away from the body here
    await flush();

    expect(debugState().edgeOpKind).toBe("Fillet");
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.client.endPreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().value).toBe(42); // 2 + 40 px × 1 world/px
  });

  // ── tier 3 fallback: today's behaviour, unchanged ────────────────────────────

  it("no mesh + no projectPoint: auto is OFF and the up-grows mapping is preserved", async () => {
    build({ projectPoint: false }); // an engine mock from before the seam existed
    await armFillet(); // no registerBox → nothing to reconstruct a direction from

    expect(debugState().edgeOpAxisSource).toBe("screen");
    expect(debugState().edgeOpAuto).toBe(false);

    // SCREEN_UP_AXIS reproduces `radiusFromDrag` exactly: up (−y) grows.
    drag(0, -40);
    await flush();
    expect(toolChipStore.getState().value).toBe(42);
    expect(debugState().edgeOpKind).toBe("Fillet");

    // …and a drag the other way cannot re-type an auto-less arm: it bottoms out
    // on the Fillet half-line and stays there.
    drag(0, 100);
    await flush();
    expect(debugState().edgeOpKind).toBe("Fillet");
    expect(toolChipStore.getState().value).toBe(0.1);
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(1); // no session churn
  });

  it("a chip segment locks the type on a bisector arm (auto dies, session swaps)", async () => {
    build();
    registerBox();
    await armFillet();

    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();

    expect(debugState().edgeOpKind).toBe("Chamfer");
    expect(debugState().edgeOpAuto).toBe(false);
    expect(toolChipStore.getState().value).toBe(1); // pristine arm reseeds to the chamfer default
    expect(drafts()[1].opType).toBe("Chamfer");

    // The direction can no longer speak: a drag AWAY sizes the chamfer, it does
    // not restore the fillet.
    drag(0, 40);
    await flush();
    expect(debugState().edgeOpKind).toBe("Chamfer");
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(2);
  });

  // ── the race: two flips inside one in-flight beginPreview ────────────────────

  it("two rapid flips over an unresolved beginPreview leave ONE live session, final op", async () => {
    build({ deferPreview: true });
    registerBox();
    await armFillet(); // beginPreview #1 is HELD open

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // flip 1 → #2 (also held)
    toolChipStore.getState().onEdgeOp?.("Fillet"); // flip 2 → #3 (also held)
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(3);
    expect(clientMock.client.endPreview).not.toHaveBeenCalled(); // nothing landed yet

    clientMock.releaseAll();
    await flush();
    await flush();

    // Every superseded session was released; none was installed under the new op.
    expect(clientMock.client.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(clientMock.client.endPreview).toHaveBeenCalledWith("pv-2", false);
    expect(clientMock.client.endPreview).toHaveBeenCalledTimes(2); // 3 begun, 1 kept
    expect(debugState().previewSessionCount).toBe(1);
    expect(debugState().sessionId).toBe("pv-3");
    expect(drafts()[2].opType).toBe("Fillet");
    expect(debugState().edgeOpKind).toBe("Fillet");
  });

  // ── re-edit while the SAME tool is already armed (review F5) ────────────────
  //
  // `setTool("fillet")` inside `editEdgeOpFeature` is a NO-OP whenever the edge-op
  // tool is ALREADY "fillet" (same value in, same value out — the controller's
  // subscriber only re-fires `onToolChange` on an actual change), so a re-edit
  // opened while a PRIOR arm still has a session in flight is never swept by the
  // ordinary cancel sweep. `editEdgeOpFeature` fences this itself, right after
  // `setTool` (see the comment there for the double-apply rationale).

  it("re-edit while fillet is ALREADY armed fences the stale arm's in-flight session", async () => {
    build({ deferPreview: true });
    registerBox();
    await armFillet(); // tool -> "fillet"; beginPreview #1 (pv-1) HELD, not yet installed

    documentStore.setState({
      features: [
        { id: "feat-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
      ],
    });
    await controller!.editEdgeOpFeature("feat-fi", "Fillet");
    await flush();

    // No re-arm was triggered — `setTool("fillet")` above was a no-op, exactly as
    // it would have been pre-unification whenever fillet was already the armed
    // tool (review F5 calls this the pre-existing half of the leak).
    expect(clientMock.client.beginPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.client.endPreview).not.toHaveBeenCalled(); // pv-1 hasn't resolved yet

    clientMock.releaseAll(); // pv-1 resolves NOW — well after the re-edit's own state was set
    await flush();

    // The fence (`invalidateArm` bump immediately followed by `cancelFillet`, run
    // INSIDE `editEdgeOpFeature` right after `setTool`) makes pv-1's captured `gen`
    // stale, so its continuation tears itself down instead of installing under the
    // re-edit — without it this is a leaked session forever, since a re-edit never
    // closes one of its own.
    expect(clientMock.client.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(debugState().previewSessionCount).toBe(0);
    expect(debugState().sessionId).toBeNull();
    expect(debugState().previewOwner ?? null).toBeNull();

    // The re-edit's own state survived intact (nothing above clobbered it, since
    // every field the fence clears is re-set by this function's own code right
    // after, in program order): committing routes through the scalar-patch path —
    // proof `filletEditFeatureId` stuck — with no edges required, proof the
    // fresh-commit edge-count bail never runs (a leaked `filletEdges` would have
    // routed here differently or not at all).
    // (W3: the edge-op re-edit cluster commits from ✓/Enter, not from the input —
    // a pure Fillet⇄Chamfer flip changes no number, so `onValue` only sizes it.)
    toolChipStore.getState().onValue?.(4);
    toolChipStore.getState().onConfirm?.();
    await flush();
    expect(clientMock.client.applyEditCommand).toHaveBeenCalledWith(
      expect.objectContaining({ cmd: "updateOperationParams", record: "feat-fi" }),
    );
  });
});
