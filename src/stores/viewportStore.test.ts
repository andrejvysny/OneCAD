import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  viewportStore,
  AUTO_DISMISS_MS,
  ISOLATE_HINT,
  SECTION_DEFAULT,
  SECTION_HINT,
  SECTION_UNSUPPORTED_HINT,
} from "./viewportStore";
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
    // The section seeds its offset from the scene; default to "no bodies".
    sectionOffsetRange: vi.fn((): { min: number; max: number } | null => null),
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
  viewportStore.setState({ isolatedBodyIds: null });
  viewportStore.getState().setStatusHint(null);
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

describe("viewportStore section (transient cut-away)", () => {
  const section = () => viewportStore.getState().section;
  beforeEach(() => {
    viewportStore.setState({ section: SECTION_DEFAULT });
    set(null);
  });
  afterEach(() => {
    viewportStore.setState({ section: SECTION_DEFAULT });
    set(null);
  });

  it("starts off, on XY, at the origin", () => {
    expect(section()).toEqual({ enabled: false, plane: "XY", offsetMm: 0, flip: false });
  });

  it("toggles on with the sticky hint and back off, clearing it", () => {
    viewportStore.getState().toggleSection();
    expect(section().enabled).toBe(true);
    expect(hint()).toEqual({ message: SECTION_HINT, severity: "info", sticky: true });

    viewportStore.getState().toggleSection();
    expect(section().enabled).toBe(false);
    expect(hint()).toBeNull();
  });

  it("keeps the plane and flip across a toggle; the OFFSET is re-seeded on enable", () => {
    fakeEngine(); // no bodies ⇒ the seed is 0
    viewportStore.getState().toggleSection();
    viewportStore.getState().setSectionPlane("YZ");
    viewportStore.getState().setSectionOffset(12);
    viewportStore.getState().flipSection();
    viewportStore.getState().toggleSection();

    // Off: nothing is thrown away, so the menu still shows the cut it had.
    expect(section()).toEqual({ enabled: false, plane: "YZ", offsetMm: 12, flip: true });

    // On again: plane and flip are the user's CHOICE and survive, but the offset
    // is re-measured against whatever is in the scene now (see the seeding tests
    // below) — 12 mm may not intersect this document at all.
    viewportStore.getState().toggleSection();
    expect(section()).toEqual({ enabled: true, plane: "YZ", offsetMm: 0, flip: true });
  });

  it("changing the plane re-seeds the offset — it was measured along the OLD axis", () => {
    fakeEngine(); // no bodies ⇒ range null ⇒ 0
    viewportStore.getState().setSectionOffset(9);
    viewportStore.getState().setSectionPlane("XZ");
    expect(section()).toMatchObject({ plane: "XZ", offsetMm: 0 });
  });

  /*
   * The offset SEEDING, which is what keeps the feature from erasing an ordinary
   * part: a body extruded up from the XY plane occupies z ∈ [0, depth], and the
   * unflipped cut keeps `z <= offset` — so a cut seeded at 0 discards the whole
   * body, caps nothing (the stencil sums to zero) and leaves an empty viewport
   * that only the slider can undo.
   */
  it("seeds the offset to the MIDDLE of the scene on enable, not to the origin", () => {
    const engine = fakeEngine();
    engine.sectionOffsetRange.mockReturnValue({ min: 0, max: 20 });

    viewportStore.getState().toggleSection();

    expect(engine.sectionOffsetRange).toHaveBeenCalledWith("XY");
    expect(section()).toMatchObject({ enabled: true, offsetMm: 10 });
  });

  it("re-seeds along the NEW axis when the plane changes", () => {
    const engine = fakeEngine();
    engine.sectionOffsetRange.mockReturnValue({ min: 0, max: 20 });
    viewportStore.getState().toggleSection();

    engine.sectionOffsetRange.mockReturnValue({ min: -50, max: -10 });
    viewportStore.getState().setSectionPlane("YZ");

    expect(engine.sectionOffsetRange).toHaveBeenLastCalledWith("YZ");
    expect(section()).toMatchObject({ plane: "YZ", offsetMm: -30 });
  });

  it("re-seeds on every enable, so a stale offset cannot survive into a new document", () => {
    const engine = fakeEngine();
    engine.sectionOffsetRange.mockReturnValue({ min: 0, max: 20 });
    viewportStore.getState().toggleSection();
    viewportStore.getState().setSectionOffset(19);
    viewportStore.getState().toggleSection(); // off; the offset is deliberately kept

    engine.sectionOffsetRange.mockReturnValue({ min: 100, max: 140 });
    viewportStore.getState().toggleSection();

    expect(section()).toMatchObject({ enabled: true, offsetMm: 120 });
  });

  it("falls back to 0 with nothing in the scene to measure", () => {
    fakeEngine(); // range null
    viewportStore.getState().toggleSection();
    expect(section().offsetMm).toBe(0);
  });

  it("no-ops on an unchanged plane or offset (a drag tick must not churn state)", () => {
    const before = section();
    viewportStore.getState().setSectionPlane("XY");
    viewportStore.getState().setSectionOffset(0);
    expect(section()).toBe(before); // same object ⇒ no subscriber fired
  });

  it("rejects a non-finite offset", () => {
    viewportStore.getState().setSectionOffset(Number.NaN);
    expect(section().offsetMm).toBe(0);
  });

  it("refuses on the WebGPU backend — a cut that cannot cut is announced, not shown", () => {
    const engine = fakeEngine();
    (engine as unknown as { isWebGPU: boolean }).isWebGPU = true;

    viewportStore.getState().toggleSection();

    expect(section().enabled).toBe(false);
    expect(hint()).toEqual({
      message: SECTION_UNSUPPORTED_HINT,
      severity: "error",
      sticky: false,
    });
    setViewportEngine(null);
  });

  it("exitSection is idempotent and leaves a FOREIGN hint alone", () => {
    set("Drag the arrow to set depth", { sticky: true });
    viewportStore.getState().exitSection();
    expect(hint()?.message).toBe("Drag the arrow to set depth");

    viewportStore.getState().toggleSection();
    set("Select a face to measure", { sticky: true });
    viewportStore.getState().exitSection();
    expect(section().enabled).toBe(false);
    expect(hint()?.message).toBe("Select a face to measure");
  });
});
