/*
 * Edge-op + shell NUMERIC entry (jsdom) — what a typed size may and may not do.
 *
 *  1. Spec §8.2 / review §4.2, TODO.md SESSION 37 H5: a size below
 *     `EDGE_OP_MIN_VALUE` is INVALID. It is never silently converted to the
 *     minimum, the field keeps the user's text, the preview keeps the last valid
 *     size, and nothing commits — at every entry, including the two deferred ones
 *     a pending range check applies later.
 *  2. Spec §9.2, TODO.md SESSION 37 H2b: Escape in the size field reverts through
 *     the controller's registered handler, which also drops its draft and the
 *     range verdict's status line.
 *  3. The shell ✓ refuses a non-valid validation like its siblings do.
 *
 * `revert()` replays `ModelToolChips.revertPrimary` exactly (registered handler,
 * else the clear-validation fallback), so a test reads what the chip would do.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { AnalyzeEdgeOpRangeResult, ApplyOperationResult, PreviewDraft, PreviewResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { formatLengthWithUnit } from "@/units/format";

const ok = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

const EDGES: EntityRef[] = [
  { kind: "edge", id: "body1#e:5", bodyId: "body1", topoKey: "e:5", elementId: "el-edge-5", anchor: { worldPoint: [1, 2, 3] } },
];

const FACE: EntityRef = {
  kind: "face",
  id: "body1#f:2",
  bodyId: "body1",
  topoKey: "f:2",
  elementId: "el-face-2",
  anchor: { worldPoint: [0, 0, 10] },
};

const MIN_MESSAGE = `Must be at least ${formatLengthWithUnit(0.1)}`;

function range(over: Partial<AnalyzeEdgeOpRangeResult>): AnalyzeEdgeOpRangeResult {
  return {
    snapshotId: 1,
    mode: "Fillet",
    searchedRange: { min: 0.001, max: 50 },
    lowerBound: 0.001,
    bestKnownMax: 5,
    provenUpperBound: 5.1,
    feasibleIntervals: [{ lower: 0.001, upper: 5 }],
    intervalsTruncated: false,
    limitingEntities: [],
    confidence: "bracketed",
    monotonicObserved: true,
    probesUsed: 8,
    budgetExhausted: false,
    stoppedReason: "bracketed",
    refusal: null,
    ...over,
  } as AnalyzeEdgeOpRangeResult;
}

function makeEngineMock() {
  return {
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
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
    hideRegionPick: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showScreenValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

function makeClientMock(capture: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  let settleRange: (r: AnalyzeEdgeOpRangeResult) => void = () => {};
  const client = {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture(cb);
      return () => {};
    }),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    prepareEdgeOp: vi.fn(() =>
      Promise.resolve({
        snapshotId: 1,
        targetBodyId: "body_body1",
        edges: EDGES.map((edge) => ({
          topoKey: edge.topoKey ?? "",
          elementId: edge.elementId,
          bodyId: "body_body1",
          kind: "edge" as const,
          picked: true,
          anchor: edge.anchor,
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(
      () => new Promise<AnalyzeEdgeOpRangeResult>((resolve) => (settleRange = resolve)),
    ),
    promoteSelection: vi.fn((bodyId: string, picks: { topoKey: string }[]) =>
      Promise.resolve(picks.map((p) => ({ topoKey: p.topoKey, elementId: `el-${p.topoKey}`, kind: "face", bodyId }))),
    ),
    beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(ok())),
    applyOperation: vi.fn(() => Promise.resolve(ok())),
    applyEditCommand: vi.fn(() => Promise.resolve(ok())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
  return { client, settleRange: (r: AnalyzeEdgeOpRangeResult) => settleRange(r) };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const settleTrailing = (): Promise<void> => new Promise((r) => setTimeout(r, 320));

describe("edge-op and shell numeric entry", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let mock: ReturnType<typeof makeClientMock>;
  let previewCb: ((r: PreviewResult) => void) | null;
  let engine: ReturnType<typeof makeEngineMock>;

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    previewCb = null;
    mock = makeClientMock((cb) => (previewCb = cb));
    engine = makeEngineMock();
    controller = new ModelToolController({
      engine: engine as unknown as ViewportEngine,
      client: mock.client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  async function armFillet(): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet");
    await flush();
    await flush();
  }

  /** `ModelToolChips.revertPrimary`, verbatim in effect. */
  function revert(start: number): void {
    const s = toolChipStore.getState();
    s.setRawValueValidity("primary", true, String(start));
    if (s.onRevertValue) {
      s.onRevertValue(start);
      return;
    }
    s.clearValidation();
    s.onValue?.(start);
  }

  function lastRadius(): unknown {
    const calls = mock.client.updatePreview.mock.calls as unknown[][];
    return (calls[calls.length - 1]?.[1] as { radius?: unknown } | undefined)?.radius;
  }

  function answerPreview(): void {
    const calls = mock.client.updatePreview.mock.calls as unknown[][];
    const last = calls[calls.length - 1];
    previewCb?.({ sessionId: last[0] as string, epoch: last[2] as number, bodyId: "preview", bodies: [], replacedBodyIds: [] });
  }

  const committed = (): boolean =>
    mock.client.applyOperation.mock.calls.length > 0 ||
    (mock.client.endPreview.mock.calls as unknown[][]).some((call) => call[1] === true);

  // ── 1. below the minimum is invalid, never converted ─────────────────────────

  it.each([-3, 0, 0.05])("typed %s is INVALID: the field text survives, the preview keeps its size, nothing commits", async (typed) => {
    await armFillet();
    mock.settleRange(range({ confidence: "none", lowerBound: null, bestKnownMax: null, provenUpperBound: null, feasibleIntervals: [] }));
    await flush();
    expect(toolChipStore.getState().validation.status).toBe("valid");
    await settleTrailing();
    const sendsBefore = mock.client.updatePreview.mock.calls.length;

    toolChipStore.getState().onValue?.(typed);

    const validation = toolChipStore.getState().validation;
    expect(validation).toMatchObject({ status: "invalid", draft: typed, message: MIN_MESSAGE });
    expect(validation.status === "invalid" && validation.suggestedValue).toBeFalsy(); // no "use nearest" offer
    expect(toolChipStore.getState().value).toBe(2); // never setValue(0.1) over the typed text
    await settleTrailing();
    expect(mock.client.updatePreview.mock.calls.length).toBe(sendsBefore);
    expect(lastRadius()).toBe(2);

    toolChipStore.getState().onConfirm?.();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(committed()).toBe(false);
  });

  it("−3 typed while the range check is pending, then Enter, cannot commit 0.1 when the check settles", async () => {
    await armFillet();
    toolChipStore.getState().onValue?.(1); // proves the range check is still in flight
    expect(toolChipStore.getState().validation.status).toBe("pending");

    toolChipStore.getState().onValue?.(-3);
    toolChipStore.getState().onConfirm?.();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    mock.settleRange(range({}));
    await flush();
    await flush();
    await settleTrailing();

    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", draft: -3, message: MIN_MESSAGE });
    expect(toolChipStore.getState().value).toBe(2);
    expect(lastRadius()).toBe(2);
    expect(committed()).toBe(false);
  });

  it("an UNBOUNDED range answer does not apply a pending −3 either", async () => {
    await armFillet();
    toolChipStore.getState().onValue?.(-3);
    mock.settleRange(range({ refusal: { code: "unsupported", message: "no range", edges: [] } }));
    await flush();
    await flush();
    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", draft: -3 });
    expect(toolChipStore.getState().value).toBe(2);
  });

  // ── 2. Escape reverts through the controller ─────────────────────────────────

  it("fillet 50 over the measured ceiling, then Escape: valid again, preview at the start, arm hint back", async () => {
    await armFillet();
    mock.settleRange(range({}));
    await flush();
    answerPreview(); // the arm hint takes the status line back from "Computing preview…"
    const armHint = viewportStore.getState().statusHint?.message;
    expect(armHint).toMatch(/^Fillet 1 edge/);

    toolChipStore.getState().onValue?.(50);
    expect(toolChipStore.getState().validation.status).toBe("invalid");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");

    revert(2);

    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
    expect(toolChipStore.getState().value).toBe(2);
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
    await settleTrailing();
    expect(lastRadius()).toBe(2);
    expect(committed()).toBe(false);
  });

  it("fillet typed while the range is pending, then Escape: the late settle does NOT apply the abandoned draft", async () => {
    await armFillet();
    expect(toolChipStore.getState().onRevertValue).toBeTypeOf("function");
    toolChipStore.getState().onValue?.(4);
    expect(toolChipStore.getState().validation).toMatchObject({ status: "pending", draft: 4 });

    revert(2);
    mock.settleRange(range({ bestKnownMax: 10, provenUpperBound: 10.1, feasibleIntervals: [{ lower: 0.001, upper: 10 }] }));
    await flush();
    await flush();
    await settleTrailing();

    expect(toolChipStore.getState().value).toBe(2);
    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
    expect(lastRadius()).toBe(2);
    expect(committed()).toBe(false);
  });

  // ── H4b W1/W5/W6 ─────────────────────────────────────────────────────────────

  /** A handle drag straight UP the screen: the degraded proxy grows the size 1 px : 1 unit. */
  function dragUp(px: number): void {
    engine.hitExtrudeHandle.mockReturnValue(true);
    const at = (type: string, y: number): void => {
      container.dispatchEvent(new MouseEvent(type, { clientX: 100, clientY: y, button: 0, buttons: type === "pointerup" ? 0 : 1, bubbles: true }));
    };
    at("pointerdown", 200);
    at("pointermove", 200 - px);
    at("pointerup", 200 - px);
  }

  it("W1: a drag after a refused typed size REPLACES the draft — valid, arm hint back, a late range settle does not re-refuse", async () => {
    await armFillet(); // the range check stays in flight
    answerPreview();
    const armHint = viewportStore.getState().statusHint?.message;
    toolChipStore.getState().onValue?.(0.05);
    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", message: MIN_MESSAGE });

    dragUp(20);
    expect(toolChipStore.getState().value).toBe(22);
    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);

    mock.settleRange(range({ confidence: "none", lowerBound: null, bestKnownMax: null, provenUpperBound: null, feasibleIntervals: [] }));
    await flush();
    await flush();
    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });

    toolChipStore.getState().onConfirm?.();
    answerPreview();
    await flush();
    await flush();
    expect(committed()).toBe(true);
  });

  it("W5: a verified maximum BELOW the 0.1 mm minimum refuses a typed size with no suggestion", async () => {
    await armFillet();
    mock.settleRange(range({ lowerBound: 0.001, bestKnownMax: 0.05, provenUpperBound: 0.06, feasibleIntervals: [{ lower: 0.001, upper: 0.05 }] }));
    await flush();

    toolChipStore.getState().onValue?.(2);
    const validation = toolChipStore.getState().validation;
    expect(validation.status).toBe("invalid");
    expect(validation.status === "invalid" ? validation.suggestedValue : "none").toBeUndefined();
    expect(validation.status === "invalid" ? validation.message : "").toMatch(/exceeds the verified maximum/);
    expect(toolChipStore.getState().onUseSuggestedValue).toBeNull();
  });

  it("W5: …and a drag against it refuses too, instead of settling on 0.1 above the verified maximum", async () => {
    await armFillet();
    mock.settleRange(range({ lowerBound: 0.001, bestKnownMax: 0.05, provenUpperBound: 0.06, feasibleIntervals: [{ lower: 0.001, upper: 0.05 }] }));
    await flush();
    answerPreview();

    dragUp(20);
    const validation = toolChipStore.getState().validation;
    expect(validation.status).toBe("invalid");
    expect(validation.status === "invalid" ? validation.suggestedValue : "none").toBeUndefined();

    toolChipStore.getState().onConfirm?.();
    answerPreview();
    await flush();
    await flush();
    expect(committed()).toBe(false);
  });

  it("W6: a size below the minimum takes the status line from an earlier 'exceeds the verified maximum'", async () => {
    await armFillet();
    mock.settleRange(range({}));
    await flush();
    toolChipStore.getState().onValue?.(50);
    expect(viewportStore.getState().statusHint?.message).toMatch(/exceeds the verified maximum/);

    toolChipStore.getState().onValue?.(0.05);
    expect(viewportStore.getState().statusHint).toMatchObject({ message: MIN_MESSAGE, severity: "error" });
  });

  // ── 3. the shell ✓ gate ──────────────────────────────────────────────────────

  it("shell: a stale raw input error refuses the ✓ (no commit of the last valid thickness)", async () => {
    selectionStore.getState().set([FACE]);
    toolStore.getState().setTool("shell");
    await flush();
    await flush();
    answerPreview();
    toolChipStore.getState().setRawValueValidity("primary", false, "2.");

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(committed()).toBe(false);
    expect(toolStore.getState().modelTool).toBe("shell");
  });
});
