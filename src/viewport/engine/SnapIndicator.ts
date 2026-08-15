/*
 * SnapIndicator — the in-canvas snap feedback (F-WP6, NEW_SPEC §14).
 *
 * Renders, in `interactionRoot` (plane-local under the plane basis):
 *   - a constant-size marker at the snapped point (THREE.Points, sizeAttenuation
 *     off ⇒ crisp regardless of zoom),
 *   - dashed H/V/polar alignment guide lines from their reference point to
 *     the snapped point (not spanning the plane — see `show()`),
 * plus a hint chip ("Endpoint" / "Vertical" / …) via the HtmlOverlayDriver,
 * honoring `settingsStore.show.snappingHints`.
 *
 * The chip is styled from design tokens through `var(--color-*)` (no raw hex —
 * the tokens-only gate), positioned each frame by the overlay driver.
 */
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import type { SnapResult } from "@/tools/sketch/snapEngine";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { planeBasisMatrix, planePointToWorld } from "./sketchBasis";

const HINT_ID = "__sketch_snap_hint";

interface SnapIndicatorDeps {
  interactionRoot: THREE.Object3D;
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
  invalidate: () => void;
}

/** Marker glyph shapes, one per snap-type family (constant screen size). */
type MarkerGlyph = "dot" | "diamond" | "cross" | "ring";

/** Map a snap kind to its marker glyph. New M6c types get distinct shapes so the
 *  marker (not just the hint chip) tells endpoint/quadrant/intersection/onCurve
 *  apart, matching the legacy per-type snap markers. */
function glyphFor(kind: SnapResult["kind"]): MarkerGlyph {
  switch (kind) {
    case "quadrant":
      return "diamond";
    case "intersection":
      return "cross";
    case "onCurve":
      return "ring";
    default:
      return "dot"; // endpoint / midpoint / center / grid / align* / polar
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
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.needsUpdate = true;
  return tex;
}

export class SnapIndicator {
  private readonly group = new THREE.Group();
  private readonly marker: THREE.Points;
  private readonly markerMat: THREE.PointsMaterial;
  private readonly guides: THREE.LineSegments;
  private readonly guideMat: THREE.LineDashedMaterial;
  private readonly hintEl: HTMLElement;
  private readonly _basis = new THREE.Matrix4();
  private readonly glyphTextures: Partial<Record<MarkerGlyph, THREE.Texture | null>> = {};
  private currentGlyph: MarkerGlyph | null = null;
  private plane: SketchPlane | null = null;
  private hintRegistered = false;

  constructor(private readonly deps: SnapIndicatorDeps) {
    this.group.name = "snapIndicator";
    this.group.visible = false;
    this.group.matrixAutoUpdate = false;
    deps.interactionRoot.add(this.group);

    this.markerMat = new THREE.PointsMaterial({
      color: palette.sketchUnder(),
      size: 11,
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

    this.guideMat = new THREE.LineDashedMaterial({
      color: palette.sketchUnder(),
      dashSize: 6,
      gapSize: 4,
      depthTest: false,
      transparent: true,
      // Weak by design (Sketcher UX cleanup, Track B4) — a TRANSIENT alignment
      // guide, not geometry; it must never compete visually with real sketch
      // entities, which is what 0.75 read as.
      opacity: 0.3,
      toneMapped: false,
    });
    this.guides = new THREE.LineSegments(new THREE.BufferGeometry(), this.guideMat);
    this.guides.renderOrder = RENDER_ORDER.SNAP_GUIDES;
    this.group.add(this.guides);
    // (refreshColors below re-reads both of the above on a theme change; the
    // hint chip styles itself from CSS vars and needs nothing.)

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

  /** Swap the marker sprite to the given glyph (lazily building its texture). */
  /** Theme change: re-read the palette into the marker + guide materials. */
  refreshColors(): void {
    this.markerMat.color.copy(palette.sketchUnder());
    this.guideMat.color.copy(palette.sketchUnder());
  }

  private setGlyph(glyph: MarkerGlyph): void {
    if (this.currentGlyph === glyph) return;
    if (!(glyph in this.glyphTextures)) this.glyphTextures[glyph] = makeGlyphTexture(glyph);
    this.markerMat.map = this.glyphTextures[glyph] ?? null;
    this.markerMat.needsUpdate = true;
    this.currentGlyph = glyph;
  }

  /** Update from a snap result. `showHints` gates the hint chip only. */
  show(snap: SnapResult, showHints: boolean): void {
    if (!this.plane || !snap.snapped) {
      this.hide();
      return;
    }
    this.group.visible = true;
    this.setGlyph(glyphFor(snap.kind));

    // Marker (plane-local).
    const pos = this.marker.geometry.getAttribute("position") as THREE.BufferAttribute;
    pos.setXYZ(0, snap.point.x, snap.point.y, 0);
    pos.needsUpdate = true;

    // Guides (plane-local dashed segments) — REFERENCE point to snapped
    // point only, not a fixed span across the whole plane (P2 hardening): a
    // full-viewport cross read as excessive even faded, and a
    // local segment is what actually shows the reader WHICH relationship
    // just snapped, not merely "something is aligned somewhere." A guide
    // this short can be exactly zero length when the cursor sits ON the
    // reference (or ON the polar anchor) — degenerate, so it's skipped
    // rather than drawing a stray zero-length dash.
    const seg: number[] = [];
    for (const g of snap.guides) {
      if (g.orientation === "vertical") {
        if (g.ref.y === snap.point.y) continue;
        seg.push(g.value, g.ref.y, 0, g.value, snap.point.y, 0);
      } else if (g.orientation === "horizontal") {
        if (g.ref.x === snap.point.x) continue;
        seg.push(g.ref.x, g.value, 0, snap.point.x, g.value, 0);
      } else {
        if (g.origin.x === snap.point.x && g.origin.y === snap.point.y) continue;
        seg.push(g.origin.x, g.origin.y, 0, snap.point.x, snap.point.y, 0);
      }
    }
    this.guides.geometry.setAttribute("position", new THREE.Float32BufferAttribute(seg, 3));
    this.guides.computeLineDistances();

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
      // nudge up-right so the text doesn't sit directly on the glyph, not the
      // floating-tooltip distance this used to be.
      this.hintEl.style.marginLeft = "6px";
      this.hintEl.style.marginTop = "-8px";
    } else {
      this.hintEl.style.display = "none";
    }
    this.deps.invalidate();
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
    this.guides.geometry.dispose();
    this.guideMat.dispose();
    if (this.hintRegistered) this.deps.overlay.unregister(HINT_ID);
    this.hintEl.remove();
    this.deps.interactionRoot.remove(this.group);
  }
}
