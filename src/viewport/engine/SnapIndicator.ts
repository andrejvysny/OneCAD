/*
 * SnapIndicator — the in-canvas snap feedback (F-WP6, hardened in SNAP P4).
 *
 * Renders, in `interactionRoot` (plane-local under the plane basis):
 *   - a per-KIND marker glyph at the snapped point (constant CSS size),
 *   - dashed guide lines from their reference point to the snapped point,
 * plus a hint chip ("Endpoint" / "Vertical · Horizontal" / …) through the
 * HtmlOverlayDriver, honoring `settingsStore.show.snappingHints`.
 *
 * ── Why the guides are `Line2`, not `LineDashedMaterial` (SNAP §10.1) ────────
 * `LineDashedMaterial` measures `dashSize`/`gapSize` in WORLD units, and the
 * old code set them to the constants 6 and 4. At any zoom other than the one
 * those numbers happened to suit, the guide was either a solid line or a few
 * enormous strokes — and the line itself was 1 *device* px, so it thinned to
 * invisibility at 2× DPR. `Line2` gives a device-pixel width, and the dash/gap
 * are recomputed per guide from the current plane→screen scale, so a guide
 * reads as 6px on / 4px off at every zoom.
 *
 * The chip is styled from design tokens through `var(--color-*)` (no raw hex —
 * the tokens-only gate), positioned each frame by the overlay driver.
 */
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import { LineGeometry } from "three/examples/jsm/lines/LineGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { SketchPlane } from "@/ipc/types";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import type { SnapDecision, SnapKind } from "@/tools/sketch/snapTypes";
import { cssToDevice, currentDpr } from "./dpr";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { planeBasisMatrix, planePointToWorld } from "./sketchBasis";

const HINT_ID = "__sketch_snap_hint";

/** Guide stroke, in CSS px. */
const GUIDE_WIDTH_CSS = 1;
/** Dash cadence, in CSS px — the pair the ±1px rendering check pins. */
export const GUIDE_DASH_CSS = 6;
export const GUIDE_GAP_CSS = 4;
/** Marker glyph size, in CSS px. */
const MARKER_SIZE_CSS = 11;

interface SnapIndicatorDeps {
  interactionRoot: THREE.Object3D;
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
  invalidate: () => void;
}

/**
 * Marker glyph shapes — ONE PER SNAP FAMILY (SNAP §10.2).
 *
 * The marker is the only feedback a user gets when snapping hints are off, so
 * endpoint and midpoint rendering as the same dot left the two
 * indistinguishable at the moment of the click. Every kind that can win now has
 * its own silhouette.
 */
export type MarkerGlyph =
  | "square"
  | "triangle"
  | "ringDot"
  | "target"
  | "diamond"
  | "cross"
  | "ring"
  | "plus"
  | "dot";

export function glyphFor(kind: SnapKind): MarkerGlyph {
  switch (kind) {
    case "endpoint":
      return "square";
    case "midpoint":
      return "triangle";
    case "center":
      return "ringDot";
    case "origin":
      return "target";
    case "quadrant":
      return "diamond";
    case "intersection":
      return "cross";
    case "onCurve":
      return "ring";
    case "grid":
      return "plus";
    default:
      // Polar / alignment guides: a small dot, because the GUIDE LINE is the
      // feedback there and a loud glyph would compete with it.
      return "dot";
  }
}

/** Render a 32×32 glyph sprite (white on transparent; tinted by the material). */
function makeGlyphTexture(glyph: MarkerGlyph): THREE.Texture | null {
  const size = 32;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null; // jsdom / no 2D context — fall back to a plain point
  // White mask — the PointsMaterial tints it with the (token-derived) marker
  // color; the CSS keyword keeps the tokens-only hex gate clean.
  ctx.strokeStyle = "white";
  ctx.fillStyle = "white";
  ctx.lineWidth = 3;
  const c = size / 2;
  const r = 9;
  ctx.beginPath();
  switch (glyph) {
    case "dot":
      ctx.arc(c, c, r * 0.7, 0, Math.PI * 2);
      ctx.fill();
      break;
    case "square":
      // FILLED: an endpoint is the strongest target on screen and should read
      // as solid next to the hollow derived ones.
      ctx.rect(c - r * 0.75, c - r * 0.75, r * 1.5, r * 1.5);
      ctx.fill();
      break;
    case "triangle":
      ctx.moveTo(c, c - r);
      ctx.lineTo(c + r * 0.9, c + r * 0.7);
      ctx.lineTo(c - r * 0.9, c + r * 0.7);
      ctx.closePath();
      ctx.stroke();
      break;
    case "ringDot":
      ctx.arc(c, c, r, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(c, c, r * 0.28, 0, Math.PI * 2);
      ctx.fill();
      break;
    case "target":
      // Crosshair through a ring — the origin is the one point with a fixed
      // meaning, and it earns the most distinct silhouette.
      ctx.arc(c, c, r * 0.72, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(c - r, c);
      ctx.lineTo(c + r, c);
      ctx.moveTo(c, c - r);
      ctx.lineTo(c, c + r);
      ctx.stroke();
      break;
    case "diamond":
      ctx.moveTo(c, c - r);
      ctx.lineTo(c + r, c);
      ctx.lineTo(c, c + r);
      ctx.lineTo(c - r, c);
      ctx.closePath();
      ctx.stroke();
      break;
    case "cross":
      ctx.moveTo(c - r, c - r);
      ctx.lineTo(c + r, c + r);
      ctx.moveTo(c + r, c - r);
      ctx.lineTo(c - r, c + r);
      ctx.stroke();
      break;
    case "ring":
      ctx.arc(c, c, r, 0, Math.PI * 2);
      ctx.stroke();
      break;
    case "plus":
      // Deliberately SMALLER than the origin's crosshair: a grid node is the
      // weakest target offered and must not out-shout a named point.
      ctx.moveTo(c - r * 0.65, c);
      ctx.lineTo(c + r * 0.65, c);
      ctx.moveTo(c, c - r * 0.65);
      ctx.lineTo(c, c + r * 0.65);
      ctx.stroke();
      break;
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.needsUpdate = true;
  return tex;
}

/** One rendered guide segment: a fat dashed line with its own dash scale. */
interface GuideLineObject {
  line: Line2;
  material: LineMaterial;
  geometry: LineGeometry;
}

export class SnapIndicator {
  private readonly group = new THREE.Group();
  private readonly marker: THREE.Points;
  private readonly markerMat: THREE.PointsMaterial;
  private readonly guideGroup = new THREE.Group();
  private readonly guideObjects: GuideLineObject[] = [];
  private readonly hintEl: HTMLElement;
  private readonly _basis = new THREE.Matrix4();
  private readonly glyphTextures: Partial<Record<MarkerGlyph, THREE.Texture | null>> = {};
  private currentGlyph: MarkerGlyph | null = null;
  private plane: SketchPlane | null = null;
  private hintRegistered = false;
  private dpr = currentDpr();
  private readonly resolution = new THREE.Vector2(1, 1);
  /** CSS px per plane unit, supplied by the engine each frame. 1 is the inert
   *  fallback (no camera metric yet), which keeps dashes at their world size. */
  private pxPerUnit = 1;

  constructor(private readonly deps: SnapIndicatorDeps) {
    this.group.name = "snapIndicator";
    this.group.visible = false;
    this.group.matrixAutoUpdate = false;
    deps.interactionRoot.add(this.group);

    this.markerMat = new THREE.PointsMaterial({
      color: palette.sketchUnder(),
      size: MARKER_SIZE_CSS,
      sizeAttenuation: false,
      depthTest: false,
      transparent: true,
      alphaTest: 0.4,
      toneMapped: false,
    });
    this.marker = new THREE.Points(
      new THREE.BufferGeometry().setAttribute("position", new THREE.Float32BufferAttribute([0, 0, 0], 3)),
      this.markerMat,
    );
    this.marker.renderOrder = RENDER_ORDER.SNAP_MARKER;
    this.group.add(this.marker);
    this.setGlyph("dot");

    this.guideGroup.name = "snapGuides";
    this.group.add(this.guideGroup);

    this.hintEl = document.createElement("div");
    this.hintEl.dataset.sketchSnapHint = "1";
    this.hintEl.style.font = "500 10.5px var(--font-ui)";
    this.hintEl.style.padding = "2px 6px";
    this.hintEl.style.borderRadius = "4px";
    this.hintEl.style.whiteSpace = "nowrap";
    this.hintEl.style.pointerEvents = "none";
    // Translucent scrim, not a solid tooltip box (Sketcher UX cleanup, Track
    // B4) — this answers "what will my click do here?", it isn't floating
    // app UI, and a solid dark chip read as the latter. Same color-mix()
    // technique as SketchController's drag-preview fill.
    this.hintEl.style.background = "color-mix(in srgb, var(--color-tooltip) 55%, transparent)";
    this.hintEl.style.color = "var(--color-tooltip-text)";
    this.hintEl.style.display = "none";
    deps.overlayEl.appendChild(this.hintEl);
  }

  setPlane(plane: SketchPlane): void {
    this.plane = plane;
    planeBasisMatrix(plane, this._basis);
    this.group.matrix.copy(this._basis);
    this.group.matrixWorldNeedsUpdate = true;
  }

  /**
   * Per-frame: the fat-line resolution (DEVICE px), the capped DPR the guide
   * widths are converted for, and the plane→screen scale the dash cadence is
   * measured in.
   */
  update(
    deviceWidth: number,
    deviceHeight: number,
    dpr: number = currentDpr(),
    pxPerUnit = 0,
  ): void {
    this.resolution.set(deviceWidth, deviceHeight);
    for (const g of this.guideObjects) g.material.resolution.copy(this.resolution);
    if (Number.isFinite(pxPerUnit) && pxPerUnit > 0) this.pxPerUnit = pxPerUnit;
    if (dpr === this.dpr) return;
    this.dpr = dpr;
    for (const g of this.guideObjects) g.material.linewidth = cssToDevice(GUIDE_WIDTH_CSS, dpr);
  }

  /** Theme change: re-read the palette into the marker + guide materials. */
  refreshColors(): void {
    this.markerMat.color.copy(palette.sketchUnder());
    for (const g of this.guideObjects) g.material.color.copy(palette.sketchUnder());
  }

  private setGlyph(glyph: MarkerGlyph): void {
    if (this.currentGlyph === glyph) return;
    if (!(glyph in this.glyphTextures)) this.glyphTextures[glyph] = makeGlyphTexture(glyph);
    this.markerMat.map = this.glyphTextures[glyph] ?? null;
    this.markerMat.needsUpdate = true;
    this.currentGlyph = glyph;
  }

  /**
   * Update from a snap DECISION. `showHints` gates the hint chip only.
   *
   * `decision.guides` is the union of EVERY accepted candidate's guides, so a
   * composed x+y alignment draws both arms — the single-winner shape this
   * replaced could only ever show one.
   */
  show(snap: SnapDecision, showHints: boolean): void {
    if (!this.plane || !snap.snapped) {
      this.hide();
      return;
    }
    this.group.visible = true;
    this.setGlyph(glyphFor(snap.primaryKind));

    // Marker (plane-local).
    const pos = this.marker.geometry.getAttribute("position") as THREE.BufferAttribute;
    pos.setXYZ(0, snap.point.x, snap.point.y, 0);
    pos.needsUpdate = true;

    // Guides — REFERENCE point to snapped point only, not a fixed span across
    // the whole plane (P2 hardening): a full-viewport cross read as excessive
    // even faded, and a local segment is what actually shows WHICH relationship
    // just snapped. A guide this short can be exactly zero length when the
    // cursor sits ON the reference (or ON the polar anchor) — degenerate, so it
    // is skipped rather than drawing a stray dash.
    const segments: Array<[number, number, number, number]> = [];
    for (const g of snap.guides) {
      if (g.orientation === "vertical") {
        if (g.ref.y === snap.point.y) continue;
        segments.push([g.value, g.ref.y, g.value, snap.point.y]);
      } else if (g.orientation === "horizontal") {
        if (g.ref.x === snap.point.x) continue;
        segments.push([g.ref.x, g.value, snap.point.x, g.value]);
      } else {
        if (g.origin.x === snap.point.x && g.origin.y === snap.point.y) continue;
        segments.push([g.origin.x, g.origin.y, snap.point.x, snap.point.y]);
      }
    }
    this.setGuides(segments);

    // Hint chip.
    if (showHints && snap.label) {
      const world = planePointToWorld(this.plane, snap.point);
      if (!this.hintRegistered) {
        this.deps.overlay.register(HINT_ID, this.hintEl, world);
        this.hintRegistered = true;
      } else {
        this.deps.overlay.setWorldPos(HINT_ID, world);
      }
      this.hintEl.textContent = snap.label;
      this.hintEl.style.display = "";
      // Sit close to the marker, not detached from it (Track B4) — a small
      // nudge up-right so the text doesn't sit directly on the glyph.
      this.hintEl.style.marginLeft = "6px";
      this.hintEl.style.marginTop = "-8px";
    } else {
      this.hintEl.style.display = "none";
    }
    this.deps.invalidate();
  }

  /**
   * Rebuild the guide segments, giving each its OWN dash scale.
   *
   * `LineMaterial`'s dash sizes are world units, so one shared material cannot
   * serve two guides of different world lengths at a fixed CSS cadence. Each
   * guide therefore carries its own material, sized from the current
   * plane→screen scale.
   */
  private setGuides(segments: ReadonlyArray<[number, number, number, number]>): void {
    while (this.guideObjects.length > segments.length) {
      const g = this.guideObjects.pop();
      if (!g) break;
      this.guideGroup.remove(g.line);
      g.geometry.dispose();
      g.material.dispose();
    }
    const worldPerPx = 1 / Math.max(this.pxPerUnit, 1e-9);
    segments.forEach((seg, i) => {
      let obj = this.guideObjects[i];
      if (!obj) {
        obj = this.makeGuide();
        this.guideObjects.push(obj);
        this.guideGroup.add(obj.line);
      }
      const [x0, y0, x1, y1] = seg;
      obj.geometry.setPositions([x0, y0, 0, x1, y1, 0]);
      obj.line.computeLineDistances();
      obj.material.dashSize = GUIDE_DASH_CSS * worldPerPx;
      obj.material.gapSize = GUIDE_GAP_CSS * worldPerPx;
      obj.material.needsUpdate = true;
      obj.line.visible = true;
    });
  }

  /** The plane→screen scale the dash cadence is measured in (CSS px per plane
   *  unit). Set by the engine; also settable directly for tests. */
  setScreenScale(pxPerUnit: number): void {
    if (Number.isFinite(pxPerUnit) && pxPerUnit > 0) this.pxPerUnit = pxPerUnit;
  }

  /** Test/diagnostic view of the rendered guides. */
  guideDashSizes(): Array<{ dash: number; gap: number; width: number }> {
    return this.guideObjects.map((g) => ({
      dash: g.material.dashSize,
      gap: g.material.gapSize,
      width: g.material.linewidth,
    }));
  }

  private makeGuide(): GuideLineObject {
    const material = new LineMaterial({
      color: palette.sketchUnder().getHex(),
      linewidth: cssToDevice(GUIDE_WIDTH_CSS, this.dpr),
      dashed: true,
      dashSize: GUIDE_DASH_CSS,
      gapSize: GUIDE_GAP_CSS,
      depthTest: false,
      transparent: true,
      // Weak by design (Sketcher UX cleanup, Track B4) — a TRANSIENT alignment
      // guide, not geometry; it must never compete visually with real sketch
      // entities, which is what 0.75 read as.
      opacity: 0.3,
      toneMapped: false,
    });
    material.resolution.copy(this.resolution);
    const geometry = new LineGeometry();
    geometry.setPositions([0, 0, 0, 0, 0, 0]);
    const line = new Line2(geometry, material);
    line.renderOrder = RENDER_ORDER.SNAP_GUIDES;
    line.computeLineDistances();
    return { line, material, geometry };
  }

  hide(): void {
    if (this.group.visible) {
      this.group.visible = false;
      this.deps.invalidate();
    }
    this.hintEl.style.display = "none";
  }

  dispose(): void {
    this.marker.geometry.dispose();
    for (const tex of Object.values(this.glyphTextures)) tex?.dispose();
    this.markerMat.dispose();
    for (const g of this.guideObjects) {
      this.guideGroup.remove(g.line);
      g.geometry.dispose();
      g.material.dispose();
    }
    this.guideObjects.length = 0;
    if (this.hintRegistered) this.deps.overlay.unregister(HINT_ID);
    this.hintEl.remove();
    this.deps.interactionRoot.remove(this.group);
  }
}
