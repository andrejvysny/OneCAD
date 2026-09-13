/*
 * screenLineStyle — the ONE unit contract for every screen-space fat line
 * (`LineMaterial` / `Line2` / `LineSegments2`) in the viewport.
 *
 * THE CONTRACT: `linewidth` is CSS (logical) pixels, `resolution` is CSS
 * (logical) pixels, and a screen-space pick radius is CSS pixels. Nothing here
 * is multiplied by the device pixel ratio, ever.
 *
 * That is not a preference — it is what the INSTALLED addon does, verified
 * against three 0.185.1 in `node_modules`:
 *
 *   - `three/examples/jsm/lines/LineSegments2.js:419-428` — `onBeforeRender`
 *     UNCONDITIONALLY overwrites the uniform:
 *         renderer.getViewport( _viewport );
 *         this.material.uniforms.resolution.value.set( _viewport.z, _viewport.w );
 *     So whatever the application wrote into `material.resolution` is discarded
 *     at draw time. The only thing an application write still buys is the
 *     PRE-FIRST-FRAME raycast (see `Picker.flushEdgeResolution`), and it must
 *     therefore be written in the same units the renderer will use.
 *   - `three/src/renderers/WebGLRenderer.js:781-785` — `getViewport` returns
 *     `_viewport`, which `setSize(w, h)` stores UNSCALED. Only
 *     `getDrawingBufferSize` multiplies by the pixel ratio. So the draw-time
 *     resolution is LOGICAL pixels.
 *   - `three/examples/jsm/lines/LineMaterial.js:239-243` — the vertex shader
 *     does `offset *= linewidth; offset /= resolution.y`. A device-pixel
 *     `linewidth` over a logical-pixel `resolution` therefore draws `dpr` times
 *     too wide. That was finding R01.
 *
 * Points and textures are a SEPARATE adapter and do not belong here — see the
 * unit note on `THREE.Points` in `SketchObject.ts`.
 */
import { LineMaterial, type LineMaterialParameters } from "three/examples/jsm/lines/LineMaterial.js";

/**
 * The baseline line hierarchy, spec §7.3. Every value is CSS pixels and is fed
 * to `LineMaterial.linewidth` UNSCALED.
 */
export const LINE_WIDTHS_CSS = {
  /** Body feature edge — the silhouette/boundary weight bodies are read by. */
  bodyFeatureEdge: 1.25,
  /** Tangent edge, only when explicitly displayed. Unused until WP13. */
  tangentEdge: 0.75,
  /** Active (in-session) sketch semantic ink. */
  activeSketchInk: 1.25,
  /** Static, non-editable sketch ink in model mode. */
  staticSketchInk: 1.0,
  /** Draft / in-progress (rubber-band) geometry. */
  draft: 1.25,
  /** Selection halo — drawn BEHIND the semantic stroke, never replacing it. */
  selectionHalo: 3.0,
  /** Dimension lines and snap guides. */
  dimensionOrSnapGuide: 1.0,
  /** Hidden selected edge, dashed. Unused until WP13. */
  hiddenSelectedEdge: 1.0,
  /**
   * Trim ghost. NOT in the §7.3 table — a transient destructive-preview
   * overlay, deliberately heavier than any real entity so it reads as "this is
   * what disappears". Kept at its authored weight.
   */
  trimGhost: 2.0,
  /**
   * Angle-preview arc. NOT in the §7.3 table — an annotation glyph that must
   * stay lighter than every real entity line. Kept at its authored weight.
   */
  angleArc: 0.75,
} as const;

export interface ScreenLineStyle {
  /** Stroke weight in CSS pixels. Fed to `LineMaterial.linewidth` unscaled. */
  readonly widthCss: number;
  /** Dashed stroke (`LineMaterial.dashed` + the `USE_DASH` define). */
  readonly dashed?: boolean;
  /**
   * Authored dash length in CSS pixels.
   *
   * CAUTION: `LineMaterial.dashSize`/`gapSize` are WORLD units (the shader
   * compares against `vLineDistance`), so this is only the SEED value. An owner
   * that wants a constant on-screen cadence must convert per frame from its own
   * world→screen metric — `SnapIndicator.update` is the worked example.
   */
  readonly dashSizeCss?: number;
  /** Authored gap length in CSS pixels. Same world-unit caveat as `dashSizeCss`. */
  readonly gapSizeCss?: number;
}

/**
 * A `LineMaterial` whose width is the style's CSS width. `params` carries
 * everything that is not a unit decision (colour, blending, depth policy) and
 * is applied AFTER the style, so a caller can still override a seeded dash with
 * a world-unit value it already computed.
 */
export function createScreenLineMaterial(
  style: ScreenLineStyle,
  params: Partial<LineMaterialParameters> = {},
): LineMaterial {
  const mat = new LineMaterial({
    linewidth: style.widthCss,
    ...(style.dashed === undefined ? {} : { dashed: style.dashed }),
    ...(style.dashSizeCss === undefined ? {} : { dashSize: style.dashSizeCss }),
    ...(style.gapSizeCss === undefined ? {} : { gapSize: style.gapSizeCss }),
    ...params,
  });
  return mat;
}

/** Re-apply a style to a live material (theme/spec change; never per frame). */
export function applyScreenLineStyle(mat: LineMaterial, style: ScreenLineStyle): void {
  mat.linewidth = style.widthCss;
  if (style.dashed !== undefined) mat.dashed = style.dashed;
  if (style.dashSizeCss !== undefined) mat.dashSize = style.dashSizeCss;
  if (style.gapSizeCss !== undefined) mat.gapSize = style.gapSizeCss;
}

/**
 * Write the LOGICAL viewport size into a fat line's `resolution`.
 *
 * At draw time `onBeforeRender` overwrites this with exactly the same numbers
 * (see the header), so the only caller that needs it is one raycasting BEFORE
 * a frame has rendered — `LineSegments2.raycast` at resolution (0,0) returns
 * silently with no hits.
 */
export function setLineResolutionCss(mat: LineMaterial, cssWidth: number, cssHeight: number): void {
  mat.resolution.set(cssWidth, cssHeight);
}

/**
 * `raycaster.params.Line2.threshold` that makes a `LineSegments2` pick radius
 * exactly `radiusCss` CSS pixels for a line drawn `widthCss` CSS pixels wide.
 *
 * `LineSegments2.raycast` tests `dist < (material.linewidth + threshold) / 2`
 * in the units `resolution` is expressed in — CSS pixels, per the header — so
 * the DRAWN width has to be subtracted out; otherwise a fatter edge would
 * silently acquire from farther away than a thin one. There is no DPR term:
 * both operands are already CSS.
 *
 * Clamped at 0 — a line drawn wider than the tolerance already picks at its own
 * half-width. The clamp is load-bearing: `LineSegments2.raycast` reads
 * `params.Line2.threshold || 0`, and a negative number is truthy, so an
 * unclamped negative value would SHRINK the pick radius below the drawn width.
 */
export function line2PickThresholdCss(radiusCss: number, widthCss: number): number {
  return Math.max(0, 2 * radiusCss - widthCss);
}
