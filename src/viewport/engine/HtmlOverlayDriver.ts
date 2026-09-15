/*
 * HTML overlay driver.
 *
 * A registry of {id, worldPos, el}. Every rendered frame, world positions are
 * projected to screen space and written straight to each element's transform —
 * no React re-render. Elements behind the camera or outside the frustum are
 * hidden. Consumers (dimension inputs, constraint glyphs) land in a later WP;
 * a dev demo label is registered behind ?vpdebug.
 */
import * as THREE from "three";
import { placeAnnotations } from "./annotationPlacement";

export interface ScreenPos {
  x: number;
  y: number;
  /** False when behind the camera or outside the clip volume. */
  visible: boolean;
  /**
   * True when the point is behind the camera. `visible` folds this together with
   * the clip test; a caller that only wants a DIRECTION (an axis' far end, say)
   * still needs to tell the two apart — an off-screen point projects usefully, a
   * point behind the camera projects through a negative w and does not.
   */
  behind: boolean;
}

/**
 * Pure world→screen projection. `viewProj` is projectionMatrix * viewMatrix
 * (camera.matrixWorldInverse). Uses a Vector4 so the sign of w distinguishes
 * points behind the camera from points in front.
 */
export function projectToScreen(
  world: { x: number; y: number; z: number },
  viewProj: THREE.Matrix4,
  width: number,
  height: number,
): ScreenPos {
  const v = new THREE.Vector4(world.x, world.y, world.z, 1).applyMatrix4(viewProj);
  const behind = v.w <= 1e-9;
  const ndcX = v.x / v.w;
  const ndcY = v.y / v.w;
  const x = (ndcX * 0.5 + 0.5) * width;
  const y = (-ndcY * 0.5 + 0.5) * height;
  const inClip = ndcX >= -1 && ndcX <= 1 && ndcY >= -1 && ndcY <= 1;
  return { x, y, visible: !behind && inClip, behind };
}

/**
 * Screen-space offset that puts an element BESIDE the axis `tail → head` rather
 * than on top of it.
 *
 * Returns both the pixel offset AND the unit direction it points, because
 * `offsetPx` is the clearance from the axis to the element's near EDGE, not to
 * its centre: the caller shifts the element by half its own size along `(ux, uy)`
 * so a wide chip cannot reach back over the axis. Measuring the element instead
 * would force a layout read on every rendered frame.
 *
 * The side is chosen deterministically (the axis rotated +90° in screen space,
 * flipped when `offsetPx` is negative) — picking the "nearer" side per frame
 * would make the element jump across the axis mid-drag. `(ux, uy)` is the
 * UNSIGNED +90° direction; `(dx, dy)` already carries the sign. When the axis
 * projects to nothing (the camera is looking straight down it) there is no
 * meaningful perpendicular, so it falls back to a fixed up-right offset instead
 * of dividing by ~0.
 */
export function offsetForAxis(
  head: { x: number; y: number },
  tail: { x: number; y: number },
  offsetPx: number,
): { dx: number; dy: number; ux: number; uy: number } {
  const ax = head.x - tail.x;
  const ay = head.y - tail.y;
  const len = Math.hypot(ax, ay);
  const [ux, uy] = len > 1 ? [ay / len, -ax / len] : [Math.SQRT1_2, -Math.SQRT1_2];
  return { dx: ux * offsetPx, dy: uy * offsetPx, ux, uy };
}

interface OverlayItem {
  worldPos: THREE.Vector3;
  el: HTMLElement;
  /** The OWNER has hidden this item — see {@link HtmlOverlayDriver.setHidden}. */
  hidden?: boolean;
  /**
   * The other end of the axis this element should sit BESIDE. Optional: without
   * it the element is centered on `worldPos`, exactly as before.
   */
  axisFrom?: THREE.Vector3;
  /** SIGNED near-edge clearance in CSS px — see {@link OverlayPlacement}. */
  offsetPx?: number;
  /**
   * Items sharing a cluster id are laid out as ONE group: neighbours are kept a
   * minimum vertical distance apart in screen px, so chips can never overlap
   * no matter how close their world anchors project (a short line, a shared
   * circle anchor). Members keep their own anchors unadjusted whenever those
   * already project far enough apart — the gap is a floor, not a magnet.
   */
  clusterId?: string;
  /**
   * A dashed leader connecting the raw anchor point to the offset chip —
   * created only for items that opt into `axisFrom`/`offsetPx`. Owned by the
   * driver (not the caller) so `mountChip`/`unmountChip` stay one element.
   */
  leaderEl?: HTMLElement;
  /** This item stays clear of the keep-out box `update()` is given. */
  avoidKeepOut?: boolean;
  /** Cached CSS-px size, kept current by a `ResizeObserver` rather than a
   *  per-frame layout read (the driver writes transforms every frame; reading
   *  a rect back would force a synchronous reflow on the drag path). */
  size?: { w: number; h: number };
  sizeObserver?: ResizeObserver;
  /** The side this item was last pushed to — see {@link keepOutShiftY}. */
  keepOutDir?: 1 | -1;
  screenPosition?: { x: number; y: number };
  constrainToSafeRect?: boolean;
  onPlacementStatus?: (status: OverlayPlacementStatus) => void;
  lastPlacementStatus?: OverlayPlacementStatus;
  annotation?: OverlayAnnotationPlacement;
  lastAnnotationStatus?: AnnotationPlacementStatus;
}

/** Below this screen-px length the leader is hidden — an offset chip that
 *  happens to project right back onto its anchor doesn't need a stub line. */
const LEADER_MIN_LEN_PX = 4;

function createLeaderEl(): HTMLElement {
  const el = document.createElement("div");
  el.style.position = "absolute";
  el.style.left = "0";
  el.style.top = "0";
  el.style.height = "0";
  el.style.borderTop = "1px dashed var(--color-border-strong)";
  el.style.transformOrigin = "0 0";
  el.style.pointerEvents = "none";
  el.style.willChange = "transform, width";
  return el;
}

/** Extra placement for an axis-anchored item. */
export interface OverlayPlacement {
  axisFrom?: THREE.Vector3;
  /**
   * Clearance in CSS px from the axis to the element's near EDGE — a SCREEN
   * constant, so it holds at every zoom.
   *
   * SIGNED: positive puts the element on the axis' +90° screen side, negative
   * on the other one. Two families of items anchored to the SAME point can
   * therefore be sent to opposite sides instead of stacking (the sketch's
   * constraint glyphs vs its dimension chips — see badgeLayout.ts).
   */
  offsetPx?: number;
  clusterId?: string;
  /** Opt into the per-frame keep-out region `update()` is given (the value
   *  arrow's screen box). See {@link keepOutShiftY}. */
  avoidKeepOut?: boolean;
  screenPosition?: { x: number; y: number };
  constrainToSafeRect?: boolean;
  onPlacementStatus?: (status: OverlayPlacementStatus) => void;
  annotation?: OverlayAnnotationPlacement;
}

export interface OverlayPlacementStatus {
  width: number;
  height: number;
  fits: boolean | null;
}

export type AnnotationPlacementStatus = "visible" | "no-space" | "unknown";

/** Explicit opt-in for screen annotations that must yield to model controls. */
export interface OverlayAnnotationPlacement {
  priority: number;
  pinned: boolean;
  /**
   * Stay on screen at the anchor when the layout could not place this item.
   *
   * A MEASUREMENT hides instead (its default, false): a measurement label in the
   * wrong place is a wrong reading. Sketch feedback — the snap hint, a live
   * dimension chip, a constraint badge — is transient annotation on geometry the
   * user is looking at, and vanishing is far worse than overlapping: it happens
   * whenever the work-area rect is momentarily unknown, which is every frame
   * before the panels have measured themselves.
   */
  keepWhenUnplaced?: boolean;
  /**
   * Reserve a square of this half-size (CSS px) around this item's ANCHOR that
   * no annotation may cover — including this one's own neighbours.
   *
   * The snap hint declares it, which is how the CURSOR gets protected: the hint
   * is anchored on the snapped point, i.e. where the user is looking and about
   * to click. Without it the collision layout is free to park a chip exactly
   * over the thing being aimed at.
   */
  protectRadiusPx?: number;
  onPlacementStatus?: (status: AnnotationPlacementStatus) => void;
  onScreenPosition?: (position: { x: number; y: number } | null) => void;
  onSizeChanged?: () => void;
}

/** A screen-space box in CSS pixels. */
export interface ScreenRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Clearance kept between a displaced item and the keep-out box, in CSS px. */
export const KEEP_OUT_PAD_PX = 6;

function clampCentre(value: number, start: number, span: number, itemSpan: number): number {
  if (itemSpan >= span) return start + span / 2;
  return Math.max(start + itemSpan / 2, Math.min(start + span - itemSpan / 2, value));
}

function rectsOverlap(a: ScreenRect, b: ScreenRect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width
    && a.y < b.y + b.height && b.y < a.y + a.height;
}

/**
 * The VERTICAL displacement that clears `rect` of `keepOut`, or null when they
 * do not overlap.
 *
 * Why vertical only: the items that opt in are wide, short chips, so a sideways
 * push large enough to clear a 120px-wide arrow box would shove the chip most of
 * a viewport away, while a vertical one moves it by a little over its own height.
 *
 * `sticky` is the direction this item was last pushed. Without it the side is
 * chosen by whichever push is shorter, and an item drifting across the box's
 * centre would flip sides mid-drag — the chip teleporting over the arrow while
 * the user is dragging it. Once pushed, an item keeps its side until it stops
 * overlapping at all.
 */
export function keepOutShiftY(
  rect: ScreenRect,
  keepOut: ScreenRect,
  pad: number,
  sticky?: 1 | -1,
): { dy: number; dir: 1 | -1 } | null {
  const overlapsX = rect.x < keepOut.x + keepOut.width && keepOut.x < rect.x + rect.width;
  const overlapsY = rect.y < keepOut.y + keepOut.height && keepOut.y < rect.y + rect.height;
  if (!overlapsX || !overlapsY) return null;
  // Up: the item's BOTTOM edge lands `pad` above the box's top. Down: its TOP
  // edge lands `pad` below the box's bottom. Both are exact, not iterative.
  const up = keepOut.y - pad - (rect.y + rect.height);
  const down = keepOut.y + keepOut.height + pad - rect.y;
  const dir: 1 | -1 = sticky ?? (Math.abs(up) <= Math.abs(down) ? -1 : 1);
  return { dy: dir === -1 ? up : down, dir };
}

/**
 * Who yields to whom when screen annotations collide (UX review S9).
 *
 * Highest number is placed FIRST and keeps its spot; everything below it is
 * pushed to the nearest free position. The order is "what is the user doing
 * right now": the snap hint answers the click about to happen, a live dimension
 * chip the gesture in progress, a selection label a passive measurement, and a
 * constraint badge a fact about geometry that is not going anywhere.
 *
 * A measurement annotation (`MeasureOverlay`) sits above all of them at its own
 * per-slot priority — it is a reading the user explicitly asked for.
 */
export const ANNOTATION_PRIORITY = {
  snapHint: 4,
  liveDimChip: 3,
  dimensionLabel: 2,
  constraintBadge: 1,
} as const;

/** Half-size (CSS px) of the square the snap hint reserves around the cursor. */
export const CURSOR_PROTECT_PX = 24;

/** Default screen gap between neighbours within an overlay cluster. Deliberately
 *  larger than the worst chip height so a cluster reads as one stacked set. */
export const CLUSTER_GAP_PX = 24;

export class HtmlOverlayDriver {
  private readonly items = new Map<string, OverlayItem>();
  private readonly viewProj = new THREE.Matrix4();

  register(id: string, el: HTMLElement, worldPos: THREE.Vector3, placement?: OverlayPlacement): void {
    el.style.position = "absolute";
    el.style.left = "0";
    el.style.top = "0";
    el.style.willChange = "transform";
    let leaderEl: HTMLElement | undefined;
    if (placement?.axisFrom && el.parentElement) {
      leaderEl = createLeaderEl();
      el.parentElement.insertBefore(leaderEl, el);
    }
    const item: OverlayItem = {
      el,
      worldPos: worldPos.clone(),
      axisFrom: placement?.axisFrom?.clone(),
      offsetPx: placement?.offsetPx,
      clusterId: placement?.clusterId,
      leaderEl,
      avoidKeepOut: placement?.avoidKeepOut,
      screenPosition: placement?.screenPosition,
      constrainToSafeRect: placement?.constrainToSafeRect,
      onPlacementStatus: placement?.onPlacementStatus,
      annotation: placement?.annotation,
    };
    if (placement?.avoidKeepOut || placement?.constrainToSafeRect || placement?.annotation) {
      const rect = el.getBoundingClientRect();
      item.size = { w: rect.width, h: rect.height };
      if (typeof ResizeObserver !== "undefined") {
        item.sizeObserver = new ResizeObserver((entries) => {
          const box = entries[0]?.contentRect;
          if (!box || box.width <= 0 || box.height <= 0) return;
          item.size = { w: box.width, h: box.height };
          item.lastPlacementStatus = undefined;
          item.onPlacementStatus?.({ width: box.width, height: box.height, fits: null });
          item.annotation?.onSizeChanged?.();
        });
        item.sizeObserver.observe(el);
      }
    }
    this.items.set(id, item);
  }

  setWorldPos(id: string, worldPos: THREE.Vector3): void {
    const item = this.items.get(id);
    if (item) item.worldPos.copy(worldPos);
  }

  /**
   * Hide or show a registered item, from its OWNER.
   *
   * The driver writes `display` on every registered element once per frame, so
   * an owner that hid its own element with `style.display = "none"` had it
   * un-hidden on the very next render — which is how the snap hint chip and the
   * auto-constraint ghost went on advertising a decision that had already been
   * cleared (UX review S4: "the snap badge is stale and sticky", surviving a
   * commit and an Escape). Hiding is therefore a driver fact, not a style the
   * owner writes behind its back.
   */
  setHidden(id: string, hidden: boolean): void {
    const item = this.items.get(id);
    if (item) item.hidden = hidden;
  }

  /** Move the axis' other end (no-op for an item registered without one). */
  setAxisFrom(id: string, axisFrom: THREE.Vector3): void {
    const item = this.items.get(id);
    if (item?.axisFrom) item.axisFrom.copy(axisFrom);
  }

  setScreenPosition(id: string, position: { x: number; y: number } | null): void {
    const item = this.items.get(id);
    if (item) item.screenPosition = position ?? undefined;
  }

  unregister(id: string): void {
    const item = this.items.get(id);
    item?.leaderEl?.remove();
    item?.sizeObserver?.disconnect();
    this.items.delete(id);
  }

  get size(): number {
    return this.items.size;
  }

  /**
   * Project all items and write their transforms. Called once per render.
   *
   * `keepOut` is a screen box no opted-in item may sit on — in practice the
   * value arrow's projected box, which a chip centred on the same anchor would
   * otherwise cover completely. It is passed per frame rather than registered
   * because it moves with the camera and the armed tool.
   */
  update(
    camera: THREE.Camera,
    width: number,
    height: number,
    keepOut?: ScreenRect | null,
    safeRect?: ScreenRect | null,
  ): void {
    if (this.items.size === 0) return;
    this.viewProj.multiplyMatrices(
      camera.projectionMatrix,
      camera.matrixWorldInverse,
    );
    interface Placed {
      id: string;
      el: HTMLElement;
      x: number;
      y: number;
      edge: string;
      visible: boolean;
      clusterId?: string;
      leaderEl?: HTMLElement;
      /** Raw (pre-offset) anchor projection — the leader's other endpoint. */
      anchorX?: number;
      anchorY?: number;
      /** Set for items that opted into the keep-out, so the pass below can find
       *  them with their own size and sticky side. */
      item?: OverlayItem;
      /** SIGNED unit offset direction, needed to turn (x, y) into the element's
       *  real centre — the `edge` translate shifts it by half its own size. */
      ux?: number;
      uy?: number;
    }
    const placed: Placed[] = [];
    for (const [id, item] of this.items) {
      const { el, worldPos, axisFrom, offsetPx, clusterId, leaderEl } = item;
      const projected = projectToScreen(worldPos, this.viewProj, width, height);
      const p = item.screenPosition
        ? { ...item.screenPosition, visible: true, behind: false }
        : projected;
      if (!p.visible || item.hidden) {
        placed.push({
          id,
          el,
          x: p.x,
          y: p.y,
          edge: "",
          visible: false,
          clusterId,
          leaderEl,
          item: item.onPlacementStatus || item.annotation ? item : undefined,
          ux: 0,
          uy: 0,
        });
        continue;
      }
      let x = p.x;
      let y = p.y;
      let edge = "";
      let anchorX: number | undefined;
      let anchorY: number | undefined;
      let offsetUx = 0;
      let offsetUy = 0;
      if (item.screenPosition) {
        if (leaderEl && projected.visible) {
          anchorX = projected.x;
          anchorY = projected.y;
        }
      } else if (axisFrom && offsetPx) {
        // The tail only supplies a DIRECTION, so an off-screen tail is still
        // usable — but a tail BEHIND the camera projects through a negative w and
        // its x/y are meaningless, so that one falls back to the fixed offset.
        const t = projectToScreen(axisFrom, this.viewProj, width, height);
        const { dx, dy, ux, uy } = offsetForAxis(p, t.behind ? p : t, offsetPx);
        x += dx;
        y += dy;
        // Half the element's OWN size, in the same direction: `offsetPx` is then
        // the clearance to its near edge, whatever its width. Percentages resolve
        // against the element's border box, and translations compose additively.
        // The sign has to travel with it — for a negative `offsetPx` the element
        // sits on the other side, so half its size must move it FURTHER from the
        // axis there too, not back across it.
        const side = offsetPx < 0 ? -1 : 1;
        edge = ` translate(${ux * side * 50}%, ${uy * side * 50}%)`;
        anchorX = p.x;
        anchorY = p.y;
        offsetUx = ux * side;
        offsetUy = uy * side;
      }
      placed.push({
        id,
        el,
        x,
        y,
        edge,
        visible: true,
        clusterId,
        leaderEl,
        anchorX,
        anchorY,
        item: item.avoidKeepOut || item.constrainToSafeRect || item.annotation ? item : undefined,
        ux: offsetUx,
        uy: offsetUy,
      });
    }
    // Screen-space cluster resolution: keep cluster neighbours at least
    // `CLUSTER_GAP_PX` apart along screen +y (downward), propagating the push
    // down a long cluster. Invisible members are skipped — a chip alone behind
    // the camera has no neighbour to push or be pushed by.
    //
    // The push-down loop assumes its group is already ordered top-to-bottom —
    // it only ever pushes a member DOWN to clear the one before it. Grouping
    // by `this.items`' Map iteration order (registration order) instead of
    // actual projected y breaks that assumption: an item registered earlier
    // but now projecting BELOW a later-registered sibling would still be
    // treated as "first" and never get pushed, while the sibling above it
    // gets shoved further down. Sorting by y first (stable, so exact ties keep
    // registration order) makes the group's order match the invariant the
    // push-down math assumes.
    const clusters = new Map<string, Placed[]>();
    for (const it of placed) {
      if (!it.clusterId || it.item?.annotation) continue;
      const group = clusters.get(it.clusterId);
      if (group) group.push(it);
      else clusters.set(it.clusterId, [it]);
    }
    for (const group of clusters.values()) {
      group.sort((a, b) => a.y - b.y);
      let prevY: number | null = null;
      for (const it of group) {
        if (!it.visible) continue;
        if (prevY !== null) it.y = Math.max(it.y, prevY + CLUSTER_GAP_PX);
        prevY = it.y;
      }
    }
    // Keep-out: an opted-in item is pushed clear of `keepOut` LAST, so a cluster
    // push cannot put it back on top of the arrow. An item with no measured size
    // is left alone rather than displaced by a guess.
    for (const it of placed) {
      const item = it.item;
      if (!item || item.annotation) continue;
      if (!it.visible || !keepOut || !item.size || item.size.w <= 0 || item.size.h <= 0) {
        item.keepOutDir = undefined;
        continue;
      }
      const { w, h } = item.size;
      const cx = it.x + (it.ux ?? 0) * (w / 2);
      const cy = it.y + (it.uy ?? 0) * (h / 2);
      const shift = keepOutShiftY(
        { x: cx - w / 2, y: cy - h / 2, width: w, height: h },
        keepOut,
        KEEP_OUT_PAD_PX,
        item.keepOutDir,
      );
      if (!shift) {
        item.keepOutDir = undefined;
        continue;
      }
      it.y += shift.dy;
      item.keepOutDir = shift.dir;
    }
    for (const it of placed) {
      const item = it.item;
      if (!item?.constrainToSafeRect || item.annotation || !safeRect || !item.size) continue;
      const halfW = item.size.w / 2;
      const halfH = item.size.h / 2;
      const centreX = it.x + (it.ux ?? 0) * halfW;
      const centreY = it.y + (it.uy ?? 0) * halfH;
      it.x += clampCentre(centreX, safeRect.x, safeRect.width, item.size.w) - centreX;
      it.y += clampCentre(centreY, safeRect.y, safeRect.height, item.size.h) - centreY;
    }
    for (const it of placed) {
      const item = it.item;
      if (!item?.avoidKeepOut || item.annotation || !keepOut || !safeRect || !item.size) continue;
      const { w, h } = item.size;
      const centreX = it.x + (it.ux ?? 0) * (w / 2);
      const centreY = it.y + (it.uy ?? 0) * (h / 2);
      const current = { x: centreX - w / 2, y: centreY - h / 2, width: w, height: h };
      if (!rectsOverlap(current, keepOut)) continue;
      for (const dir of [-1, 1] as const) {
        const shift = keepOutShiftY(current, keepOut, KEEP_OUT_PAD_PX, dir);
        if (!shift) break;
        const wanted = centreY + shift.dy;
        const nextCentreY = clampCentre(wanted, safeRect.y, safeRect.height, h);
        const candidate = { ...current, y: nextCentreY - h / 2 };
        if (rectsOverlap(candidate, keepOut)) continue;
        it.y += nextCentreY - centreY;
        item.keepOutDir = dir;
        break;
      }
    }
    const annotationItems = placed.filter((it) => it.item?.annotation);
    if (annotationItems.length > 0) {
      const protectedRects: ScreenRect[] = keepOut ? [keepOut] : [];
      for (const it of placed) {
        const item = it.item;
        if (!item || item.annotation || !item.constrainToSafeRect || !item.size || !it.visible) continue;
        const centerX = it.x + (it.ux ?? 0) * (item.size.w / 2);
        const centerY = it.y + (it.uy ?? 0) * (item.size.h / 2);
        protectedRects.push({ x: centerX - item.size.w / 2, y: centerY - item.size.h / 2, width: item.size.w, height: item.size.h });
      }
      // Anchor reservations (the cursor vicinity): a square around the ANCHOR,
      // not around the placed element — the point being protected is where the
      // user is pointing, which is `anchorX/anchorY` when the item carries an
      // axis and its own projected position otherwise.
      for (const it of annotationItems) {
        const r = it.item!.annotation!.protectRadiusPx;
        if (!it.visible || !r || !(r > 0)) continue;
        const ax = it.anchorX ?? it.x;
        const ay = it.anchorY ?? it.y;
        protectedRects.push({ x: ax - r, y: ay - r, width: r * 2, height: r * 2 });
      }
      // Candidate centres are the element's REAL centres — `it.x/it.y` is the
      // axis attachment point, and the `edge` transform then shifts the element
      // by half its own size. Feeding the unshifted point in (and writing the
      // result back the same way) is what keeps an axis-anchored chip's standoff
      // intact through the collision pass.
      const halfOffset = (it: Placed): { dx: number; dy: number } => {
        const size = it.item?.size;
        if (!size) return { dx: 0, dy: 0 };
        return { dx: (it.ux ?? 0) * (size.w / 2), dy: (it.uy ?? 0) * (size.h / 2) };
      };
      const outcomes = placeAnnotations(
        annotationItems.map((it) => {
          const { dx, dy } = halfOffset(it);
          return {
            id: it.id,
            priority: it.item!.annotation!.priority,
            center: { x: it.x + dx, y: it.y + dy },
            size: it.item!.size ? { width: it.item!.size.w, height: it.item!.size.h } : null,
            pinned: it.item!.annotation!.pinned,
            visible: it.visible,
          };
        }),
        { x: 0, y: 0, width, height },
        safeRect ?? null,
        protectedRects,
      );
      const byId = new Map(outcomes.map((outcome) => [outcome.id, outcome]));
      for (const it of annotationItems) {
        const item = it.item!;
        const annotation = item.annotation!;
        const outcome = byId.get(it.id);
        const status = outcome?.status ?? "unknown";
        it.visible = status === "visible" || (annotation.keepWhenUnplaced === true && it.visible);
        if (outcome?.center) {
          const { dx, dy } = halfOffset(it);
          it.x = outcome.center.x - dx;
          it.y = outcome.center.y - dy;
        }
        annotation.onScreenPosition?.(outcome?.center ?? null);
        if (item.lastAnnotationStatus === status) continue;
        item.lastAnnotationStatus = status;
        annotation.onPlacementStatus?.(status);
      }
    }
    for (const it of placed) {
      const item = it.item;
      if (!item?.onPlacementStatus || !item.size || item.size.w <= 0 || item.size.h <= 0) continue;
      const { w, h } = item.size;
      let fits: boolean | null = null;
      if (safeRect) {
        const centreX = it.x + (it.ux ?? 0) * (w / 2);
        const centreY = it.y + (it.uy ?? 0) * (h / 2);
        const rect = { x: centreX - w / 2, y: centreY - h / 2, width: w, height: h };
        const fitsSafe = it.visible && w <= safeRect.width && h <= safeRect.height
          && rect.x >= safeRect.x && rect.y >= safeRect.y
          && rect.x + rect.width <= safeRect.x + safeRect.width
          && rect.y + rect.height <= safeRect.y + safeRect.height;
        const fitsKeepOut = !item.avoidKeepOut || !keepOut || !rectsOverlap(rect, keepOut);
        fits = fitsSafe && fitsKeepOut;
      }
      const status = { width: w, height: h, fits };
      const previous = item.lastPlacementStatus;
      if (previous?.width === status.width && previous.height === status.height && previous.fits === status.fits) continue;
      item.lastPlacementStatus = status;
      item.onPlacementStatus(status);
    }
    for (const it of placed) {
      if (!it.visible) {
        // An OWNER-hidden item is gone, not merely unplaced: `display: none`,
        // whatever its placement mode. `visibility: hidden` is reserved for an
        // annotation the LAYOUT could not fit, which keeps its box measurable.
        if (it.item?.annotation && !this.items.get(it.id)?.hidden) {
          it.el.style.display = "";
          it.el.style.visibility = "hidden";
        } else {
          it.el.style.display = "none";
        }
        if (it.leaderEl) it.leaderEl.style.display = "none";
        continue;
      }
      it.el.style.display = "";
      it.el.style.visibility = "";
      it.el.style.transform = `translate(-50%, -50%) translate(${it.x}px, ${it.y}px)${it.edge}`;
      if (it.leaderEl && it.anchorX !== undefined && it.anchorY !== undefined) {
        const ldx = it.x - it.anchorX;
        const ldy = it.y - it.anchorY;
        const len = Math.hypot(ldx, ldy);
        if (len < LEADER_MIN_LEN_PX) {
          it.leaderEl.style.display = "none";
        } else {
          it.leaderEl.style.display = "";
          it.leaderEl.style.width = `${len}px`;
          const angleDeg = (Math.atan2(ldy, ldx) * 180) / Math.PI;
          it.leaderEl.style.transform = `translate(${it.anchorX}px, ${it.anchorY}px) rotate(${angleDeg}deg)`;
        }
      }
    }
  }

  clear(): void {
    for (const item of this.items.values()) {
      item.leaderEl?.remove();
      item.sizeObserver?.disconnect();
    }
    this.items.clear();
  }
}
