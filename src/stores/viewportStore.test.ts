import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { viewportStore, AUTO_DISMISS_MS, ISOLATE_HINT } from "./viewportStore";
import { selectionStore, type EntityRef } from "./selectionStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const hint = () => viewportStore.getState().statusHint;
const set = (message: string | null, opts?: { severity?: "info" | "error"; sticky?: boolean }) =>
  viewportStore.getState().setStatusHint(message, opts);

describe("viewportStore.setStatusHint", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    set(null);
  });
  afterEach(() => {
    set(null); // cancel any armed timer
    vi.useRealTimers();
  });

  it("defaults to a non-sticky info hint that auto-dismisses after AUTO_DISMISS_MS", () => {
    set("Extruded");
    expect(hint()).toEqual({ message: "Extruded", severity: "info", sticky: false });

    vi.advanceTimersByTime(AUTO_DISMISS_MS - 1);
    expect(hint()).not.toBeNull();
    vi.advanceTimersByTime(1);
    expect(hint()).toBeNull();
  });

  it("keeps a sticky hint past the auto-dismiss window", () => {
    set("Select a plane", { sticky: true });
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 3);
    expect(hint()).toEqual({ message: "Select a plane", severity: "info", sticky: true });
  });

  it("stores error severity", () => {
    set("Extrude failed: boom", { severity: "error", sticky: true });
    expect(hint()).toEqual({ message: "Extrude failed: boom", severity: "error", sticky: true });
  });

  it("latest-wins: a newer hint re-arms the timer and a stale timer never clears it early", () => {
    set("first");
    vi.advanceTimersByTime(AUTO_DISMISS_MS - 100); // first is nearly expired
    set("second"); // supersedes → cancels first's timer, arms its own

    // The moment first's ORIGINAL deadline would have fired: second must survive.
    vi.advanceTimersByTime(100);
    expect(hint()?.message).toBe("second");

    // second lives out its own full window, then clears.
    vi.advanceTimersByTime(AUTO_DISMISS_MS - 100);
    expect(hint()).toBeNull();
  });

  it("explicit null clears and cancels the pending timer", () => {
    set("transient");
    set(null);
    expect(hint()).toBeNull();
    // A leftover timer would try to clear again (harmless) — assert nothing throws / re-fires.
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 2);
    expect(hint()).toBeNull();
  });

  it("a sticky hint replacing a non-sticky one cancels the pending auto-dismiss", () => {
    set("transient");
    set("prompt", { sticky: true });
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 2);
    expect(hint()?.message).toBe("prompt"); // the earlier non-sticky timer did not fire
  });
});

// ── W3 view UX: display mode · zoom-to-selection · isolate ───────────────────

function fakeEngine(previewHidden = false) {
  const engine = {
    fitView: vi.fn(),
    fitToBodies: vi.fn(),
    hasPreviewHiddenBodies: vi.fn(() => previewHidden),
  };
  setViewportEngine(engine as unknown as ViewportEngine);
  return engine;
}

function select(refs: EntityRef[]): void {
  selectionStore.getState().set(refs);
}

afterEach(() => {
  setViewportEngine(null);
  selectionStore.getState().clear();
  viewportStore.setState({ isolatedBodyIds: null, displayMode: "shadedEdges" });
  viewportStore.getState().setStatusHint(null);
});

describe("viewportStore display mode", () => {
  it("defaults to shadedEdges — what the renderer has always actually drawn", () => {
    expect(viewportStore.getState().displayMode).toBe("shadedEdges");
  });

  it("cycles shadedEdges → wireframe → shaded → shadedEdges", () => {
    const cycle = () => viewportStore.getState().cycleDisplayMode();
    cycle();
    expect(viewportStore.getState().displayMode).toBe("wireframe");
    cycle();
    expect(viewportStore.getState().displayMode).toBe("shaded");
    cycle();
    expect(viewportStore.getState().displayMode).toBe("shadedEdges");
  });
});

describe("viewportStore.zoomFit — frames the selection", () => {
  it("maps a body ref to its own id", () => {
    const engine = fakeEngine();
    select([{ kind: "body", id: "bodyA" }]);
    viewportStore.getState().zoomFit();
    expect(engine.fitToBodies).toHaveBeenCalledWith(["bodyA"]);
    expect(engine.fitView).not.toHaveBeenCalled();
  });

  it("maps face/edge picks to their OWNING body, de-duplicated", () => {
    const engine = fakeEngine();
    select([
      { kind: "face", id: "bodyA#f:1", bodyId: "bodyA" },
      { kind: "edge", id: "bodyA#e:2", bodyId: "bodyA" },
      { kind: "face", id: "bodyB#f:0", bodyId: "bodyB" },
    ]);
    viewportStore.getState().zoomFit();
    expect(engine.fitToBodies).toHaveBeenCalledWith(["bodyA", "bodyB"]);
  });

  it("falls back to fit-all for a selection with no bodies (sketch / region / datum)", () => {
    const engine = fakeEngine();
    select([
      { kind: "sketch", id: "sketch2" },
      { kind: "datum", id: "datum1" },
    ]);
    viewportStore.getState().zoomFit();
    expect(engine.fitView).toHaveBeenCalled();
    expect(engine.fitToBodies).not.toHaveBeenCalled();
  });

  it("falls back to fit-all with an empty selection", () => {
    const engine = fakeEngine();
    select([]);
    viewportStore.getState().zoomFit();
    expect(engine.fitView).toHaveBeenCalled();
  });

  it("is a no-op before the engine mounts", () => {
    setViewportEngine(null);
    select([{ kind: "body", id: "bodyA" }]);
    expect(() => viewportStore.getState().zoomFit()).not.toThrow();
  });
});

describe("viewportStore isolate", () => {
  it("isolates the selected bodies and shows the sticky hint", () => {
    fakeEngine();
    select([{ kind: "body", id: "bodyA" }, { kind: "face", id: "bodyB#f:1", bodyId: "bodyB" }]);
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["bodyA", "bodyB"]);
    expect(hint()).toEqual({ message: ISOLATE_HINT, severity: "info", sticky: true });
  });

  it("refuses to isolate a selection that names no body (would hide everything)", () => {
    fakeEngine();
    select([{ kind: "sketch", id: "sketch2" }]);
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
    expect(hint()).toBeNull();
  });

  it("toggles back off and clears the hint", () => {
    fakeEngine();
    select([{ kind: "body", id: "bodyA" }]);
    viewportStore.getState().toggleIsolate();
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
    expect(hint()).toBeNull();
  });

  it("exitIsolate leaves a FOREIGN hint alone (a tool prompt must survive)", () => {
    fakeEngine();
    select([{ kind: "body", id: "bodyA" }]);
    viewportStore.getState().isolateSelection();
    set("Select a face to measure", { sticky: true });
    viewportStore.getState().exitIsolate();
    expect(hint()?.message).toBe("Select a face to measure");
  });

  it("exitIsolate is a no-op when isolation is already off", () => {
    set("Drag the arrow to set depth", { sticky: true });
    viewportStore.getState().exitIsolate();
    expect(hint()?.message).toBe("Drag the arrow to set depth");
  });

  // The preview save/restore snapshot owns `visible` for the bodies it hides, so
  // the toggle must not move underneath it — in EITHER direction.
  it("ignores the toggle while a preview holds bodies hidden (enter)", () => {
    fakeEngine(true);
    select([{ kind: "body", id: "bodyA" }]);
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
  });

  it("ignores the toggle while a preview holds bodies hidden (exit)", () => {
    const engine = fakeEngine();
    select([{ kind: "body", id: "bodyA" }]);
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["bodyA"]);

    engine.hasPreviewHiddenBodies.mockReturnValue(true);
    viewportStore.getState().toggleIsolate();
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["bodyA"]); // still isolated
  });
});
