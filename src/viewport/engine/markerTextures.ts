/*
 * Persistent sketch marker glyphs (vertex ring, midpoint/centroid dot).
 *
 * Same technique as SnapIndicator's `makeGlyphTexture`: a white mask on a
 * transparent canvas, tinted by the consuming PointsMaterial's `color` — so
 * one texture serves any token-driven color without baking a color into the
 * bitmap itself.
 */
import * as THREE from "three";

const SIZE = 32;

function canvasTexture(draw: (ctx: CanvasRenderingContext2D, c: number) => void): THREE.Texture | null {
  const canvas = document.createElement("canvas");
  canvas.width = SIZE;
  canvas.height = SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null; // jsdom / no 2D context — caller falls back to a plain point
  ctx.strokeStyle = "white";
  ctx.fillStyle = "white";
  draw(ctx, SIZE / 2);
  const tex = new THREE.CanvasTexture(canvas);
  tex.needsUpdate = true;
  return tex;
}

/** Open circle outline — the committed-entity vertex/corner marker. */
export function buildRingTexture(): THREE.Texture | null {
  return canvasTexture((ctx, c) => {
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.arc(c, c, c - 4, 0, Math.PI * 2);
    ctx.stroke();
  });
}

/** Filled circle — edge midpoint / closed-loop centroid marker. */
export function buildDotTexture(): THREE.Texture | null {
  return canvasTexture((ctx, c) => {
    ctx.beginPath();
    ctx.arc(c, c, c * 0.4, 0, Math.PI * 2);
    ctx.fill();
  });
}
