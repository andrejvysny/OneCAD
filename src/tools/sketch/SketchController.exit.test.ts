/*
 * SketchController exit-order pin (EXTRUDE-COMMIT-FIX / REVOLVE-REGION-PARITY).
 *
 * Every keep-exit must run `cancelSketch` FIRST (worker-gesture teardown + the
 * take-once session squash) and `finishSketch` SECOND (solve + regions + the
 * `Sketch` TIMELINE RECORD upsert). The order is load-bearing, not cosmetic:
 *   - finish-then-cancel would squash AFTER the record was minted, and
 *   - cancel-only (the pre-postmortem behaviour) left every interactively drawn
 *     sketch RECORDLESS, so the regen planner failed every later modeling-op
 *     commit with "profile sketch not found in plan".
 *
 * The re-edit + arm paths now deliberately read regions PURELY
 * (`getSketchRegions`), so this exit is one of only two places that author the
 * record at all — the other being the commit-boundary guarantee. Pin it.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};
const ENTITIES: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
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
    moveChip: vi.fn(),
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchAngleReference: vi.fn(),
    setSketchAnglePreview: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    setSketchProjectedIds: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
    getCameraDistance: vi.fn(() => 100),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController exit ordering", () => {
  let container: HTMLDivElement;
  let controller: SketchController;
  let calls: string[];
  let clientMock: {
    enterSketch: ReturnType<typeof vi.fn>;
    cancelSketch: ReturnType<typeof vi.fn>;
    deleteSketch: ReturnType<typeof vi.fn>;
    finishSketch: ReturnType<typeof vi.fn>;
  };

  beforeEach(() => {
    resetStores();
    calls = [];
    container = document.createElement("div");
    document.body.appendChild(container);
    clientMock = {
      enterSketch: vi.fn(
        (): Promise<SketchSession> =>
          Promise.resolve({
            sketchId: "sketch1",
            plane: PLANE,
            entities: ENTITIES.map((e) => ({ ...e })),
            constraints: [],
            dof: 2,
            status: "UnderConstrained",
          }),
      ),
      cancelSketch: vi.fn((id: string, opts?: { discard?: boolean }) => {
        calls.push(`cancelSketch:${id}${opts?.discard ? ":discard" : ""}`);
        return Promise.resolve({ discarded: opts?.discard === true });
      }),
      deleteSketch: vi.fn((id: string) => {
        calls.push(`deleteSketch:${id}`);
        return Promise.resolve();
      }),
      finishSketch: vi.fn((id: string) => {
        calls.push(`finishSketch:${id}`);
        return Promise.resolve({ regions: [] });
      }),
    };
    controller = new SketchController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  it("exiting sketch mode runs cancelSketch THEN finishSketch (record mint after the squash)", async () => {
    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();
    expect(sketchStore.getState().session).not.toBeNull();

    toolStore.getState().setMode("model"); // keep-exit
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1", "finishSketch:sketch1"]);
    expect(sketchStore.getState().session).toBeNull();
  });

  it("a failing cancelSketch does not silently skip the record mint chain", async () => {
    // The exit is best-effort: a cancel rejection is caught and logged, but the
    // sequence must not leave the session dangling.
    clientMock.cancelSketch.mockImplementation((id: string) => {
      calls.push(`cancelSketch:${id}`);
      return Promise.reject(new Error("worker gone"));
    });
    // The failure now lands on the structured log lane (DEV-OBSERVABILITY Wave
    // F), whose gate vitest keeps closed — open it just for this assertion.
    __resetLogForTests();

    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();
    toolStore.getState().setMode("model");
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1"]); // finish never reached
    // ...but the failure is LOUD, never silent.
    expect(logSnapshot().filter((e) => e.level === "error" && e.tag === "sketch")).toHaveLength(1);
    expect(sketchStore.getState().session).toBeNull();
    __resetLogForTests({ enabled: false });
  });
  /*
   * WP-S1 (was a red-first S0 probe; both assertions were measured failing on
   * 2026-09-09 before the fix). A refused profile used to leave the user with
   * nothing on screen, and the one thing that IS recorded is wrong.
   *
   * Measured on a real session (`logs/dev.jsonl`, 2026-09-09 17:30): the user
   * drew a profile, exited, and `finishSketch` was refused with "SketchRegions:
   * profile has overlapping or coincident analytic curves". `SketchController`
   * runs the exit chain in a fire-and-forget `void (async () => …)()` and, on
   * failure, calls only `logError` — no `errorHint` — then closes the session
   * regardless. The user saw nothing and found out later, when arming Extrude.
   *
   * The log copy is also FALSE. `document_runtime.rs` upserts the timeline
   * record BEFORE the regions call precisely so a region refusal cannot lose it,
   * and returns `FinishSketchError { committed }` so the api layer still emits,
   * schedules and persists. The record is committed; the regions CACHE is what
   * failed. "timeline record may be missing" sends a reader hunting a bug that
   * is not there.
   */
  it("surfaces a refused finishSketch to the user, and does not claim the record is missing", async () => {
    const refusal = Object.assign(
      new Error("op failed (OpFailed, recoverable=true): SketchRegions: profile has overlapping or coincident analytic curves"),
      {
        kind: "opFailed",
        diagnostics: [
          {
            severity: "error",
            code: "OP_FAILED",
            reasonCode: "SKETCH_PROFILE_OVERLAPPING_CURVES",
            message: "profile has overlapping or coincident analytic curves",
            stage: "profile",
            evidence: { entityIds: ["e1", "e2"] },
          },
        ],
      },
    );
    clientMock.finishSketch.mockImplementation((id: string) => {
      calls.push(`finishSketch:${id}`);
      return Promise.reject(refusal);
    });
    // The structured log lane's gate is closed under vitest — open it for this
    // assertion, exactly as the cancel-failure spec above does.
    __resetLogForTests();

    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();
    toolStore.getState().setMode("model");
    await flush();
    await flush();

    const entries = logSnapshot();
    const finishFailure = entries.find((e) => String(e.msg).includes("finishSketch FAILED"));
    // (a) the user must be told something went wrong
    const hint = viewportStore.getState().statusHint;
    expect(hint, "a refused finish must reach the user, not only the log").not.toBeNull();
    expect(hint?.severity).toBe("error");

    // (b) and the log must not send a reader hunting a record that IS committed
    expect(
      String(finishFailure?.msg ?? ""),
      "the timeline record is committed before regions; the copy must not say otherwise",
    ).not.toContain("timeline record may be missing");
    __resetLogForTests({ enabled: false });
  });

  /*
   * WP-U7 D-1 — the chrome bar's Cancel DISCARDS. It arms `sketchStore.exitIntent`
   * and flips the mode; this exit must then revert the session and skip
   * `finishSketch` entirely (minting a timeline record for geometry that was just
   * reverted would record something the user never asked for).
   */
  it("a discarding exit reverts the session and never mints the record", async () => {
    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();

    sketchStore.getState().setExitIntent("discard");
    toolStore.getState().setMode("model");
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1:discard"]);
    expect(clientMock.finishSketch).not.toHaveBeenCalled();
    // A RE-ENTERED sketch is never deleted — only one minted in this visit is.
    expect(clientMock.deleteSketch).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe("Sketch changes discarded");
    expect(sketchStore.getState().session).toBeNull();
    // Take-once: the next exit is a plain keep-exit again.
    expect(sketchStore.getState().exitIntent).toBe("keep");
  });

  it("a REFUSED discard keeps the geometry and says so", async () => {
    clientMock.cancelSketch.mockImplementation((id: string) => {
      calls.push(`cancelSketch:${id}`);
      return Promise.resolve({ discarded: false, keptReason: "history was trimmed" });
    });

    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();
    sketchStore.getState().setExitIntent("discard");
    toolStore.getState().setMode("model");
    await flush();
    await flush();

    const hint = viewportStore.getState().statusHint;
    expect(hint?.message).toBe("Cannot discard — history was trimmed; changes kept");
    expect(hint?.severity).toBe("warn");
    expect(clientMock.deleteSketch).not.toHaveBeenCalled();
    // A refused discard is a keep-exit: the timeline record must still be
    // minted/refreshed, or the model would be built from stale geometry.
    expect(calls).toEqual(["cancelSketch:sketch1", "finishSketch:sketch1"]);
  });

  it("Esc / Finish leave the keep-exit untouched", async () => {
    toolStore.getState().setMode("sketch", "sketch1", { tool: "line" });
    await flush();
    toolStore.getState().setMode("model"); // no intent armed
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1", "finishSketch:sketch1"]);
  });
});
