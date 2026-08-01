/*
 * DatumLayer — the always-visible presence of every document datum (reference)
 * plane, plus the datum TOOL's live ghost quad (DATUM W1).
 *
 * Structurally a PlanePicker clone: one group per datum carrying the datum's
 * basis matrix (`sketchBasis.planeBasisMatrix`), holding a translucent quad + an
 * outline authored plane-local, with `matrixAutoUpdate` off. The quads are
 * rescaled per frame so their edge spans a constant number of CSS px — same rule
 * the origin triad and the plane picker use, and load-bearing for the same
 * reason: the quad IS the raycast geometry, so a world-sized one would shrink
 * out of reach zoomed out and swallow the viewport zoomed in.
 *
 * The frame comes from `DatumMeta.plane`, which the BACKEND resolved. It is
 * never re-derived here from `basePlaneId + offset` — the core stamps a
 * datum-hosted sketch with exactly that basis, and a second derivation on this
 * side is precisely the two-sources-of-truth bug the migration exists to remove.
 * The one exception is the GHOST, which by definition has no backend frame yet
 * (nothing is committed) — see `datumGhostPlane`.
 *
 * Unlike the plane picker this layer is NOT in the orbit gate (SketchStaticLayer
 * convention): clicking empty space near a datum still orbits.
 */
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";
import { planeFor } from "@/ipc/mockSketch";
import type { PickablePlane } from "./PlanePicker";
import { planeBasisMatrix } from "./sketchBasis";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { worldPerPixel } from "./screenScale";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";

/** The minimum a datum needs to be drawn (a structural subset of `DatumMeta`). */
export interface DatumVisual {
  id: string;
  name: string;
  /** Backend-resolved frame — authoritative, never re-derived here. */
  plane: SketchPlane;
  /** `false` ⇒ the definition did not resolve; drawn dimmed, still selectable. */
  resolvedValid: boolean;
}

export interface DatumLayerDeps {
  root: THREE.Object3D; // interactionRoot
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
  invalidate: () => void;
}

/** Quad edge in world units, authored once; `update()` rescales it per frame. */
const SIZE = 120;
const HALF = SIZE / 2;
/** Quad edge in CSS px, held constant across zoom (PlanePicker's PLANE_PX / 2 —
 *  a datum is document content, not a modal gizmo, so it sits back a little). */
const DATUM_PX = 120;
const OPACITY_BASE = 0.1;
const OPACITY_ACTIVE = 0.24;
const OPACITY_UNRESOLVED = 0.05;
const OUTLINE_OPACITY = 0.45;
const GHOST_OPACITY = 0.2;
const GHOST_ID = "__datum_ghost";
const LABEL_PREFIX = "__datum_label_";

/**
 * The frame a freshly authored `OffsetFromPlane` datum will land on: slide the
 * base plane's origin along its own NORMAL by `offset`, carrying the base axes
 * verbatim. This mirrors `DocumentSession::resolve_datum_frame` (and the mock's
 * `mockResolveDatum`) so the ghost sits exactly where the commit will put it.
 *
 * **The repo bases are LEGACY-SWAPPED** (`mockSketch.PLANES`, ported from
 * `Sketch.h`): kind "XZ" has world normal **+X** and "YZ" has **+Y**. So an "XZ"
 * datum at offset 10 slides along world +X. Callers must not "correct" that.
 *
 * PURE + exported so the ghost's placement is unit-testable without a renderer.
 */
export function datumGhostPlane(baseKind: PickablePlane, offset: number): SketchPlane {
  const base = planeFor(baseKind);
  const n = base.normal;
  return {
    kind: "custom",
    origin: [
      base.origin[0] + n[0] * offset,
      base.origin[1] + n[1] * offset,
      base.origin[2] + n[2] * offset,
    ],
    xAxis: [...base.xAxis],
    yAxis: [...base.yAxis],
    normal: [...base.normal],
  };
}

interface DatumQuad {
  id: string;
  name: string;
  /** Frame identity, so a re-sync only rebuilds a datum whose basis moved. */
  frameKey: string;
  resolvedValid: boolean;
  group: THREE.Group;
  /** The un-scaled basis matrix; `update()` re-composes matrix = basis · scale. */
  basis: THREE.Matrix4;
  mesh: THREE.Mesh;
  mat: THREE.MeshBasicMaterial;
  outline: THREE.LineLoop;
  outlineMat: THREE.LineBasicMaterial;
  labelEl: HTMLElement;
}

/** Identity of a datum's rendered state (name + frame) — a change rebuilds it. */
function visualKey(d: DatumVisual): string {
  const p = d.plane;
  return JSON.stringify([d.name, d.resolvedValid, p.origin, p.xAxis, p.yAxis, p.normal]);
}

function quadOutlineGeometry(): THREE.BufferGeometry {
  return new THREE.BufferGeometry().setFromPoints([
    new THREE.Vector3(-HALF, -HALF, 0),
    new THREE.Vector3(HALF, -HALF, 0),
    new THREE.Vector3(HALF, HALF, 0),
    new THREE.Vector3(-HALF, HALF, 0),
  ]);
}

export class DatumLayer {
  private readonly root = new THREE.Group();
  private readonly quads = new Map<string, DatumQuad>();
  private hoverId: string | null = null;
  private selectedIds = new Set<string>();
  private readonly _scale = new THREE.Vector3();
  private readonly _origin = new THREE.Vector3();

  // The tool ghost: one reusable quad, hidden until `setGhost` supplies a frame.
  /** The ghost's UN-scaled basis (see `rescale`). */
  private readonly ghostBasis = new THREE.Matrix4();
  private ghostGroup: THREE.Group | null = null;
  private ghostMat: THREE.MeshBasicMaterial | null = null;
  private ghostOutlineMat: THREE.LineBasicMaterial | null = null;
  private ghostMesh: THREE.Mesh | null = null;
  private ghostOutline: THREE.LineLoop | null = null;
  private ghostEl: HTMLElement | null = null;
  private ghostRegistered = false;

  constructor(private readonly deps: DatumLayerDeps) {
    this.root.name = "datumRoot";
    deps.root.add(this.root);
  }

  /**
   * Reconcile the rendered datums against `metas` (add / remove / frame change).
   * A datum whose name + frame are unchanged is left ALONE — rebuilding every
   * entry on each projection would churn geometry (and the overlay labels) on
   * edits that have nothing to do with datums.
   */
  syncDatums(metas: readonly DatumVisual[]): void {
    const seen = new Set<string>();
    for (const meta of metas) {
      seen.add(meta.id);
      const prev = this.quads.get(meta.id);
      const key = visualKey(meta);
      if (prev && prev.frameKey === key) continue;
      if (prev) this.destroyQuad(prev);
      this.quads.set(meta.id, this.buildQuad(meta, key));
    }
    for (const [id, quad] of [...this.quads]) {
      if (seen.has(id)) continue;
      this.destroyQuad(quad);
      this.quads.delete(id);
    }
    this.root.updateMatrixWorld(true);
    this.applyTints();
    this.deps.invalidate();
  }

  private buildQuad(meta: DatumVisual, frameKey: string): DatumQuad {
    const mat = new THREE.MeshBasicMaterial({
      color: palette.bodyNeutral(),
      transparent: true,
      opacity: OPACITY_BASE,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(SIZE, SIZE), mat);
    mesh.renderOrder = RENDER_ORDER.DATUM_FILL;
    mesh.userData.datumId = meta.id;

    const outlineMat = new THREE.LineBasicMaterial({
      color: palette.bodyNeutral(),
      transparent: true,
      opacity: OUTLINE_OPACITY,
    });
    const outline = new THREE.LineLoop(quadOutlineGeometry(), outlineMat);
    outline.renderOrder = RENDER_ORDER.DATUM_OUTLINE;

    const group = new THREE.Group();
    group.name = `datum_${meta.id}`;
    group.matrixAutoUpdate = false;
    const basis = planeBasisMatrix(meta.plane, new THREE.Matrix4());
    group.matrix.copy(basis);
    group.matrixWorldNeedsUpdate = true;
    group.add(mesh, outline);
    this.root.add(group);

    const labelEl = document.createElement("div");
    labelEl.dataset.datumLabel = meta.id;
    labelEl.textContent = meta.name;
    const s = labelEl.style;
    s.font = "600 11px var(--font-ui)";
    s.padding = "2px 6px";
    s.borderRadius = "4px";
    s.whiteSpace = "nowrap";
    s.pointerEvents = "none";
    s.background = "var(--color-chip)";
    s.color = "var(--color-ink-3)";
    this.deps.overlayEl.appendChild(labelEl);
    this.deps.overlay.register(
      LABEL_PREFIX + meta.id,
      labelEl,
      new THREE.Vector3().fromArray(meta.plane.origin),
    );

    return {
      id: meta.id,
      name: meta.name,
      frameKey,
      resolvedValid: meta.resolvedValid,
      group,
      basis,
      mesh,
      mat,
      outline,
      outlineMat,
      labelEl,
    };
  }

  private destroyQuad(quad: DatumQuad): void {
    quad.mesh.geometry.dispose();
    quad.mat.dispose();
    quad.outline.geometry.dispose();
    quad.outlineMat.dispose();
    this.root.remove(quad.group);
    this.deps.overlay.unregister(LABEL_PREFIX + quad.id);
    quad.labelEl.remove();
  }

  /** Ids of the datums currently rendered (introspection / tests). */
  get datumIds(): string[] {
    return [...this.quads.keys()];
  }

  setHover(id: string | null): void {
    if (this.hoverId === id) return;
    this.hoverId = id;
    this.applyTints();
    this.deps.invalidate();
  }

  setSelected(ids: readonly string[]): void {
    this.selectedIds = new Set(ids);
    this.applyTints();
    this.deps.invalidate();
  }

  private applyTints(): void {
    for (const q of this.quads.values()) {
      const selected = this.selectedIds.has(q.id);
      const hovered = this.hoverId === q.id;
      const color = selected
        ? palette.selectedEdge()
        : hovered
          ? palette.hoverAccent()
          : palette.bodyNeutral();
      q.mat.color.copy(color);
      q.outlineMat.color.copy(color);
      q.mat.opacity = !q.resolvedValid
        ? OPACITY_UNRESOLVED
        : selected || hovered
          ? OPACITY_ACTIVE
          : OPACITY_BASE;
    }
  }

  /**
   * Show (or hide, with `null`) the datum TOOL's live ghost at `plane`. The
   * ghost is drawn in the accent color and is deliberately NOT hit-testable —
   * it represents a datum that does not exist yet.
   */
  setGhost(plane: SketchPlane | null, label?: string): void {
    if (!plane) {
      if (this.ghostGroup) this.ghostGroup.visible = false;
      if (this.ghostRegistered) {
        this.deps.overlay.unregister(GHOST_ID);
        this.ghostRegistered = false;
      }
      if (this.ghostEl) this.ghostEl.style.display = "none";
      this.deps.invalidate();
      return;
    }
    this.ensureGhost();
    const group = this.ghostGroup as THREE.Group;
    planeBasisMatrix(plane, this.ghostBasis);
    group.matrix.copy(this.ghostBasis);
    group.matrixWorldNeedsUpdate = true;
    group.visible = true;
    group.updateMatrixWorld(true);

    const el = this.ghostEl as HTMLElement;
    el.textContent = label ?? "";
    el.style.display = label ? "" : "none";
    const origin = new THREE.Vector3().fromArray(plane.origin);
    if (!this.ghostRegistered) {
      this.deps.overlay.register(GHOST_ID, el, origin);
      this.ghostRegistered = true;
    } else {
      this.deps.overlay.setWorldPos(GHOST_ID, origin);
    }
    this.deps.invalidate();
  }

  /** True while the tool ghost is on screen (gate/introspection probe). */
  get ghostVisible(): boolean {
    return this.ghostGroup?.visible === true;
  }

  private ensureGhost(): void {
    if (this.ghostGroup) return;
    const mat = new THREE.MeshBasicMaterial({
      color: palette.hoverAccent(),
      transparent: true,
      opacity: GHOST_OPACITY,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(SIZE, SIZE), mat);
    mesh.renderOrder = RENDER_ORDER.DATUM_FILL;
    const outlineMat = new THREE.LineBasicMaterial({
      color: palette.hoverAccent(),
      transparent: true,
      opacity: OUTLINE_OPACITY,
    });
    const outline = new THREE.LineLoop(quadOutlineGeometry(), outlineMat);
    outline.renderOrder = RENDER_ORDER.DATUM_OUTLINE;
    const group = new THREE.Group();
    group.name = "datumGhost";
    group.matrixAutoUpdate = false;
    group.add(mesh, outline);
    this.root.add(group);

    const el = document.createElement("div");
    el.dataset.datumGhostLabel = "1";
    const s = el.style;
    s.font = "700 12px var(--font-ui)";
    s.padding = "3px 7px";
    s.borderRadius = "5px";
    s.whiteSpace = "nowrap";
    s.pointerEvents = "none";
    s.background = "var(--color-sel-bg)";
    s.color = "var(--color-sel-text)";
    this.deps.overlayEl.appendChild(el);

    this.ghostGroup = group;
    this.ghostMat = mat;
    this.ghostMesh = mesh;
    this.ghostOutline = outline;
    this.ghostOutlineMat = outlineMat;
    this.ghostEl = el;
  }

  /**
   * Per-frame: hold every quad at a constant on-screen size. Each datum has its
   * OWN origin (unlike the plane picker's shared one), so `worldPerPixel` is
   * evaluated per group. `height` is the viewport height in CSS px.
   */
  update(camera: THREE.Camera, height: number): void {
    for (const q of this.quads.values()) {
      this.rescale(q.group, q.basis, camera, height);
    }
    if (this.ghostGroup?.visible) {
      this.rescale(this.ghostGroup, this.ghostBasis, camera, height);
    }
  }

  /**
   * Re-compose `group.matrix` as basis · uniform-scale. The UN-scaled `basis` is
   * kept per entry precisely so this is idempotent — reading the scale back out
   * of the composed matrix each frame would compound it.
   */
  private rescale(
    group: THREE.Group,
    basis: THREE.Matrix4,
    camera: THREE.Camera,
    height: number,
  ): void {
    this._origin.setFromMatrixPosition(basis);
    const scale = (worldPerPixel(camera, this._origin, height) * DATUM_PX) / SIZE;
    this._scale.setScalar(scale);
    group.matrix.copy(basis).scale(this._scale);
    group.matrixWorldNeedsUpdate = true;
    group.updateMatrixWorld(true);
  }

  /**
   * The datum id under `raycaster`, or null. Flushes the group world matrices
   * first (matrixAutoUpdate is off) so a raycast issued outside the render loop
   * — e.g. straight after `syncDatums` — still resolves.
   */
  hitTest(raycaster: THREE.Raycaster): string | null {
    if (this.quads.size === 0) return null;
    this.root.updateMatrixWorld(true);
    const meshes: THREE.Object3D[] = [];
    for (const q of this.quads.values()) meshes.push(q.mesh);
    const hit = raycaster.intersectObjects(meshes, false)[0];
    return hit ? ((hit.object.userData.datumId as string | undefined) ?? null) : null;
  }

  dispose(): void {
    for (const q of this.quads.values()) this.destroyQuad(q);
    this.quads.clear();
    if (this.ghostRegistered) this.deps.overlay.unregister(GHOST_ID);
    this.ghostRegistered = false;
    this.ghostMesh?.geometry.dispose();
    this.ghostMat?.dispose();
    this.ghostOutline?.geometry.dispose();
    this.ghostOutlineMat?.dispose();
    this.ghostEl?.remove();
    this.ghostGroup = null;
    this.ghostMesh = null;
    this.ghostMat = null;
    this.ghostOutline = null;
    this.ghostOutlineMat = null;
    this.ghostEl = null;
    this.deps.root.remove(this.root);
  }
}
