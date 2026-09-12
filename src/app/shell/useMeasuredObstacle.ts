import { useLayoutEffect } from "react";
import {
  viewportWorkAreaStore,
  type ScreenRect,
  type WorkAreaObstacle,
} from "@/stores/viewportWorkAreaStore";

function relativeRect(element: HTMLElement, viewport: HTMLElement): ScreenRect | null {
  const base = viewport.getBoundingClientRect();
  const rect = element.getBoundingClientRect();
  const left = Math.max(base.left, rect.left);
  const top = Math.max(base.top, rect.top);
  const right = Math.min(base.right, rect.right);
  const bottom = Math.min(base.bottom, rect.bottom);
  return Number.isFinite(left) && Number.isFinite(top) && Number.isFinite(right) && Number.isFinite(bottom)
    && right > left && bottom > top
    ? { x: left - base.left, y: top - base.top, width: right - left, height: bottom - top }
    : null;
}

/** Union root and transformed descendants into viewport-local obstacle bounds. */
export function obstacleUnion(root: HTMLElement, viewport: HTMLElement): ScreenRect | null {
  // ViewCube's CSS-3D face transforms can protrude beyond its layout wrapper.
  const elements = [root, ...root.querySelectorAll<HTMLElement>("[data-obstacle-protrusion]")];
  const rects = elements.map((element) => relativeRect(element, viewport)).filter((rect): rect is ScreenRect => rect !== null);
  if (rects.length === 0) return null;
  const left = Math.min(...rects.map((rect) => rect.x));
  const top = Math.min(...rects.map((rect) => rect.y));
  const right = Math.max(...rects.map((rect) => rect.x + rect.width));
  const bottom = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** Publishes a static viewport-chrome obstacle on resize, never per frame. */
export function useMeasuredObstacle(obstacle: WorkAreaObstacle, root: HTMLElement | null): void {
  useLayoutEffect(() => {
    if (!root) return;
    const viewport = root.parentElement;
    if (!viewport) return;
    let frame = 0;
    const publish = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        viewportWorkAreaStore.getState().publishObstacle(obstacle, obstacleUnion(root, viewport));
      });
    };
    const observer = new ResizeObserver(publish);
    observer.observe(viewport);
    observer.observe(root);
    for (const protrusion of root.querySelectorAll<HTMLElement>("[data-obstacle-protrusion]")) observer.observe(protrusion);
    const unsubscribe = viewportWorkAreaStore.subscribe((state, previous) => {
      // Region updates can move an obstacle without resizing either element.
      if (state.viewport !== previous.viewport || state.regions !== previous.regions) publish();
    });
    publish();
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      unsubscribe();
      viewportWorkAreaStore.getState().publishObstacle(obstacle, null);
    };
  }, [obstacle, root]);
}
