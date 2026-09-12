import { beforeEach, describe, expect, it } from "vitest";
import { deriveAvailableRect, largestClearRect, viewportWorkAreaStore } from "./viewportWorkAreaStore";

describe("viewportWorkAreaStore", () => {
  beforeEach(() => viewportWorkAreaStore.getState().reset());

  it.each([
    [1024, 704, 472],
    [1366, 1046, 814],
    [1728, 1408, 1176],
  ])("derives the free rectangle from actual panel edges at %ipx", (width, rightX, freeWidth) => {
    expect(deriveAvailableRect(
      { x: 0, y: 0, width, height: 700 },
      {
        left: { x: 12, y: 12, width: 220, height: 650 },
        right: { x: rightX, y: 0, width: 320, height: 666 },
        bottom: { x: 0, y: 666, width, height: 34 },
      },
    )).toEqual({ x: 232, y: 0, width: freeWidth, height: 666 });
  });

  it("handles absent workspace panels and suppresses equal publications", () => {
    const store = viewportWorkAreaStore.getState();
    store.setViewport({ x: 0, y: 0, width: 800, height: 600 });
    const before = viewportWorkAreaStore.getState();
    store.publishRegion("left", null);
    expect(viewportWorkAreaStore.getState()).toBe(before);
    store.publishRegion("right", { x: 768, y: 0, width: 32, height: 566 });
    expect(viewportWorkAreaStore.getState().available).toEqual({ x: 0, y: 0, width: 768, height: 600 });
  });

  it("tracks toolbar without subtracting it from the framing rectangle", () => {
    const store = viewportWorkAreaStore.getState();
    store.setViewport({ x: 0, y: 0, width: 1366, height: 768 });
    store.publishRegion("toolbar", { x: 240, y: 12, width: 900, height: 82 });
    expect(viewportWorkAreaStore.getState().available).toEqual({ x: 0, y: 0, width: 1366, height: 768 });
    expect(viewportWorkAreaStore.getState().regions.toolbar?.height).toBe(82);
  });

  it("includes the toolbar with keyed obstacles when deriving clear space", () => {
    const store = viewportWorkAreaStore.getState();
    store.setViewport({ x: 0, y: 0, width: 100, height: 100 });
    store.publishRegion("toolbar", { x: 0, y: 0, width: 100, height: 20 });
    expect(viewportWorkAreaStore.getState().obstacleClearRect).toEqual({
      x: 0, y: 32, width: 100, height: 68,
    });
  });

  it("publishes transient named obstacles into the padded largest clear rectangle", () => {
    const store = viewportWorkAreaStore.getState();
    store.setViewport({ x: 0, y: 0, width: 1000, height: 700 });
    store.publishObstacle("cornerCluster", { x: 900, y: 0, width: 100, height: 200 });
    expect(viewportWorkAreaStore.getState().obstacleClearRect).toEqual({ x: 0, y: 0, width: 888, height: 700 });
    store.publishObstacle("cornerCluster", null);
    expect(viewportWorkAreaStore.getState().obstacleClearRect).toEqual({ x: 0, y: 0, width: 1000, height: 700 });
  });
});

describe("largestClearRect", () => {
  const base = { x: 0, y: 0, width: 100, height: 100 };

  it("returns the base without obstacles and null when fully blocked", () => {
    expect(largestClearRect(base, [])).toEqual(base);
    expect(largestClearRect(base, [base])).toBeNull();
  });

  it("clips padded obstacles and ignores non-finite or zero-area inputs", () => {
    expect(largestClearRect(base, [{ x: 95, y: 0, width: 10, height: 100 }], 10)).toEqual({ x: 0, y: 0, width: 85, height: 100 });
    expect(largestClearRect(base, [{ x: Number.NaN, y: 0, width: 2, height: 2 }, { x: 4, y: 4, width: 0, height: 8 }])).toEqual(base);
  });

  it("uses strict overlap and stable top-then-left ties", () => {
    expect(largestClearRect(base, [{ x: 40, y: 0, width: 20, height: 100 }])).toEqual({ x: 0, y: 0, width: 40, height: 100 });
    expect(largestClearRect(base, [{ x: 0, y: 40, width: 100, height: 20 }])).toEqual({ x: 0, y: 0, width: 100, height: 40 });
  });
});
