/*
 * ModelToolController — the CHAMFER REFERENCE FACE (SCHEMA §7.3 `referenceFaces`,
 * kernel-hardening WP-F) end to end in the frontend.
 *
 * Before WP-F an asymmetric chamfer measured `radius` on whichever adjacent face
 * had the smaller SNAPSHOT ordinal, so an upstream edit that reordered the face
 * map silently mirrored the two legs. The face is now PERSISTED, and this suite
 * pins the four rules the frontend owns:
 *
 *  1. DEFAULT MOVES NOTHING. The pair binds `adjacentFaces[0]` of the contour's
 *     first listed edge — the legacy smaller-ordinal face — so authoring a typed
 *     chamfer reproduces yesterday's geometry.
 *  2. FLIP. [Flip reference] swaps every contour to `adjacentFaces[1]`, and the
 *     control is offered ONLY when there is a second face to flip to.
 *  3. SLOT ORDER. The face refs ride the op's `inputs[]` AFTER the N edge refs, so
 *     pair `i` is addressable as `<opId>.input<N+i>` (SCHEMA §7.3, §9 repair).
 *  4. NEVER FABRICATE. A body whose adjacency the backend cannot report blocks the
 *     ✓ with the reason instead of committing a chamfer that falls back to the
 *     ordinal rule — the exact defect this work package removes.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  ElementInfo,
  PreviewDraft,
  PreviewResult,
  PromotePick,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { __resetLogForTests } from "@/debug/log";

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

/** One picked edge of the seed box. */
const EDGES: EntityRef[] = [
  {
    kind: "edge",
    id: "body1#e:0",
    bodyId: "body1",
    topoKey: "e:0",
    elementId: "el_e0",
    anchor: { worldPoint: [1, 2, 3] },
  },
];

/** The edge's two adjacent faces, face-ordinal ascending (SCHEMA §7.6). */
const ADJACENT = ["f:3", "f:5"];

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setRegionSelected: vi.fn(),
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
    probePick: vi.fn(() => null),
  };
}


function makeClientMock(
  opts?: { adjacentFaces?: string[] },
  capturePreview?: (cb: (r: PreviewResult) => void) => void,
) {
  let seq = 0;
  const adjacentFaces = opts?.adjacentFaces ?? ADJACENT;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareEdgeOp: vi.fn(() =>
      Promise.resolve({
        snapshotId: 7,
        targetBodyId: "body1",
        edges: EDGES.map((edge) => ({
          topoKey: edge.topoKey ?? "",
          elementId: edge.elementId ?? "",
          bodyId: "body1",
          kind: "edge" as const,
          picked: true,
          anchor: edge.anchor,
          contour: 0,
          ...(adjacentFaces.length > 0 ? { adjacentFaces } : {}),
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(() =>
      Promise.resolve({
        snapshotId: 7,
        mode: "Chamfer" as const,
        searchedRange: { min: 0, max: 0 },
        lowerBound: null,
        bestKnownMax: null,
        provenUpperBound: null,
        feasibleIntervals: [],
        intervalsTruncated: false,
        limitingEntities: [],
        confidence: "none" as const,
        monotonicObserved: true,
        probesUsed: 0,
        budgetExhausted: true,
        stoppedReason: "budgetExhausted" as const,
        refusal: null,
      }),
    ),
    // Deterministic promotion, mirroring the mock lane: one id per (body, topoKey).
    promoteSelection: vi.fn((bodyId: string, picks: PromotePick[]) =>
      Promise.resolve(
        picks.map((p) => ({
          topoKey: p.topoKey,
          elementId: `el_${p.topoKey.replace(":", "_")}`,
          kind: p.topoKey.startsWith("e:") ? "edge" : "face",
          bodyId,
        })),
      ),
    ),
    // The FACE CENTRE the reference-face anchor must be (never the edge midpoint,
    // which lies on both adjacent faces and would tie them on every replay).
    elementInfo: vi.fn((bodyId: string, elementId: string, topoKey?: string) =>
      Promise.resolve({
        elementId,
        topoKey: topoKey ?? "",
        bodyId,
        kind: "face",
        surfaceType: 0,
        curveType: -1,
        center: topoKey === "f:5" ? [50, 50, 50] : [10, 20, 30],
        normal: [0, 0, 1],
        hasNormal: true,
        size: 10,
        magnitude: 100,
      } as ElementInfo),
    ),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() =>
      Promise.resolve({
        radius: { value: 4 },
        edgeIds: ["el_e0"],
        edges: [{ primary: { bodyId: "body1", elementId: "el_e0", kind: "edge" } }],
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController — chamfer reference faces (SCHEMA §7.3, WP-F)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  let previewCb: ((r: PreviewResult) => void) | null;

  function build(opts?: { adjacentFaces?: string[] }): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(opts, (cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({ revision: 1 });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __resetLogForTests({ enabled: false });
  });

  /** Arm the unified edge tool on the box edge and flip it to Chamfer. */
  async function armChamfer(): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet");
    await flush();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    await flush();
  }

  /** The draft of the most recent `beginPreview` — the op the ✓ will commit. */
  function lastDraft(): PreviewDraft {
    const calls = clientMock.beginPreview.mock.calls;
    return calls[calls.length - 1][0];
  }

  it("an equal-leg chamfer authors NO pairs and promotes NOTHING", async () => {
    build();
    await armChamfer();
    expect(lastDraft().params.referenceFaces).toBeUndefined();
    expect(lastDraft().inputs).toHaveLength(1);
    // Core refuses an equal-leg record that names a reference face, and there is
    // nothing to promote for a face the op never measures on.
    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
  });

  it("the second leg authors ONE pair on `adjacentFaces[0]` — the legacy face", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();

    // `adjacentFaces[0]` is the smaller-ordinal face, so a DEFAULT pick reproduces
    // the pre-WP-F geometry exactly — authoring the field moves nothing.
    expect(clientMock.promoteSelection).toHaveBeenCalledWith(
      "body1",
      [{ topoKey: "f:3" }],
      7,
    );
    const draft = lastDraft();
    expect(draft.params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    // SLOT ORDER: the N edge refs, THEN the pair's face ref (`inputs[N + i]`), with
    // the FACE CENTRE as its anchor.
    expect(draft.inputs).toEqual([
      {
        primary: { bodyId: "body1", elementId: "el_e0", kind: "edge" },
        anchor: { worldPoint: [1, 2, 3] },
      },
      {
        primary: { bodyId: "body1", elementId: "el_f_3", kind: "face" },
        anchor: { worldPoint: [10, 20, 30] },
      },
    ]);
  });

  it("[Flip reference] rebinds every pair to the OTHER adjacent face", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();
    // The control is offered only once there IS a second face to flip to.
    expect(toolChipStore.getState().showChamferFlip).toBe(true);

    toolChipStore.getState().onChamferFlip?.();
    await flush();
    await flush();

    const draft = lastDraft();
    expect(draft.params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_5" }]);
    expect(draft.inputs?.[1].anchor).toEqual({ worldPoint: [50, 50, 50] });
  });

  it("clearing the second leg DROPS the pairs and the face input", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();
    expect(lastDraft().params.referenceFaces).toHaveLength(1);

    toolChipStore.getState().onDistance2?.(null);
    await flush();
    await flush();
    expect(lastDraft().params.referenceFaces).toBeUndefined();
    expect(lastDraft().inputs).toHaveLength(1);
    expect(toolChipStore.getState().showChamferFlip).toBe(false);
  });

  it("flipping to Fillet drops the pairs — a Fillet may not carry them", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();

    toolChipStore.getState().onEdgeOp?.("Fillet");
    await flush();
    await flush();
    expect(lastDraft().opType).toBe("Fillet");
    expect(lastDraft().params.referenceFaces).toBeUndefined();
    expect(lastDraft().inputs).toHaveLength(1);
  });

  it("a body with NO adjacency evidence blocks the ✓ with the reason, never commits", async () => {
    // An empty `adjacentFaces` is a REAL answer (a free edge, SCHEMA §7.6) as well
    // as the mock's gap, so no face can be named honestly. Committing would either
    // be refused by core or fall back to the ordinal reference face — so the tool
    // refuses, and says so in terms of the MODEL, not the backend.
    build({ adjacentFaces: [] });
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();

    toolChipStore.getState().onConfirm?.();
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalledWith(expect.anything(), true);
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toMatch(/no adjacent faces to measure the first distance on/);
    expect(hint?.message).not.toMatch(/MOCK LIMIT/);
    expect(toolChipStore.getState().showChamferFlip).toBe(false);
  });

  /** The arm's authored pairs, read off the controller's debug surface. */
  async function armedPairs(): Promise<Array<{ edgeId: string; faceId: string }> | null> {
    await flush();
    return (
      (controller as unknown as {
        chamferPairs: { pairs: Array<{ edgeId: string; faceId: string }> } | null;
      }).chamferPairs?.pairs ?? null
    );
  }

  /** Answer the newest `updatePreview` with a matching-epoch exact result — what
   *  `requireExactPreview` waits for before it lets the ✓ through. */
  function answerPreview(): void {
    const calls = clientMock.updatePreview.mock.calls as unknown as Array<
      [string, Record<string, unknown>, number]
    >;
    const last = calls[calls.length - 1];
    if (!last) return;
    previewCb?.({ sessionId: last[0], epoch: last[2], bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
  }

  it("Enter that sets the second leg AND confirms in one turn still commits the pairs", async () => {
    // The chip's Enter applies the value and then calls `onConfirm` synchronously,
    // so the pair resolution — and the preview-session reopen it forces — is still
    // in flight when the ✓ lands. The committed op is built from the session's
    // FROZEN `inputs[]`, so a ✓ that raced it would send an op with no face slot.
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    toolChipStore.getState().onConfirm?.();
    for (let i = 0; i < 8; i++) {
      await flush();
      answerPreview();
    }

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    const draft = lastDraft();
    expect(draft.params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    expect(draft.inputs).toHaveLength(2);
  });

  it("[Flip] and ✓ in ONE turn commits the FLIPPED pair, losing neither", async () => {
    // The flip re-resolves the pairs, which re-opens the preview session (its
    // `inputs[]` are frozen). A ✓ that raced it would `updatePreview` a session the
    // reopen is about to close and then time out waiting for an exact answer that
    // can never arrive — the confirm silently lost.
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    for (let i = 0; i < 4; i++) {
      await flush();
      answerPreview();
    }
    const before = (await armedPairs())![0];

    toolChipStore.getState().onChamferFlip?.();
    toolChipStore.getState().onConfirm?.();
    for (let i = 0; i < 10; i++) {
      await flush();
      answerPreview();
    }

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    const draft = lastDraft();
    const committed = draft.params.referenceFaces as Array<{ edgeId: string; faceId: string }>;
    expect(committed).toHaveLength(1);
    expect(committed[0].edgeId).toBe(before.edgeId);
    expect(committed[0].faceId).not.toBe(before.faceId);
    expect(draft.inputs?.[1].primary.elementId).toBe(committed[0].faceId);
  });

  it("promotes ONCE per (contour, face) however often the user flips back", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onDistance2?.(1);
    await flush();
    await flush();
    toolChipStore.getState().onChamferFlip?.();
    await flush();
    await flush();
    toolChipStore.getState().onChamferFlip?.();
    await flush();
    await flush();

    // f:3 and f:5, one promote each — the arm caches them.
    expect(clientMock.promoteSelection).toHaveBeenCalledTimes(2);
    expect(lastDraft().params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
  });

  // ── re-edit (SCHEMA §7.3: the update that INTRODUCES the asymmetry) ────────

  /** Arm the re-edit on a committed edge-op row with `stored` params. */
  async function reedit(stored: Record<string, unknown>, opType = "Chamfer"): Promise<void> {
    clientMock.getOperationParams.mockResolvedValue(
      stored as unknown as Awaited<ReturnType<typeof clientMock.getOperationParams>>,
    );
    documentStore.setState({
      features: [
        {
          id: "feat-ch",
          kind: "fillet",
          opType,
          label: opType,
          valueText: "4.0 mm",
          status: "ok",
        },
      ],
    });
    await controller.editEdgeOpFeature("feat-ch", opType as "Fillet" | "Chamfer");
    await flush();
    await flush();
  }

  const STORED_EQUAL_LEG = {
    radius: { value: 4 },
    edgeIds: ["el_e0"],
    edges: [{ primary: { bodyId: "body1", elementId: "el_e0", kind: "edge" } }],
    chainTangentEdges: true,
  };

  function lastUpdateParams(): Record<string, unknown> {
    const calls = clientMock.applyEditCommand.mock.calls as unknown as Array<
      [{ op: { params: Record<string, unknown> } }]
    >;
    return calls[calls.length - 1][0].op.params;
  }

  it("a re-edit that INTRODUCES the asymmetry authors the pairs in the SAME command", async () => {
    build();
    await reedit(STORED_EQUAL_LEG);
    // The closure is re-derived from the record's OWN edge ids, so the re-edit
    // groups contours exactly as the fresh arm did.
    expect(clientMock.prepareEdgeOp).toHaveBeenCalledWith(
      expect.objectContaining({
        mode: "Chamfer",
        pickedEdges: [{ bodyId: "body1", elementId: "el_e0" }],
        chainTangentEdges: true,
      }),
    );

    toolChipStore.getState().onDistance2?.(1);
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const params = lastUpdateParams();
    expect(params.distance2).toEqual({ value: 1 });
    expect(params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    // Both keys ride together — core refuses a length mismatch by name.
    expect(params.referenceFaceRefs).toEqual([
      {
        primary: { bodyId: "body1", elementId: "el_f_3", kind: "face" },
        anchor: { worldPoint: [10, 20, 30] },
      },
    ]);
  });

  it("clearing the asymmetry in a re-edit REMOVES both keys from the record", async () => {
    build();
    await reedit({
      ...STORED_EQUAL_LEG,
      distance2: { value: 1 },
      referenceFaces: [{ edgeId: "el_e0", faceId: "el_f_3" }],
      referenceFaceRefs: [
        {
          primary: { bodyId: "body1", elementId: "el_f_3", kind: "face" },
          anchor: { worldPoint: [10, 20, 30] },
        },
      ],
    });

    // A patch cannot delete a key, so an equal-leg result has to lose both from the
    // BASE — core refuses an equal-leg chamfer that names a reference face.
    toolChipStore.getState().onDistance2?.(null);
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const params = lastUpdateParams();
    expect("distance2" in params).toBe(false);
    expect("referenceFaces" in params).toBe(false);
    expect("referenceFaceRefs" in params).toBe(false);
  });

  it("a re-edit that keeps the asymmetry carries the record's OWN pairs through", async () => {
    const pairs = [{ edgeId: "el_e0", faceId: "el_STORED" }];
    const refs = [
      {
        primary: { bodyId: "body1", elementId: "el_STORED", kind: "face" },
        anchor: { worldPoint: [1, 1, 1] },
      },
    ];
    build();
    await reedit({ ...STORED_EQUAL_LEG, distance2: { value: 1 }, referenceFaces: pairs, referenceFaceRefs: refs });

    toolChipStore.getState().onValue?.(6);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    // A scalar edit must not re-pick the face: the user's persisted choice wins.
    const params = lastUpdateParams();
    expect(params.referenceFaces).toEqual(pairs);
    expect(params.referenceFaceRefs).toEqual(refs);
  });

  it("a LEGACY asymmetric record edited scalar-only sends NO referenceFaces", async () => {
    // THE regression this lane exists to prevent: authoring `adjacentFaces[0]` of
    // today's head here would silently persist the ordinal guess WP-F removes, make
    // the §9 repair item vanish, and record a face the user never chose. SCHEMA
    // §7.3: "a scalar edit on a record that lacks the field keeps it lacking".
    build();
    await reedit({ ...STORED_EQUAL_LEG, distance2: { value: 1 } });

    toolChipStore.getState().onValue?.(6);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const params = lastUpdateParams();
    expect(params.radius).toEqual({ value: 6 });
    expect(params.distance2).toEqual({ value: 1 });
    // Neither key — and emphatically not an EMPTY array, which Rust reads as
    // "strip the pairs" on a record that has them.
    expect("referenceFaces" in params).toBe(false);
    expect("referenceFaceRefs" in params).toBe(false);
    // …and nothing was promoted for a face nobody picked.
    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
  });

  it("a legacy record whose projection row has NO opType still authors nothing", async () => {
    // The "did this edit introduce the asymmetry" predicate is read off the STORED
    // PARAMS, not the projection's `opType` — which a legacy payload omits. Reading
    // it from there would fall through to "introduces" and author the ordinal pair
    // on exactly the record that must not get one.
    build();
    clientMock.getOperationParams.mockResolvedValue({
      ...STORED_EQUAL_LEG,
      distance2: { value: 1 },
    } as unknown as Awaited<ReturnType<typeof clientMock.getOperationParams>>);
    documentStore.setState({
      features: [
        { id: "feat-ch", kind: "fillet", label: "Chamfer", valueText: "4.0 mm", status: "ok" },
      ],
    });
    await controller.editEdgeOpFeature("feat-ch", "Chamfer");
    await flush();
    await flush();

    toolChipStore.getState().onValue?.(6);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const params = lastUpdateParams();
    expect("referenceFaces" in params).toBe(false);
    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
  });

  it("a legacy asymmetric record offers NO flip control — §9 repair is its path", async () => {
    build();
    await reedit({ ...STORED_EQUAL_LEG, distance2: { value: 1 } });
    expect(toolChipStore.getState().showChamferFlip).toBe(false);
  });

  it("seeds the flip side from the STORED face, so the first press really moves it", async () => {
    // A record committed on `adjacentFaces[1]` must not "flip" to the face it
    // already names: the flag restarts at false on every re-arm, so it is seeded by
    // matching the stored faceId against both promoted adjacent faces.
    build();
    await reedit({
      ...STORED_EQUAL_LEG,
      distance2: { value: 1 },
      referenceFaces: [{ edgeId: "el_e0", faceId: "el_f_5" }],
      referenceFaceRefs: [
        {
          primary: { bodyId: "body1", elementId: "el_f_5", kind: "face" },
          anchor: { worldPoint: [50, 50, 50] },
        },
      ],
    });
    expect(toolChipStore.getState().showChamferFlip).toBe(true);
    expect((controller as unknown as { chamferFlipped: boolean }).chamferFlipped).toBe(true);

    toolChipStore.getState().onChamferFlip?.();
    await flush();
    await flush();

    // Back to side 0 — a real move, not a no-op reported as success.
    const calls = clientMock.applyEditCommand.mock.calls as unknown as Array<
      [{ reference: { element: { primary: { elementId: string } } } }]
    >;
    expect(calls).toHaveLength(1);
    expect(calls[0][0].reference.element.primary.elementId).toBe("el_f_3");
  });

  it("hides the flip when the stored face is NEITHER adjacent face today", async () => {
    build();
    await reedit({
      ...STORED_EQUAL_LEG,
      distance2: { value: 1 },
      referenceFaces: [{ edgeId: "el_e0", faceId: "el_GONE" }],
      referenceFaceRefs: [
        {
          primary: { bodyId: "body1", elementId: "el_GONE", kind: "face" },
          anchor: { worldPoint: [0, 0, 0] },
        },
      ],
    });
    // "The other one" has no meaning against a face this head does not offer.
    expect(toolChipStore.getState().showChamferFlip).toBe(false);
    expect(viewportStore.getState().statusHint?.message).toMatch(
      /reference face is not one of the edge's current faces/,
    );
  });

  it("[Flip reference] on a re-edit sends ONE editOperationInput per pair", async () => {
    const pairs = [{ edgeId: "el_e0", faceId: "el_f_3" }];
    build();
    await reedit({
      ...STORED_EQUAL_LEG,
      distance2: { value: 1 },
      referenceFaces: pairs,
      referenceFaceRefs: [
        {
          primary: { bodyId: "body1", elementId: "el_f_3", kind: "face" },
          anchor: { worldPoint: [10, 20, 30] },
        },
      ],
    });
    expect(toolChipStore.getState().showChamferFlip).toBe(true);

    toolChipStore.getState().onChamferFlip?.();
    await flush();
    await flush();

    // The `edgeId` rides even on a REBIND: core then checks that this slot really
    // is the contour the caller thinks it is, instead of silently repairing another.
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "editOperationInput",
      record: "feat-ch",
      path: { path: "chamferReferenceFace", index: 0, edgeId: "el_e0" },
      reference: {
        element: {
          primary: { bodyId: "body1", elementId: "el_f_5", kind: "face" },
          anchor: { worldPoint: [50, 50, 50] },
        },
      },
    });
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
  });
});
