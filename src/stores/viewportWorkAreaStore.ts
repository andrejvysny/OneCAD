import { createStore, useStore } from "zustand";

export type WorkAreaRegion = "left" | "right" | "bottom" | "toolbar";
export type WorkAreaObstacle = "cornerCluster" | "navPill";
export interface ScreenRect { x: number; y: number; width: number; height: number }

const OBSTACLE_PAD = 12;

interface ViewportWorkAreaState {
  viewport: ScreenRect;
  regions: Partial<Record<WorkAreaRegion, ScreenRect>>;
  obstacles: Partial<Record<WorkAreaObstacle, ScreenRect>>;
  available: ScreenRect;
  obstacleClearRect: ScreenRect | null;
  setViewport(rect: ScreenRect): void;
  publishRegion(region: WorkAreaRegion, rect: ScreenRect | null): void;
  publishObstacle(obstacle: WorkAreaObstacle, rect: ScreenRect | null): void;
  reset(): void;
}

const EMPTY: ScreenRect = { x: 0, y: 0, width: 0, height: 0 };

function equalRect(a: ScreenRect | undefined, b: ScreenRect | undefined): boolean {
  return a?.x === b?.x && a?.y === b?.y && a?.width === b?.width && a?.height === b?.height;
}

export function deriveAvailableRect(
  viewport: ScreenRect,
  regions: Partial<Record<WorkAreaRegion, ScreenRect>>,
): ScreenRect {
  const left = regions.left ? regions.left.x + regions.left.width : 0;
  const right = regions.right ? viewport.width - regions.right.x : 0;
  const bottom = regions.bottom ? viewport.height - regions.bottom.y : 0;
  return {
    x: left,
    y: 0,
    width: Math.max(0, viewport.width - left - right),
    height: Math.max(0, viewport.height - bottom),
  };
}

function finitePositive(rect: ScreenRect): boolean {
  return Number.isFinite(rect.x) && Number.isFinite(rect.y)
    && Number.isFinite(rect.width) && Number.isFinite(rect.height)
    && rect.width > 0 && rect.height > 0;
}

function clippedObstacle(base: ScreenRect, obstacle: ScreenRect, pad: number): ScreenRect | null {
  if (!finitePositive(base) || !finitePositive(obstacle) || !Number.isFinite(pad) || pad < 0) return null;
  const left = Math.max(base.x, obstacle.x - pad);
  const top = Math.max(base.y, obstacle.y - pad);
  const right = Math.min(base.x + base.width, obstacle.x + obstacle.width + pad);
  const bottom = Math.min(base.y + base.height, obstacle.y + obstacle.height + pad);
  return right > left && bottom > top ? { x: left, y: top, width: right - left, height: bottom - top } : null;
}

function overlapsArea(a: ScreenRect, b: ScreenRect): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x
    && a.y < b.y + b.height && a.y + a.height > b.y;
}

/** Largest axis-aligned base rectangle that has no positive-area overlap with an obstacle. */
export function largestClearRect(
  base: ScreenRect,
  obstacles: readonly ScreenRect[],
  pad = 0,
): ScreenRect | null {
  if (!finitePositive(base) || !Number.isFinite(pad) || pad < 0) return null;
  const clipped = obstacles
    .map((obstacle) => clippedObstacle(base, obstacle, pad))
    .filter((obstacle): obstacle is ScreenRect => obstacle !== null);
  const xs = [...new Set([base.x, base.x + base.width, ...clipped.flatMap((rect) => [rect.x, rect.x + rect.width])])]
    .sort((a, b) => a - b);
  const ys = [...new Set([base.y, base.y + base.height, ...clipped.flatMap((rect) => [rect.y, rect.y + rect.height])])]
    .sort((a, b) => a - b);
  let best: ScreenRect | null = null;
  for (let left = 0; left < xs.length; left += 1) {
    for (let right = left + 1; right < xs.length; right += 1) {
      for (let top = 0; top < ys.length; top += 1) {
        for (let bottom = top + 1; bottom < ys.length; bottom += 1) {
          const candidate = { x: xs[left], y: ys[top], width: xs[right] - xs[left], height: ys[bottom] - ys[top] };
          if (clipped.some((obstacle) => overlapsArea(candidate, obstacle))) continue;
          const area = candidate.width * candidate.height;
          const bestArea = best ? best.width * best.height : -1;
          if (!best || area > bestArea || (area === bestArea && (candidate.y < best.y || (candidate.y === best.y && candidate.x < best.x)))) {
            best = candidate;
          }
        }
      }
    }
  }
  return best;
}

function deriveObstacleClearRect(
  available: ScreenRect,
  regions: Partial<Record<WorkAreaRegion, ScreenRect>>,
  obstacles: Partial<Record<WorkAreaObstacle, ScreenRect>>,
): ScreenRect | null {
  const occupied = [
    ...(regions.toolbar ? [regions.toolbar] : []),
    ...Object.values(obstacles),
  ];
  return largestClearRect(available, occupied, OBSTACLE_PAD);
}

export const viewportWorkAreaStore = createStore<ViewportWorkAreaState>((set, get) => ({
  viewport: EMPTY,
  regions: {},
  obstacles: {},
  available: EMPTY,
  obstacleClearRect: null,
  setViewport(viewport) {
    if (equalRect(get().viewport, viewport)) return;
    set((state) => {
      const available = deriveAvailableRect(viewport, state.regions);
      return { viewport, available, obstacleClearRect: deriveObstacleClearRect(available, state.regions, state.obstacles) };
    });
  },
  publishRegion(region, rect) {
    const previous = get().regions[region];
    if (equalRect(previous, rect ?? undefined)) return;
    set((state) => {
      const regions = { ...state.regions };
      if (rect) regions[region] = rect;
      else delete regions[region];
      const available = deriveAvailableRect(state.viewport, regions);
      return { regions, available, obstacleClearRect: deriveObstacleClearRect(available, regions, state.obstacles) };
    });
  },
  publishObstacle(obstacle, rect) {
    const previous = get().obstacles[obstacle];
    if (equalRect(previous, rect ?? undefined)) return;
    set((state) => {
      const obstacles = { ...state.obstacles };
      if (rect) obstacles[obstacle] = rect;
      else delete obstacles[obstacle];
      return { obstacles, obstacleClearRect: deriveObstacleClearRect(state.available, state.regions, obstacles) };
    });
  },
  reset() {
    set({ viewport: EMPTY, regions: {}, obstacles: {}, available: EMPTY, obstacleClearRect: null });
  },
}));

export function useViewportWorkArea<T>(selector: (state: ViewportWorkAreaState) => T): T {
  return useStore(viewportWorkAreaStore, selector);
}
