import type { ScreenRect } from "./HtmlOverlayDriver";

export interface AnnotationCandidate {
  id: string;
  priority: number;
  center: { x: number; y: number };
  size: { width: number; height: number } | null;
  pinned: boolean;
  visible: boolean;
}

export type AnnotationPlacement = {
  id: string;
  center: { x: number; y: number } | null;
  status: "visible" | "no-space" | "unknown";
};

const PAD = 6;

function isValidRect(rect: ScreenRect | null): rect is ScreenRect {
  return rect !== null && Number.isFinite(rect.x) && Number.isFinite(rect.y)
    && Number.isFinite(rect.width) && Number.isFinite(rect.height)
    && rect.width > 0 && rect.height > 0;
}

function intersection(a: ScreenRect, b: ScreenRect): ScreenRect | null {
  const left = Math.max(a.x, b.x);
  const top = Math.max(a.y, b.y);
  const right = Math.min(a.x + a.width, b.x + b.width);
  const bottom = Math.min(a.y + a.height, b.y + b.height);
  return right > left && bottom > top ? { x: left, y: top, width: right - left, height: bottom - top } : null;
}

function overlaps(a: ScreenRect, b: ScreenRect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width
    && a.y < b.y + b.height && b.y < a.y + a.height;
}

function expanded(rect: ScreenRect): ScreenRect {
  return { x: rect.x - PAD, y: rect.y - PAD, width: rect.width + PAD * 2, height: rect.height + PAD * 2 };
}

function finiteCenter(center: { x: number; y: number }): boolean {
  return Number.isFinite(center.x) && Number.isFinite(center.y);
}

function finiteSize(size: { width: number; height: number } | null): size is { width: number; height: number } {
  return size !== null && Number.isFinite(size.width) && Number.isFinite(size.height)
    && size.width > 0 && size.height > 0;
}

function rectAt(center: { x: number; y: number }, size: { width: number; height: number }): ScreenRect {
  return { x: center.x - size.width / 2, y: center.y - size.height / 2, width: size.width, height: size.height };
}

function contains(container: ScreenRect, candidate: ScreenRect): boolean {
  return candidate.x >= container.x && candidate.y >= container.y
    && candidate.x + candidate.width <= container.x + container.width
    && candidate.y + candidate.height <= container.y + container.height;
}

function clampCenter(center: { x: number; y: number }, safe: ScreenRect, size: { width: number; height: number }) {
  return {
    x: Math.max(safe.x + size.width / 2, Math.min(safe.x + safe.width - size.width / 2, center.x)),
    y: Math.max(safe.y + size.height / 2, Math.min(safe.y + safe.height - size.height / 2, center.y)),
  };
}

function edgeCandidates(
  desired: { x: number; y: number },
  safe: ScreenRect,
  size: { width: number; height: number },
  occupied: readonly ScreenRect[],
): Array<{ x: number; y: number }> {
  const halfWidth = size.width / 2;
  const halfHeight = size.height / 2;
  const xs = new Set([desired.x, safe.x + halfWidth, safe.x + safe.width - halfWidth]);
  const ys = new Set([desired.y, safe.y + halfHeight, safe.y + safe.height - halfHeight]);
  for (const rect of occupied) {
    xs.add(rect.x - halfWidth - PAD);
    xs.add(rect.x + rect.width + halfWidth + PAD);
    ys.add(rect.y - halfHeight - PAD);
    ys.add(rect.y + rect.height + halfHeight + PAD);
  }
  return [...xs].flatMap((x) => [...ys].map((y) => ({ x, y }))).sort((a, b) => {
    const aDistance = (a.x - desired.x) ** 2 + (a.y - desired.y) ** 2;
    const bDistance = (b.x - desired.x) ** 2 + (b.y - desired.y) ** 2;
    return aDistance - bDistance || a.x - b.x || a.y - b.y;
  });
}

/** Places only opted-in annotations after all protected overlay controls settle. */
export function placeAnnotations(
  annotations: readonly AnnotationCandidate[],
  viewport: ScreenRect,
  safeRect: ScreenRect | null,
  protectedRects: readonly ScreenRect[],
): AnnotationPlacement[] {
  const safe = safeRect === null
    ? null
    : isValidRect(safeRect) && isValidRect(viewport) ? intersection(safeRect, viewport) : null;
  const placed: ScreenRect[] = [...protectedRects.filter(isValidRect)];
  const outcomes = new Map<string, AnnotationPlacement>();
  const ordered = [...annotations].sort((a, b) => b.priority - a.priority || a.id.localeCompare(b.id));

  for (const annotation of ordered) {
    if (!annotation.visible || !finiteCenter(annotation.center) || !finiteSize(annotation.size)) {
      outcomes.set(annotation.id, { id: annotation.id, center: null, status: "unknown" });
      continue;
    }
    if (!safe) {
      outcomes.set(annotation.id, { id: annotation.id, center: null, status: safeRect === null ? "unknown" : "no-space" });
      continue;
    }
    if (annotation.size.width > safe.width || annotation.size.height > safe.height) {
      outcomes.set(annotation.id, { id: annotation.id, center: null, status: "no-space" });
      continue;
    }
    const desired = annotation.pinned ? annotation.center : clampCenter(annotation.center, safe, annotation.size);
    const candidates = annotation.pinned ? [desired] : edgeCandidates(desired, safe, annotation.size, placed);
    const center = candidates.find((candidate) => {
      const rect = rectAt(candidate, annotation.size!);
      return contains(safe, rect) && !placed.some((occupied) => overlaps(rect, expanded(occupied)));
    });
    if (!center) {
      outcomes.set(annotation.id, { id: annotation.id, center: null, status: "no-space" });
      continue;
    }
    placed.push(rectAt(center, annotation.size));
    outcomes.set(annotation.id, { id: annotation.id, center, status: "visible" });
  }
  return annotations.map((annotation) => outcomes.get(annotation.id) ?? {
    id: annotation.id,
    center: null,
    status: "unknown",
  });
}
