/*
 * PlanePicker — the plane-select gizmo shown before entering a sketch: three
 * translucent origin-plane quads (XY/XZ/YZ) living in the engine's
 * interactionRoot, each under its own group carrying the sketch-plane basis
 * (RevolvePreview / SketchObject convention — geometry authored plane-local,
 * matrixAutoUpdate off).
 *
 * The repo's canonical bases (mockSketch.ts planeFor) are LEGACY-SWAPPED: kind
 * "XZ" has world normal +X and kind "YZ" has world normal +Y. hitTest() still
 * returns the repo `kind` (for wire use — Sketch.plane payloads), but the
 * hover LABEL shows the GEOMETRIC name derived from the quad's actual world
 * normal, so the on-screen chip reads correctly regardless of the swap.
 */
import * as THREE from "three";
import type { SketchPlaneKind } from "@/ipc/types";
import { planeFor } from "@/ipc/mockSketch";
import { planeBasisMatrix } from "./sketchBasis";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { worldPerPixel } from "./screenScale";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";

export type PickablePlane = Exclude<SketchPlaneKind, "custom">;

export interface PlanePickerDeps {
  root: THREE.Object3D; // interactionRoot
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
  invalidate: () => void;
}

export interface PlaneHit {
  kind: PickablePlane;
  point: THREE.Vector3;
}

const KINDS: PickablePlane[] = ["XY", "XZ", "YZ"];
/** Quad edge in world units, authored once; `update()` rescales it per frame. */
const SIZE = 120;
const HALF = SIZE / 2;
/**
 * Quad edge in CSS px, held constant across zoom. Matches what SIZE used to
 * cover at the default framing, so the gizmo looks unchanged when you open it
 * and simply stops swelling as you zoom in.
 */
const PLANE_PX = 240;
const OPACITY_BASE = 0.16;
const OPACITY_HOVER = 0.35;
const OUTLINE_OPACITY = 0.5;
const LABEL_ID = "__plane_pick_label";

function colorFor(kind: PickablePlane): THREE.Color {
  switch (kind) {
    case "XY":
      return palette.planeXY();
    case "XZ":
      return palette.planeXZ();
    case "YZ":
      return palette.planeYZ();
  }
}

/** Geometric plane name for a WORLD normal (standard convention: the plane
 *  perpendicular to +X is "YZ", to +Y is "XZ", to +Z is "XY") — independent of
 *  which repo `kind` that basis happens to be labeled with (see module doc).
 *  Exported so anything that shows a plane to the user (the datum chip) reads
 *  the same name the picker's hover chip does. */
export function geometricLabel(normal: readonly [number, number, number]): string {
  const [ax, ay, az] = [Math.abs(normal[0]), Math.abs(normal[1]), Math.abs(normal[2])];
  if (ax >= ay && ax >= az) return "YZ";
  if (ay >= az) return "XZ";
  return "XY";
}

interface PlaneQuad {
  kind: PickablePlane;
  label: string; // geometric name, derived from the quad's world normal
  group: THREE.Group;
  mesh: THREE.Mesh;
  mat: THREE.MeshBasicMaterial;
  outline: THREE.LineLoop;
  outlineMat: THREE.LineBasicMaterial;
}

export class PlanePicker {
  private readonly group = new THREE.Group();
  private readonly quads: PlaneQuad[];
  private readonly labelEl: HTMLElement;
  private labelRegistered = false;
  private hoveredKind: PickablePlane | null = null;
  private readonly _basis = new THREE.Matrix4();

  constructor(private readonly deps: PlanePickerDeps) {
    this.group.name = "planePicker";
    this.group.visible = false;
    deps.root.add(this.group);

    this.quads = KINDS.map((kind) => this.buildQuad(kind));
    // The quad groups are matrixAutoUpdate:false, so their world matrices are stale
    // until the render loop runs (scene.updateMatrixWorld). Flush them once now so a
    // hitTest right after construction (before any frame) raycasts correctly.
    this.group.updateMatrixWorld(true);

    this.labelEl = document.createElement("div");
    this.labelEl.dataset.planePickLabel = "1";
    const s = this.labelEl.style;
    s.font = "700 12px var(--font-ui)";
    s.padding = "3px 7px";
    s.borderRadius = "5px";
    s.whiteSpace = "nowrap";
    s.pointerEvents = "none";
    s.background = "var(--color-sel-bg)";
    s.color = "var(--color-sel-text)";
    s.display = "none";
    deps.overlayEl.appendChild(this.labelEl);
  }

  private buildQuad(kind: PickablePlane): PlaneQuad {
    const plane = planeFor(kind);
    const color = colorFor(kind);

    const mat = new THREE.MeshBasicMaterial({
      color,
      transparent: true,
      opacity: OPACITY_BASE,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(SIZE, SIZE), mat);
    mesh.renderOrder = RENDER_ORDER.PLANE_PICK_FILL;
    mesh.userData.kind = kind;

    const outlineMat = new THREE.LineBasicMaterial({ color, transparent: true, opacity: OUTLINE_OPACITY, toneMapped: false });
    const outlineGeo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-HALF, -HALF, 0),
      new THREE.Vector3(HALF, -HALF, 0),
      new THREE.Vector3(HALF, HALF, 0),
      new THREE.Vector3(-HALF, HALF, 0),
    ]);
    const outline = new THREE.LineLoop(outlineGeo, outlineMat);
    outline.renderOrder = RENDER_ORDER.PLANE_PICK_OUTLINE;

    const group = new THREE.Group();
    group.name = `planePick_${kind}`;
    group.matrixAutoUpdate = false;
    planeBasisMatrix(plane, this._basis);
    group.matrix.copy(this._basis);
    group.matrixWorldNeedsUpdate = true;
    group.add(mesh, outline);
    this.group.add(group);

    return { kind, label: geometricLabel(plane.normal), group, mesh, mat, outline, outlineMat };
  }

  setVisible(visible: boolean): void {
    this.group.visible = visible;
    if (!visible) this.setHover(null);
    this.deps.invalidate();
  }

  /**
   * Per-frame: rescale the quads so their edge spans PLANE_PX on screen, the
   * same constant-size rule the origin triad uses. This also keeps the CLICK
   * TARGET a fixed size — the quads are the raycast geometry, so a world-sized
   * gizmo would shrink out of reach when zoomed out and swallow the viewport
   * when zoomed in. `height` is the viewport height in CSS px.
   */
  update(camera: THREE.Camera, height: number): void {
    // The picker sits at the world origin (the origin planes' shared point).
    const scale = (worldPerPixel(camera, this.group.position, height) * PLANE_PX) / SIZE;
    this.group.scale.setScalar(scale);
    this.group.updateMatrixWorld(true);
  }

  get visible(): boolean {
    return this.group.visible;
  }

  /** Nearest quad under `raycaster`, or null. Returns the repo `kind` (wire use). */
  hitTest(raycaster: THREE.Raycaster): PlaneHit | null {
    if (!this.group.visible) return null;
    // Ensure the quad world matrices are current (matrices are static, 3 groups —
    // negligible) so intersectObjects sees the real plane positions even if the
    // caller raycasts outside the render loop.
    this.group.updateMatrixWorld(true);
    const hits = raycaster.intersectObjects(
      this.quads.map((q) => q.mesh),
      false,
    );
    if (hits.length === 0) return null;
    const hit = hits[0];
    const kind = hit.object.userData.kind as PickablePlane;
    return { kind, point: hit.point.clone() };
  }

  /** Highlight the hovered quad + show its geometric-name label chip, or clear. */
  setHover(hit: PlaneHit | null): void {
    this.hoveredKind = hit?.kind ?? null;
    for (const q of this.quads) {
      q.mat.opacity = q.kind === this.hoveredKind ? OPACITY_HOVER : OPACITY_BASE;
    }

    if (!hit) {
      if (this.labelRegistered) {
        this.deps.overlay.unregister(LABEL_ID);
        this.labelRegistered = false;
      }
      this.labelEl.style.display = "none";
      this.deps.invalidate();
      return;
    }

    const quad = this.quads.find((q) => q.kind === hit.kind);
    this.labelEl.textContent = quad?.label ?? hit.kind;
    this.labelEl.style.display = "";
    if (!this.labelRegistered) {
      this.deps.overlay.register(LABEL_ID, this.labelEl, hit.point);
      this.labelRegistered = true;
    } else {
      this.deps.overlay.setWorldPos(LABEL_ID, hit.point);
    }
    this.deps.invalidate();
  }

  dispose(): void {
    for (const q of this.quads) {
      q.mesh.geometry.dispose();
      q.mat.dispose();
      q.outline.geometry.dispose();
      q.outlineMat.dispose();
    }
    if (this.labelRegistered) this.deps.overlay.unregister(LABEL_ID);
    this.labelEl.remove();
    this.deps.root.remove(this.group);
  }
}
