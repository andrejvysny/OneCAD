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
 * The side is chosen deterministically (always the axis rotated +90° in screen
 * space) — picking the "nearer" side per frame would make the element jump across
 * the axis mid-drag. When the axis projects to nothing (the camera is looking
 * straight down it) there is no meaningful perpendicular, so it falls back to a
 * fixed up-right offset instead of dividing by ~0.
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
  /**
   * The other end of the axis this element should sit BESIDE. Optional: without
   * it the element is centered on `worldPos`, exactly as before.
   */
  axisFrom?: THREE.Vector3;
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
  offsetPx?: number;
  clusterId?: string;
}

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
    this.items.set(id, {
      el,
      worldPos: worldPos.clone(),
      axisFrom: placement?.axisFrom?.clone(),
      offsetPx: placement?.offsetPx,
      clusterId: placement?.clusterId,
      leaderEl,
    });
  }

  setWorldPos(id: string, worldPos: THREE.Vector3): void {
    const item = this.items.get(id);
    if (item) item.worldPos.copy(worldPos);
  }

  /** Move the axis' other end (no-op for an item registered without one). */
  setAxisFrom(id: string, axisFrom: THREE.Vector3): void {
    const item = this.items.get(id);
    if (item?.axisFrom) item.axisFrom.copy(axisFrom);
  }

  unregister(id: string): void {
    const item = this.items.get(id);
    item?.leaderEl?.remove();
    this.items.delete(id);
  }

  get size(): number {
    return this.items.size;
  }

  /** Project all items and write their transforms. Called once per render. */
  update(camera: THREE.Camera, width: number, height: number): void {
    if (this.items.size === 0) return;
    this.viewProj.multiplyMatrices(
      camera.projectionMatrix,
      camera.matrixWorldInverse,
    );
    interface Placed {
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
    }
    const placed: Placed[] = [];
    for (const { el, worldPos, axisFrom, offsetPx, clusterId, leaderEl } of this.items.values()) {
      const p = projectToScreen(worldPos, this.viewProj, width, height);
      if (!p.visible) {
        placed.push({ el, x: p.x, y: p.y, edge: "", visible: false, clusterId, leaderEl });
        continue;
      }
      let x = p.x;
      let y = p.y;
      let edge = "";
      let anchorX: number | undefined;
      let anchorY: number | undefined;
      if (axisFrom && offsetPx) {
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
        edge = ` translate(${ux * 50}%, ${uy * 50}%)`;
        anchorX = p.x;
        anchorY = p.y;
      }
      placed.push({ el, x, y, edge, visible: true, clusterId, leaderEl, anchorX, anchorY });
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
      if (!it.clusterId) continue;
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
    for (const it of placed) {
      if (!it.visible) {
        it.el.style.display = "none";
        if (it.leaderEl) it.leaderEl.style.display = "none";
        continue;
      }
      it.el.style.display = "";
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
    this.items.clear();
  }
}
