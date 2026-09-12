import { useLayoutEffect, useRef, type ReactNode } from "react";
import {
  viewportWorkAreaStore,
  type ScreenRect,
  type WorkAreaRegion,
} from "@/stores/viewportWorkAreaStore";

export function relativeUnion(root: HTMLElement, viewport: HTMLElement): ScreenRect | null {
  const base = viewport.getBoundingClientRect();
  const rects = [...root.children]
    .filter((child): child is HTMLElement => child instanceof HTMLElement && !child.hidden)
    .map((child) => child.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0);
  if (rects.length === 0) return null;
  const left = Math.max(base.left, Math.min(...rects.map((rect) => rect.left)));
  const top = Math.max(base.top, Math.min(...rects.map((rect) => rect.top)));
  const right = Math.min(base.right, Math.max(...rects.map((rect) => rect.right)));
  const bottom = Math.min(base.bottom, Math.max(...rects.map((rect) => rect.bottom)));
  if (right <= left || bottom <= top) return null;
  return { x: left - base.left, y: top - base.top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
}

export function observeRegionRoots(
  resize: Pick<ResizeObserver, "disconnect" | "observe">,
  root: HTMLElement,
  viewport: HTMLElement,
): void {
  resize.disconnect();
  resize.observe(viewport);
  for (const child of root.children) {
    if (child instanceof HTMLElement) resize.observe(child);
  }
}

export function MeasuredShellRegion({
  region,
  viewport,
  children,
}: {
  region: WorkAreaRegion;
  viewport: HTMLDivElement | null;
  children: ReactNode;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || !viewport) return;
    let frame = 0;
    const publish = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const base = viewport.getBoundingClientRect();
        viewportWorkAreaStore.getState().setViewport({ x: 0, y: 0, width: base.width, height: base.height });
        viewportWorkAreaStore.getState().publishRegion(region, relativeUnion(root, viewport));
      });
    };
    const resize = new ResizeObserver(publish);
    const observeRoots = () => {
      observeRegionRoots(resize, root, viewport);
      publish();
    };
    const mutation = new MutationObserver(observeRoots);
    mutation.observe(root, { childList: true });
    observeRoots();
    return () => {
      cancelAnimationFrame(frame);
      mutation.disconnect();
      resize.disconnect();
      viewportWorkAreaStore.getState().publishRegion(region, null);
    };
  }, [region, viewport]);
  return <div ref={rootRef} data-shell-region={region} className="contents">{children}</div>;
}
