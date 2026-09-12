import { StrictMode, useState } from "react";
import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { viewportWorkAreaStore } from "@/stores/viewportWorkAreaStore";
import { obstacleUnion, useMeasuredObstacle } from "./useMeasuredObstacle";

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height } as DOMRect;
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  viewportWorkAreaStore.getState().reset();
});

describe("obstacleUnion", () => {
  it("includes transformed ViewCube face protrusions beyond the root", () => {
    const viewport = document.createElement("div");
    const root = document.createElement("div");
    const face = document.createElement("button");
    const unrelated = document.createElement("button");
    face.setAttribute("data-obstacle-protrusion", "");
    unrelated.setAttribute("aria-label", "not a view");
    root.append(face, unrelated);
    vi.spyOn(viewport, "getBoundingClientRect").mockReturnValue(rect(100, 50, 800, 600));
    vi.spyOn(root, "getBoundingClientRect").mockReturnValue(rect(840, 70, 60, 60));
    vi.spyOn(face, "getBoundingClientRect").mockReturnValue(rect(820, 60, 100, 100));
    vi.spyOn(unrelated, "getBoundingClientRect").mockReturnValue(rect(0, 0, 800, 600));
    expect(obstacleUnion(root, viewport)).toEqual({ x: 720, y: 10, width: 80, height: 100 });
  });
});

describe("useMeasuredObstacle", () => {
  it("coalesces resize publications and clears its obstacle on StrictMode unmount", () => {
    const observers: ResizeObserverMock[] = [];
    class ResizeObserverMock {
      readonly observe = vi.fn();
      readonly disconnect = vi.fn();
      constructor(readonly callback: ResizeObserverCallback) { observers.push(this); }
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    let queued: FrameRequestCallback | null = null;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      queued = callback;
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.dataset.viewport !== undefined) return rect(0, 0, 1024, 700);
      return rect(900, 20, 80, 120);
    });

    function Harness() {
      const [root, setRoot] = useState<HTMLDivElement | null>(null);
      useMeasuredObstacle("cornerCluster", root);
      return <div data-viewport><div ref={setRoot} /></div>;
    }
    const mounted = render(<StrictMode><Harness /></StrictMode>);
    act(() => queued?.(0));
    expect(viewportWorkAreaStore.getState().obstacles.cornerCluster).toEqual({ x: 900, y: 20, width: 80, height: 120 });
    const observer = observers[observers.length - 1];
    expect(observer?.observe).toHaveBeenCalledTimes(2);
    act(() => {
      observer?.callback([], observer as unknown as ResizeObserver);
      observer?.callback([], observer as unknown as ResizeObserver);
      queued?.(0);
    });
    mounted.unmount();
    expect(viewportWorkAreaStore.getState().obstacles.cornerCluster).toBeUndefined();
    expect(observers.every((observer) => observer.disconnect.mock.calls.length > 0)).toBe(true);
  });

  it("remeasures when a work-area region moves, without self-feedback", () => {
    const observers: ResizeObserverMock[] = [];
    class ResizeObserverMock {
      readonly observe = vi.fn();
      readonly disconnect = vi.fn();
      constructor(readonly callback: ResizeObserverCallback) { observers.push(this); }
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    const frames: FrameRequestCallback[] = [];
    const request = vi.fn((callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    vi.stubGlobal("requestAnimationFrame", request);
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    let rootLeft = 900;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.dataset.viewport !== undefined) return rect(0, 0, 1024, 700);
      return rect(rootLeft, 20, 80, 120);
    });

    function Harness() {
      const [root, setRoot] = useState<HTMLDivElement | null>(null);
      useMeasuredObstacle("cornerCluster", root);
      return <div data-viewport><div ref={setRoot} /></div>;
    }
    render(<Harness />);
    act(() => frames.shift()?.(0));
    expect(viewportWorkAreaStore.getState().obstacles.cornerCluster?.x).toBe(900);
    const initialFrameCount = request.mock.calls.length;

    rootLeft = 800;
    act(() => {
      viewportWorkAreaStore.getState().publishRegion("toolbar", { x: 0, y: 0, width: 500, height: 40 });
    });
    expect(request).toHaveBeenCalledTimes(initialFrameCount + 1);
    act(() => frames.shift()?.(0));
    expect(viewportWorkAreaStore.getState().obstacles.cornerCluster?.x).toBe(800);
    expect(request).toHaveBeenCalledTimes(initialFrameCount + 1);
    expect(observers).toHaveLength(1);
  });
});
