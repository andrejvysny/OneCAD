import { createElement, StrictMode, useState } from "react";
import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { viewportWorkAreaStore } from "@/stores/viewportWorkAreaStore";
import { observeRegionRoots, relativeUnion } from "./MeasuredShellRegion";
import { MeasuredShellRegion } from "./MeasuredShellRegion";

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height } as DOMRect;
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  viewportWorkAreaStore.getState().reset();
});

describe("relativeUnion", () => {
  it("unions multiple contribution roots in viewport-local coordinates", () => {
    const viewport = document.createElement("div");
    const root = document.createElement("div");
    const first = document.createElement("aside");
    const second = document.createElement("aside");
    root.append(first, second);
    vi.spyOn(viewport, "getBoundingClientRect").mockReturnValue(rect(100, 50, 800, 600));
    vi.spyOn(first, "getBoundingClientRect").mockReturnValue(rect(112, 62, 220, 500));
    vi.spyOn(second, "getBoundingClientRect").mockReturnValue(rect(112, 570, 220, 30));

    expect(relativeUnion(root, viewport)).toEqual({ x: 12, y: 12, width: 220, height: 538 });
  });

  it("ignores hidden and zero-size roots and clips to the viewport", () => {
    const viewport = document.createElement("div");
    const root = document.createElement("div");
    const visible = document.createElement("div");
    const hidden = document.createElement("div");
    hidden.hidden = true;
    root.append(visible, hidden);
    vi.spyOn(viewport, "getBoundingClientRect").mockReturnValue(rect(10, 20, 300, 200));
    vi.spyOn(visible, "getBoundingClientRect").mockReturnValue(rect(0, 0, 50, 80));
    vi.spyOn(hidden, "getBoundingClientRect").mockReturnValue(rect(10, 20, 300, 200));

    expect(relativeUnion(root, viewport)).toEqual({ x: 0, y: 0, width: 40, height: 60 });
  });

  it("returns no region for a root wholly outside the viewport", () => {
    const viewport = document.createElement("div");
    const root = document.createElement("div");
    const outside = document.createElement("aside");
    root.append(outside);
    vi.spyOn(viewport, "getBoundingClientRect").mockReturnValue(rect(100, 50, 800, 600));
    vi.spyOn(outside, "getBoundingClientRect").mockReturnValue(rect(920, 80, 200, 500));

    expect(relativeUnion(root, viewport)).toBeNull();
  });
});

describe("MeasuredShellRegion", () => {
  it("starts observing after the parent callback ref resolves and cleans up in StrictMode", () => {
    const observers: ResizeObserverMock[] = [];
    class ResizeObserverMock {
      readonly observe = vi.fn();
      readonly disconnect = vi.fn();
      constructor(_callback: ResizeObserverCallback) {
        observers.push(this);
      }
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.dataset.viewport !== undefined) return rect(100, 50, 1024, 700);
      if (this.dataset.panel !== undefined) return rect(804, 50, 320, 666);
      return rect(0, 0, 0, 0);
    });

    function Harness() {
      const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
      return createElement(
        "div",
        { "data-viewport": true, ref: setViewport },
        createElement(
          MeasuredShellRegion,
          {
            region: "right",
            viewport,
            children: createElement("aside", { "data-panel": true }),
          },
        ),
      );
    }

    const mounted = render(createElement(StrictMode, null, createElement(Harness)));

    expect(viewportWorkAreaStore.getState().viewport).toEqual({ x: 0, y: 0, width: 1024, height: 700 });
    expect(viewportWorkAreaStore.getState().regions.right).toEqual({ x: 704, y: 0, width: 320, height: 666 });
    expect(observers[observers.length - 1]?.observe).toHaveBeenCalledTimes(2);

    mounted.unmount();
    expect(viewportWorkAreaStore.getState().regions.right).toBeUndefined();
    expect(observers.every((observer) => observer.disconnect.mock.calls.length > 0)).toBe(true);
  });
});

describe("observeRegionRoots", () => {
  it("observes the viewport and direct contribution roots only", () => {
    const viewport = document.createElement("div");
    const root = document.createElement("div");
    const contribution = document.createElement("aside");
    const nestedInput = document.createElement("input");
    contribution.append(nestedInput);
    root.append(contribution);
    const observe = vi.fn();
    const disconnect = vi.fn();

    observeRegionRoots({ observe, disconnect }, root, viewport);

    expect(disconnect).toHaveBeenCalledOnce();
    expect(observe).toHaveBeenCalledTimes(2);
    expect(observe).toHaveBeenNthCalledWith(1, viewport);
    expect(observe).toHaveBeenNthCalledWith(2, contribution);
    expect(observe).not.toHaveBeenCalledWith(nestedInput);
  });
});
