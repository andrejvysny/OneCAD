/*
 * ValueWitnessLayer — the segment an armed value tool draws to SHOW what its
 * number means, plus the label that says what the segment is allowed to claim.
 *
 * WHY IT IS A CLAIM AND NOT A MEASUREMENT. A Fillet/Chamfer handle is a
 * PARAMETER construction: `H(q) = E + q·b`, a segment whose length is the value
 * being dragged. It is not the blend's contact point — for a certified convex
 * right-angle corner the true blend midpoint moves the OTHER way, by
 * −(√2−1)·q·b, and on the concave corner that sign reverses
 * (docs/design/astra/modeling-handle-attachment.md §7). Drawing this segment
 * without its label would therefore be a visible lie, which is why
 * `ValueWitness.meaning` and `ValueWitness.label` travel with the geometry and
 * are not something the renderer invents.
 *
 * The line is a plain two-point `THREE.Line` in the interaction band with the
 * drag handle's own tokens, so it reads as part of the same affordance; the
 * label rides the shared `HtmlOverlayDriver` (no React, no per-frame layout).
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import type { ValueWitness } from "@/tools/preview/handleProjection";

/**
 * One driver id per SEGMENT — only one value tool is armed at a time, but a tool
 * may draw more than one segment: an OffsetFace `Total` shows the prepared
 * reference thickness AND the target it is being dragged to, and those two carry
 * DIFFERENT claims (one is a kernel measurement, the other is not). Collapsing
 * them into a single label would attach the measurement to a value nobody has
 * built yet. The first id is unsuffixed so a single-segment tool keeps the
 * overlay registration it always had.
 */
const OVERLAY_ID = "__value_witness";
const overlayIdFor = (index: number): string => (index === 0 ? OVERLAY_ID : `${OVERLAY_ID}_${index}`);

export interface ValueWitnessDeps {
  root: THREE.Object3D; // interactionRoot
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement | null;
  invalidate: () => void;
}

/** One drawn segment: its own line, its own buffer, its own label. */
interface WitnessSegment {
  readonly geometry: THREE.BufferGeometry;
  readonly positions: Float32Array;
  readonly line: THREE.Line;
  readonly label: HTMLElement | null;
}

export class ValueWitnessLayer {
  private readonly material: THREE.LineBasicMaterial;
  private readonly segments: WitnessSegment[] = [];
  private shown = false;

  constructor(private readonly deps: ValueWitnessDeps) {
    // depthTest off for the same reason the drag handle has it off: the witness
    // describes the op, and a body it passes through must not hide it.
    this.material = new THREE.LineBasicMaterial({
      color: palette.hoverAccent(),
      depthTest: false,
      transparent: true,
      opacity: 0.9,
      toneMapped: false,
    });
  }

  /** The `index`-th segment, created on first use. Shared material, own buffer. */
  private segmentAt(index: number): WitnessSegment {
    const existing = this.segments[index];
    if (existing) return existing;
    const positions = new Float32Array(6);
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    const line = new THREE.Line(geometry, this.material);
    line.name = index === 0 ? "valueWitness" : `valueWitness_${index}`;
    line.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    line.visible = false;
    this.deps.root.add(line);

    const overlayEl = this.deps.overlayEl;
    let label: HTMLElement | null = null;
    if (overlayEl) {
      label = document.createElement("div");
      label.dataset.testid = "value-witness-label";
      label.style.font = "11px system-ui, sans-serif";
      label.style.padding = "1px 6px";
      label.style.borderRadius = "3px";
      label.style.whiteSpace = "nowrap";
      label.style.background = "var(--color-tooltip)";
      label.style.color = "var(--color-tooltip-text)";
      label.style.pointerEvents = "none";
      label.style.display = "none";
      overlayEl.appendChild(label);
    }
    const segment: WitnessSegment = { geometry, positions, line, label };
    this.segments[index] = segment;
    return segment;
  }

  /** Take one segment off screen without destroying it — a later show reuses it. */
  private retire(index: number): void {
    const segment = this.segments[index];
    if (!segment) return;
    segment.line.visible = false;
    if (segment.label) {
      // Unregistering is the mechanism: `HtmlOverlayDriver.update` writes
      // `display = ""` to every registered visible item on every frame, so a
      // `display: none` alone is undone by the next frame.
      this.deps.overlay.unregister(overlayIdFor(index));
      segment.label.style.display = "none";
    }
  }

  /**
   * Draw `witnesses` — the segments AND the claims. Called at arm and on every
   * value change, so the segments track the number the way the handle does.
   *
   * A tool may hand over one witness or several; a shorter list than last time
   * RETIRES the extra segments rather than leaving a stale claim on screen.
   *
   * A zero-length segment is still drawn: at the authoring floor the value is
   * genuinely tiny, and inflating the geometry to make it visible would be the
   * one thing this layer exists to prevent.
   */
  show(witnesses: ValueWitness | readonly ValueWitness[]): void {
    const list = Array.isArray(witnesses) ? witnesses : [witnesses as ValueWitness];
    if (list.length === 0) {
      this.hide();
      return;
    }
    list.forEach((witness, index) => {
      const segment = this.segmentAt(index);
      segment.positions.set(witness.fromMm, 0);
      segment.positions.set(witness.toMm, 3);
      segment.geometry.getAttribute("position").needsUpdate = true;
      segment.geometry.computeBoundingSphere();
      segment.line.visible = true;
      if (!segment.label) return;
      segment.label.textContent = witness.label;
      segment.label.style.display = "";
      this.deps.overlay.register(
        overlayIdFor(index),
        segment.label,
        new THREE.Vector3(
          (witness.fromMm[0] + witness.toMm[0]) / 2,
          (witness.fromMm[1] + witness.toMm[1]) / 2,
          (witness.fromMm[2] + witness.toMm[2]) / 2,
        ),
      );
    });
    for (let i = list.length; i < this.segments.length; i++) this.retire(i);
    this.shown = true;
    this.deps.invalidate();
  }

  hide(): void {
    if (!this.shown) return;
    this.shown = false;
    for (let i = 0; i < this.segments.length; i++) this.retire(i);
    this.deps.invalidate();
  }

  /** True while the witness is on screen (gate / introspection probe). */
  get visible(): boolean {
    return this.shown;
  }

  refreshColors(): void {
    this.material.color.copy(palette.hoverAccent());
  }

  dispose(): void {
    this.segments.forEach((segment, index) => {
      this.deps.overlay.unregister(overlayIdFor(index));
      segment.label?.remove();
      segment.geometry.dispose();
      this.deps.root.remove(segment.line);
    });
    this.segments.length = 0;
    this.material.dispose();
  }
}
