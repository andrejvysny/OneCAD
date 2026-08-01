/*
 * ViewportEngine — imperative Three.js orchestrator (NO react-three-fiber).
 *
 * Owns the scene graph, renderer, camera rig, controls, grid and HTML overlay.
 * Rendering is ON-DEMAND: invalidate() marks the frame dirty and schedules a
 * single rAF; while idle NO frame is scheduled and NO rendering happens. Camera
 * tweens keep scheduling frames until they finish, then the loop goes quiet.
 *
 * Lifecycle is StrictMode-safe: init() and dispose() are idempotent and a
 * dispose() that races an in-flight async init() still fully releases GPU state.
 *
 * Scene graph (see README.md — Z-UP RIGHT-HANDED is a HARD INVARIANT):
 *   scene
 *   ├── HemisphereLight + headlight DirectionalLight (follows the camera)
 *   ├── GridPlane            (world XY, Z=0)
 *   ├── bodiesRoot           (mesh ingestion — F-WP5)
 *   ├── sketchRoot           (sketch entities — later WP)
 *   └── interactionRoot      (previews / gizmos — later WP)
 */
import * as THREE from "three";
import { createRenderer, type RendererHandle } from "./renderer";
import { CameraRig, type ProjectionKind } from "./CameraRig";
import { CadOrbitControls } from "./CadOrbitControls";
import type { DevicePref, InputDevice } from "./navInput";
import { GridPlane } from "./GridPlane";
import { OriginTriad } from "./OriginTriad";
import { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { palette } from "./palette";
import { Picker, linePickThreshold, type PickHit, type PickModifiers } from "./Picker";
import { HighlightLayer } from "./HighlightLayer";
import {
  SketchStaticLayer,
  sketchStaticHitKey,
  type SketchStaticHit,
} from "./SketchStaticLayer";
import { RegionPickLayer } from "./RegionPickLayer";
import { flushDisposals } from "../mesh/meshRegistry";
import type { MeshEntry } from "../mesh/meshRegistry";
import type { EntityRef } from "@/stores/selectionStore";
import { SketchObject } from "./SketchObject";
import { SnapIndicator } from "./SnapIndicator";
import { planeGeometry, worldToPlanePoint, type Point2 } from "./sketchBasis";
import type { SketchEntity, SketchPlane, SketchRegion, SketchSolveStatus } from "@/ipc/types";
import type { SnapResult } from "@/tools/sketch/snapEngine";
import type { DraftEntity } from "@/tools/sketch/toolMachine";
import { PreviewMesh } from "./PreviewMesh";
import { DragHandle } from "./DragHandle";
import { RevolvePreview, type AxisCandidate } from "./RevolvePreview";
import { PlanePicker, type PickablePlane } from "./PlanePicker";
import { DatumLayer, datumGhostPlane, type DatumVisual } from "./DatumLayer";
import { GhostLayer } from "./GhostLayer";
import type { LatheAxis } from "@/tools/preview/lathePreview";
import type { GhostTransform } from "@/tools/preview/patternPreview";
import { buildBodyObject, createBodyMaterials, type BodyMaterials, type BodyObjectHandle } from "./BodyObject";
import type { PrismProfile } from "@/tools/preview/prismPreview";
import type { Vec3 } from "@/tools/preview/depthProjection";

const FRAME_RING = 240; // ?vpdebug frame-interval ring buffer capacity

export interface EngineInitOptions {
  experimentalWebGpu?: boolean;
  /** ?vpdebug — expose window.__vpFrames + a debug overlay label. */
  debug?: boolean;
  gridVisible?: boolean;
  /** Navigation seams — supplied by the bridge so the engine stays store-agnostic. */
  getDevicePref?: () => DevicePref;
  isDragActive?: () => boolean;
  onDeviceChange?: (device: InputDevice) => void;
}

/** Store-wiring seam the viewport bridge supplies for picking (engine stays store-agnostic). */
export interface PickHandlers {
  /** Picking is live only when this returns true (model + select mode). */
  isActive: () => boolean;
  onHover: (hit: PickHit | null, clientX: number, clientY: number, alt: boolean) => void;
  onPick: (hit: PickHit | null, mods: PickModifiers, clientX: number, clientY: number) => void;
}

const MAX_DPR = 2;
/** Generous pick radius for always-visible sketch curves (easier than body edges). */
const SKETCH_PICK_PX = 8;
const Z0_PLANE = new THREE.Plane(new THREE.Vector3(0, 0, 1), 0);

export class ViewportEngine {
  // Lifecycle
  private initialized = false;
  private disposed = false;

  // DOM / renderer. The engine OWNS its canvas (created per init) so that
  // forceContextLoss() on dispose is safe: React StrictMode remounts get a
  // fresh canvas + fresh WebGL context instead of a permanently-lost one.
  private container: HTMLElement | null = null;
  private canvas: HTMLCanvasElement | null = null;
  private rendererHandle: RendererHandle | null = null;
  isWebGPU = false;
  // Context-loss recovery (WKWebView drops GL contexts on GPU switch/sleep):
  // preventDefault on loss is REQUIRED for the browser to ever restore it.
  private readonly onContextLost = (e: Event): void => {
    if (!this.disposed) e.preventDefault();
  };
  private readonly onContextRestored = (): void => {
    this.invalidate();
  };

  // Scene graph
  private scene = new THREE.Scene();
  private rig = new CameraRig(76);
  private controls: CadOrbitControls | null = null;
  private grid: GridPlane | null = null;
  private triad: OriginTriad | null = null;
  private readonly overlayDriver = new HtmlOverlayDriver();
  readonly bodiesRoot = new THREE.Group();
  readonly sketchRoot = new THREE.Group();
  readonly interactionRoot = new THREE.Group();
  readonly previewRoot = new THREE.Group(); // L2 preview body (lit like a real body)
  private headlight: THREE.DirectionalLight | null = null;

  // Model-tool previews (F-WP7).
  private previewMesh: PreviewMesh | null = null; // L1 extrude prism
  private dragHandle: DragHandle | null = null;
  private revolvePreview: RevolvePreview | null = null; // L1 lathe + axis picker
  private ghostLayer: GhostLayer | null = null; // L1 pattern / mirror clones
  private regionPickLayer: RegionPickLayer | null = null; // multi-region extrude/revolve pick
  private planePicker: PlanePicker | null = null; // origin-plane pick gizmo
  private previewMaterials: BodyMaterials | null = null;
  // One L2 preview body per armed region, keyed by bodyId (N==1 single-region;
  // N in Wave 2 multi-select). The materials are shared, so a Cut tint hits all.
  private previewBodies = new Map<string, BodyObjectHandle>();
  /** Committed body groups temporarily hidden under exact replacement previews. */
  private previewHiddenBodies = new Map<THREE.Object3D, boolean>();
  private extrudePreviewCut = false; // Cut tint sticks across preview-body swaps

  // Picking + highlighting (mesh ingestion F-WP5).
  private picker: Picker | null = null;
  private highlights: HighlightLayer | null = null;
  private pickHandlers: PickHandlers | null = null;

  // Always-visible document sketches in MODEL mode (Fusion-style static layer).
  private sketchStatic: SketchStaticLayer | null = null;

  // Always-visible datum (reference) planes + the datum tool's ghost (DATUM W1).
  private datumLayer: DatumLayer | null = null;

  // Sketch mode (F-WP6).
  private overlayEl: HTMLElement | null = null;
  private sketch: SketchObject | null = null;
  private snapIndicator: SnapIndicator | null = null;
  private sketchPlane: SketchPlane | null = null;
  private savedView: ReturnType<CadOrbitControls["getViewState"]> | null = null;
  private ghostEl: HTMLElement | null = null;
  private ghostRegistered = false;
  private readonly _plane = new THREE.Plane();

  // Render-on-demand
  private dirty = true;
  private rafId = 0;
  private rafPending = false;
  private frames = 0;
  private debug = false;
  // ?vpdebug frame-interval ring (ms between rendered frames) for the 60fps gate.
  private readonly frameTimes = new Float64Array(FRAME_RING);
  private frameTimeCount = 0;
  private lastFrameAt = 0;

  // Observers / listeners
  private resizeObserver: ResizeObserver | null = null;
  private readonly cameraListeners = new Set<() => void>();

  private readonly raycaster = new THREE.Raycaster();

  get overlay(): HtmlOverlayDriver {
    return this.overlayDriver;
  }

  get frameCount(): number {
    return this.frames;
  }

  async init(
    container: HTMLElement,
    overlayEl: HTMLElement,
    opts: EngineInitOptions = {},
  ): Promise<void> {
    if (this.initialized || this.disposed) return;
    this.container = container;
    this.overlayEl = overlayEl;
    this.canvas = this.createCanvas(container);
    this.canvas.addEventListener("webglcontextlost", this.onContextLost);
    this.canvas.addEventListener("webglcontextrestored", this.onContextRestored);
    this.debug = opts.debug ?? false;

    this.buildScene();

    const handle = await createRenderer(this.canvas, {
      experimentalWebGpu: opts.experimentalWebGpu,
    });
    // dispose() may have run during the await — release and bail.
    if (this.disposed) {
      handle.dispose();
      return;
    }
    this.rendererHandle = handle;
    this.isWebGPU = handle.isWebGPU;

    this.grid = new GridPlane({
      minor: palette.gridMinor(),
      major: palette.gridMajor(),
      clear: palette.clear(),
    });
    this.grid.setVisible(opts.gridVisible ?? false);
    this.scene.add(this.grid.object3D);

    // Always visible — NOT gated by gridVisible, so the viewport never loses
    // its sense of orientation.
    this.triad = new OriginTriad({ x: palette.axisX(), y: palette.axisY(), z: palette.axisZ() });
    this.scene.add(this.triad.object3D);

    this.controls = new CadOrbitControls({
      rig: this.rig,
      element: this.canvas,
      onChange: this.handleCameraChanged,
      getBounds: this.getSceneBounds,
      // Orbit gating: an LMB drag that STARTS on pickable geometry (or the
      // extrude drag handle) selects/drags (no orbit); empty space orbits.
      hitTest: (x, y) =>
        (this.picker?.hasHitAt(x, y) ?? false) ||
        this.hitExtrudeHandle(x, y) ||
        (this.planePicker?.visible === true && this.planePickerHitTest(x, y) !== null),
      getDevicePref: opts.getDevicePref,
      isDragActive: opts.isDragActive,
      onDeviceChange: opts.onDeviceChange,
    });

    this.highlights = new HighlightLayer({
      root: this.interactionRoot,
      invalidate: () => this.invalidate(),
    });
    this.picker = new Picker({
      canvas: this.canvas,
      getCamera: () => this.rig.getCamera(),
      getRoot: () => this.bodiesRoot,
      getViewportHeight: () => this.viewportSize().height,
      getFocusDistance: () => this.controls?.getDistance() ?? 260,
      invalidate: () => this.invalidate(),
      isActive: () => this.pickHandlers?.isActive() ?? false,
      onHover: (hit, x, y, alt) => this.pickHandlers?.onHover(hit, x, y, alt),
      onPick: (hit, mods, x, y) => this.pickHandlers?.onPick(hit, mods, x, y),
      // Secondary hover token: the always-visible sketch — or, failing that, the
      // datum plane — under the pointer (only consulted when there is no body
      // hit). A datum must be folded in here too, else moving between two datums
      // in empty space produces no key change and hover never fires.
      secondaryHoverKey: (x, y) => {
        const hit = this.sketchStaticHitTest(x, y);
        if (hit) return sketchStaticHitKey(hit);
        const datumId = this.datumHitTest(x, y);
        return datumId ? `datum:${datumId}` : null;
      },
    });

    if (this.debug) this.setupDebugOverlay(overlayEl);

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(container);

    this.initialized = true;
    this.resize();
    this.controls.homeView(false); // framed iso, no intro animation
    this.invalidate();

    if (this.debug) {
      (window as unknown as { __vpEngine?: ViewportEngine }).__vpEngine = this;
    }
  }

  /** Debug-only introspection (?vpdebug). */
  debugSnapshot(): Record<string, unknown> {
    const cam = this.rig.getCamera();
    const bounds = this.getSceneBounds();
    return {
      isWebGPU: this.isWebGPU,
      frames: this.frames,
      camPos: cam.position.toArray(),
      camNear: (cam as THREE.PerspectiveCamera).near,
      camFar: (cam as THREE.PerspectiveCamera).far,
      target: this.controls?.getTarget().toArray(),
      distance: this.controls?.getDistance(),
      bodiesChildren: this.bodiesRoot.children.length,
      sceneChildren: this.scene.children.length,
      gridVisible: this.grid?.object3D.visible,
      bounds: bounds
        ? { min: bounds.min.toArray(), max: bounds.max.toArray() }
        : null,
    };
  }

  private createCanvas(container: HTMLElement): HTMLCanvasElement {
    const canvas = document.createElement("canvas");
    canvas.style.position = "absolute";
    canvas.style.inset = "0";
    canvas.style.width = "100%";
    canvas.style.height = "100%";
    canvas.style.display = "block";
    // Without this the browser may claim touch pan/pinch before pointer events
    // fire, fighting the controller's own 2-pointer handling.
    canvas.style.touchAction = "none";
    container.appendChild(canvas);
    return canvas;
  }

  private buildScene(): void {
    const hemi = new THREE.HemisphereLight(
      new THREE.Color(1, 1, 1),
      new THREE.Color(0.4, 0.43, 0.48),
      1.15,
    );
    this.scene.add(hemi);

    this.headlight = new THREE.DirectionalLight(new THREE.Color(1, 1, 1), 0.75);
    this.scene.add(this.headlight);
    this.scene.add(this.headlight.target);

    this.bodiesRoot.name = "bodiesRoot";
    this.sketchRoot.name = "sketchRoot";
    this.interactionRoot.name = "interactionRoot";
    this.previewRoot.name = "previewRoot";
    this.scene.add(this.bodiesRoot, this.sketchRoot, this.interactionRoot, this.previewRoot);
  }

  private setupDebugOverlay(overlayEl: HTMLElement): void {
    const label = document.createElement("div");
    label.textContent = "origin";
    label.dataset.vpDebugLabel = "1";
    label.style.font = "11px ui-monospace, monospace";
    label.style.padding = "1px 4px";
    label.style.borderRadius = "3px";
    label.style.background = "rgba(0,0,0,0.55)";
    label.style.color = "rgb(240,240,240)";
    label.style.pointerEvents = "none";
    overlayEl.appendChild(label);
    this.overlayDriver.register("__debug_origin", label, new THREE.Vector3(0, 0, 0));
  }

  // ---- Render-on-demand ----

  invalidate(): void {
    if (this.disposed) return;
    this.dirty = true;
    this.scheduleFrame();
  }

  private scheduleFrame(): void {
    if (this.rafPending || this.disposed || !this.initialized) return;
    this.rafPending = true;
    this.rafId = requestAnimationFrame(this.tick);
  }

  private tick = (nowMs: number): void => {
    this.rafPending = false;
    if (this.disposed) return;
    const animating = this.controls ? this.controls.update(nowMs) : false;
    if (this.dirty || animating) {
      this.renderFrame();
      this.dirty = false;
    }
    if (animating) this.scheduleFrame();
  };

  private renderFrame(): void {
    if (!this.rendererHandle || !this.controls) return;
    const camera = this.rig.getCamera();

    // Headlight tracks the camera (a simple headlight, projection-agnostic).
    if (this.headlight) {
      this.headlight.position.copy(camera.position);
      this.headlight.target.position.copy(this.controls.getTarget());
      this.headlight.target.updateMatrixWorld();
    }
    if (this.grid) {
      this.grid.update(this.controls.getTarget(), this.controls.getDistance());
    }

    const { width, height } = this.viewportSize();
    const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
    if (this.triad) {
      // Constant on-screen size: the triad reads the camera itself (its own
      // depth from the origin, which orbit distance does not give once panned).
      this.triad.update(camera, width, height, dpr);
    }
    if (this.planePicker?.visible) {
      this.planePicker.update(camera, height);
    }
    // Datum quads hold a constant on-screen size (they ARE the hit geometry).
    this.datumLayer?.update(camera, height);
    if (this.sketch) {
      this.sketch.update(width * dpr, height * dpr, this.controls.getTarget(), this.controls.getDistance());
    }
    // Keep the extrude handle a constant screen size across zoom/orbit.
    if (this.dragHandle) this.dragHandle.setScale(this.planePixelWorld());
    this.rendererHandle.renderer.render(this.scene, camera);
    this.overlayDriver.update(camera, width, height);
    // Double-buffer disposal: geometries swapped out earlier are freed now, one
    // frame later, so nothing that referenced them this frame reads freed data.
    flushDisposals();
    this.frames++;
    if (this.debug) {
      const now = performance.now();
      if (this.lastFrameAt > 0) {
        this.frameTimes[this.frameTimeCount % FRAME_RING] = now - this.lastFrameAt;
        this.frameTimeCount++;
      }
      this.lastFrameAt = now;
      const w = window as unknown as { __vpFrames?: number; __vpFrameTimes?: number[] };
      w.__vpFrames = this.frames;
      // Expose the ring newest-last for the 60fps gate (p95 frame interval).
      const n = Math.min(this.frameTimeCount, FRAME_RING);
      const out: number[] = new Array(n);
      for (let i = 0; i < n; i++) out[i] = this.frameTimes[(this.frameTimeCount - n + i) % FRAME_RING];
      w.__vpFrameTimes = out;
    }
  }

  /** Reset the ?vpdebug frame-interval ring (call before a measured drag). */
  resetFrameTimes(): void {
    this.frameTimeCount = 0;
    this.lastFrameAt = 0;
    if (this.debug) {
      (window as unknown as { __vpFrameTimes?: number[] }).__vpFrameTimes = [];
    }
  }

  private viewportSize(): { width: number; height: number } {
    const c = this.container;
    return {
      width: c ? c.clientWidth : 1,
      height: c ? c.clientHeight : 1,
    };
  }

  private resize(): void {
    if (!this.rendererHandle || !this.canvas) return;
    const { width, height } = this.viewportSize();
    if (width === 0 || height === 0) return;
    const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
    this.rendererHandle.renderer.setPixelRatio(dpr);
    this.rendererHandle.renderer.setSize(width, height, false);
    this.rig.setAspect(width / height);
    this.invalidate();
  }

  // ---- Camera changes / listeners ----

  private handleCameraChanged = (): void => {
    this.dirty = true;
    for (const cb of this.cameraListeners) cb();
    this.scheduleFrame();
  };

  onCameraChanged(cb: () => void): () => void {
    this.cameraListeners.add(cb);
    return () => this.cameraListeners.delete(cb);
  }

  getCameraQuaternion(out: THREE.Quaternion): THREE.Quaternion {
    return out.copy(this.rig.getCamera().quaternion);
  }

  getViewDirection(): THREE.Vector3 {
    return this.controls
      ? this.controls.getViewDirection()
      : new THREE.Vector3(0, 0, 1);
  }

  getCameraDistance(): number {
    return this.controls?.getDistance() ?? 260;
  }

  /** True when the camera looks nearly straight down a world axis (e.g. TOP). */
  private isViewAxisAligned(thresholdDeg = 5): boolean {
    const dir = this.getViewDirection(); // unit vector already
    const m = Math.max(Math.abs(dir.x), Math.abs(dir.y), Math.abs(dir.z));
    return m >= Math.cos((thresholdDeg * Math.PI) / 180);
  }

  // ---- Public actions (bridge / shell) ----

  setProjection(p: ProjectionKind): void {
    if (this.rig.projection === p) return;
    this.rig.setProjection(p);
    this.controls?.applyToRig();
    this.handleCameraChanged();
  }

  setGridVisible(visible: boolean): void {
    this.grid?.setVisible(visible);
    this.invalidate();
  }

  homeView(): void {
    this.controls?.homeView(true);
  }

  fitView(): void {
    this.controls?.fitView();
  }

  /**
   * Frame exactly `bodyIds` (zoom-to-selection, W3). Falls back to the
   * whole-scene fit when none of them has visible geometry — a stale or hidden
   * id must never leave the camera pointing at nothing.
   */
  fitToBodies(bodyIds: readonly string[]): void {
    this.controls?.fitView(this.getBoundsForBodies(bodyIds) ?? undefined);
  }

  /**
   * World bounds of the VISIBLE body groups among `bodyIds` (null = none).
   *
   * The `child.visible` filter is load-bearing: `Box3.setFromObject` walks
   * `children` unconditionally (three's `expandByObject` recurses without
   * consulting `visible`), so a body hidden by the tree eye or by isolation
   * would otherwise pull the frame out to geometry the user cannot see.
   */
  getBoundsForBodies(bodyIds: readonly string[]): THREE.Box3 | null {
    const wanted = new Set(bodyIds);
    const box = new THREE.Box3();
    const one = new THREE.Box3();
    for (const child of this.bodiesRoot.children) {
      if (!child.visible) continue;
      if (!wanted.has(String(child.userData.bodyId ?? ""))) continue;
      box.union(one.setFromObject(child));
    }
    return box.isEmpty() ? null : box;
  }

  /**
   * True while an armed preview holds committed bodies hidden (see
   * `setPreviewReplacedBodyIds`). Read by the isolate toggle, which must not
   * touch body visibility that the preview is going to restore from a snapshot.
   */
  hasPreviewHiddenBodies(): boolean {
    return this.previewHiddenBodies.size > 0;
  }

  snapToViewDirection(dir: THREE.Vector3): void {
    this.controls?.snapToViewDirection(dir);
  }

  private getSceneBounds = (): THREE.Box3 | null => {
    const box = new THREE.Box3().setFromObject(this.bodiesRoot);
    return box.isEmpty() ? null : box;
  };

  /** Raycast a client point onto the world Z=0 plane. Null if it misses. */
  screenToWorldOnZ0(clientX: number, clientY: number): THREE.Vector3 | null {
    if (!this.canvas) return null;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    const hit = new THREE.Vector3();
    return this.raycaster.ray.intersectPlane(Z0_PLANE, hit) ? hit : null;
  }

  // ---- Sketch mode (F-WP6) ----

  /**
   * Enter sketch mode: build the sketch presence + snap indicator, save the
   * current camera view, and animate to look straight down the plane normal.
   * Projection (ortho) is owned by the store/controller.
   */
  enterSketch(plane: SketchPlane, entities: SketchEntity[], status: SketchSolveStatus): void {
    if (this.disposed) return;
    if (!this.sketch) {
      this.sketch = new SketchObject({ sketchRoot: this.sketchRoot, invalidate: () => this.invalidate() });
    }
    if (!this.snapIndicator && this.overlayEl) {
      this.snapIndicator = new SnapIndicator({
        interactionRoot: this.interactionRoot,
        overlay: this.overlayDriver,
        overlayEl: this.overlayEl,
        invalidate: () => this.invalidate(),
      });
    }
    this.sketchPlane = plane;
    this.sketch.setSession(plane, entities, status);
    this.snapIndicator?.setPlane(plane);

    this.savedView = this.controls?.getViewState() ?? null;
    const normal = new THREE.Vector3().fromArray(plane.normal);
    const origin = new THREE.Vector3().fromArray(plane.origin);
    const dist = this.controls?.getDistance() ?? 260;
    this.controls?.viewAlongNormal(normal, origin, dist, true);
    this.invalidate();
  }

  /** Refresh the committed sketch geometry after a solve/commit. */
  updateSketchSession(plane: SketchPlane, entities: SketchEntity[], status: SketchSolveStatus): void {
    this.sketchPlane = plane;
    this.sketch?.setSession(plane, entities, status);
    // A committed edit invalidates any hover-time doomed-piece ghost (it was drawn
    // against the pre-edit geometry) — clear it so it never lingers stale.
    this.sketch?.setTrimGhost(null);
    this.snapIndicator?.setPlane(plane);
  }

  setSketchPreview(drafts: DraftEntity[]): void {
    this.sketch?.setPreview(drafts);
  }

  /** Show/clear the destructive trim doomed-piece ghost (Trim tool hover). */
  setSketchTrimGhost(draft: DraftEntity | null): void {
    this.sketch?.setTrimGhost(draft);
  }

  setSketchSelection(ids: Iterable<string>): void {
    this.sketch?.setSelection(ids);
  }

  setSketchHover(ids: Iterable<string>): void {
    this.sketch?.setHover(ids);
  }

  setSketchSnap(snap: SnapResult | null, showHints: boolean): void {
    if (!this.snapIndicator) return;
    if (snap) this.snapIndicator.show(snap, showHints);
    else this.snapIndicator.hide();
  }

  /** Show/hide the auto-constraint ghost glyph (H/V) at a plane point. */
  setSketchGhost(label: string | null, at: Point2 | null): void {
    if (!this.overlayEl || !this.sketchPlane) return;
    if (!this.ghostEl) {
      this.ghostEl = document.createElement("div");
      this.ghostEl.dataset.sketchGhost = "1";
      const s = this.ghostEl.style;
      s.font = "600 11px var(--font-mono)";
      s.padding = "1px 5px";
      s.borderRadius = "4px";
      s.pointerEvents = "none";
      s.background = "var(--color-sel-bg)";
      s.color = "var(--color-accent)";
      s.marginLeft = "14px";
      s.marginTop = "14px";
      this.overlayEl.appendChild(this.ghostEl);
    }
    if (!label || !at) {
      this.ghostEl.style.display = "none";
      return;
    }
    const world = new THREE.Vector3();
    const p = this.sketchPlane;
    world.fromArray(p.origin).addScaledVector(new THREE.Vector3().fromArray(p.xAxis), at.x).addScaledVector(new THREE.Vector3().fromArray(p.yAxis), at.y);
    if (!this.ghostRegistered) {
      this.overlayDriver.register("__sketch_ghost", this.ghostEl, world);
      this.ghostRegistered = true;
    } else {
      this.overlayDriver.setWorldPos("__sketch_ghost", world);
    }
    this.ghostEl.textContent = label;
    this.ghostEl.style.display = "";
    this.invalidate();
  }

  /** LMB orbit suppression while a drawing tool owns the pointer. */
  setSketchDrawingActive(active: boolean): void {
    this.controls?.setLmbOrbitSuppressed(active);
  }

  /** Suppress LMB orbit (a modal model tool owns the drag, e.g. fillet radius). */
  setOrbitSuppressed(active: boolean): void {
    this.controls?.setLmbOrbitSuppressed(active);
  }

  /** One-shot body/edge pick regardless of the picking mode (boolean tool pick). */
  probePick(clientX: number, clientY: number): PickHit | null {
    return this.picker?.probe(clientX, clientY) ?? null;
  }

  /** Raycast a client point onto the current sketch plane → plane (u,v). */
  screenToPlane(clientX: number, clientY: number): Point2 | null {
    if (!this.canvas || !this.sketchPlane) return null;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    planeGeometry(this.sketchPlane, this._plane);
    const hit = new THREE.Vector3();
    if (!this.raycaster.ray.intersectPlane(this._plane, hit)) return null;
    return worldToPlanePoint(this.sketchPlane, hit);
  }

  /** World units per screen pixel at the plane (sizes snap thresholds). */
  planePixelWorld(): number {
    const height = Math.max(this.viewportSize().height, 1);
    const dist = this.controls?.getDistance() ?? 260;
    const cam = this.rig.getCamera();
    if ((cam as THREE.PerspectiveCamera).isPerspectiveCamera) {
      const fov = (cam as THREE.PerspectiveCamera).fov;
      return (2 * dist * Math.tan((fov * Math.PI) / 360)) / height;
    }
    const oc = cam as THREE.OrthographicCamera;
    return (oc.top - oc.bottom) / height;
  }

  /** Exit sketch mode: tear down the presence and restore the saved camera. */
  exitSketch(): void {
    this.sketch?.dispose();
    this.sketch = null;
    this.snapIndicator?.dispose();
    this.snapIndicator = null;
    this.sketchPlane = null;
    if (this.ghostRegistered) {
      this.overlayDriver.unregister("__sketch_ghost");
      this.ghostRegistered = false;
    }
    this.ghostEl?.remove();
    this.ghostEl = null;
    this.controls?.setLmbOrbitSuppressed(false);
    if (this.savedView && this.controls) {
      this.controls.setView(this.savedView, true);
      this.savedView = null;
    }
    this.invalidate();
  }

  // ---- Model-tool previews (F-WP7) ----

  /**
   * Show the extrude L1 preview + drag handle for a single region: the unit prism
   * on `plane`, positioned at `centroid` pointing along `normal`. Idempotent —
   * reuses the objects across arms. Delegates to the plural form (Wave 2).
   */
  showExtrudePreview(plane: SketchPlane, profile: PrismProfile, centroid: Vec3, normal: Vec3): void {
    this.showExtrudePreviews(plane, [profile], centroid, normal);
  }

  /**
   * Show the extrude L1 preview + drag handle for N regions (Wave 2 multi-select):
   * one unit prism per profile on `plane`, ONE drag handle at the combined
   * `centroid` pointing along `normal`. Idempotent — reuses the objects across arms.
   */
  showExtrudePreviews(plane: SketchPlane, profiles: PrismProfile[], centroid: Vec3, normal: Vec3): void {
    if (this.disposed) return;
    if (!this.previewMesh) {
      this.previewMesh = new PreviewMesh({ root: this.interactionRoot, invalidate: () => this.invalidate() });
    }
    if (!this.dragHandle) {
      this.dragHandle = new DragHandle({ root: this.interactionRoot, invalidate: () => this.invalidate() });
    }
    this.previewMesh.setProfiles(plane, profiles);
    this.previewMesh.setTint(this.extrudePreviewCut);
    this.previewMesh.setVisible(true);
    this.dragHandle.setAnchor(new THREE.Vector3().fromArray(centroid), new THREE.Vector3().fromArray(normal));
    this.dragHandle.setScale(this.planePixelWorld());
    this.dragHandle.setVisible(true);
    this.invalidate();
  }

  /** Set the live extrude depth on the L1 prism(s) (symmetric grows both ways). */
  setExtrudeDepth(depth: number, symmetric: boolean): void {
    this.previewMesh?.setDepth(depth, symmetric);
  }

  /**
   * Tint the LIVE op preview (Wave 2 boolean modes): "cut" recolors the L1 prism(s)
   * / lathe shell AND the shared L2 preview-body face material to the destructive
   * token; "normal" restores the accent. Sticky across preview swaps —
   * `setPreviewBody` re-applies it. Only one preview kind is live at a time, so
   * tinting every surface here is harmless — which is why this is `setPreviewTint`
   * and not an extrude-specific seam: it already served the lathe and the shared
   * L2 body before any other tool joined the kernel-preview lane.
   */
  setPreviewTint(tint: "normal" | "cut"): void {
    const cut = tint === "cut";
    this.extrudePreviewCut = cut;
    this.previewMesh?.setTint(cut);
    this.revolvePreview?.setTint(cut);
    if (this.previewMaterials) {
      this.previewMaterials.face.color.copy(cut ? palette.destructive() : palette.bodyNeutral());
    }
    this.invalidate();
  }

  /** Hover state on the drag handle (does not affect the prism). */
  setExtrudeHandleHover(hovered: boolean): void {
    this.dragHandle?.setHover(hovered);
  }

  /** True while the L1 extrude prism is visible (commit-reconcile + gate probe). */
  isExtrudePreviewVisible(): boolean {
    return this.previewMesh?.visible ?? false;
  }

  /** Hide the extrude L1 preview + handle (kept for reuse; disposed with the engine). */
  hideExtrudePreview(): void {
    this.previewMesh?.setVisible(false);
    this.dragHandle?.setVisible(false);
    this.dragHandle?.setHover(false);
  }

  // ---- Revolve L1 preview + axis picker ----

  private ensureRevolvePreview(): RevolvePreview {
    if (!this.revolvePreview) {
      this.revolvePreview = new RevolvePreview({ root: this.interactionRoot, invalidate: () => this.invalidate() });
    }
    return this.revolvePreview;
  }

  /** Enter axis-pick: show the candidate sketch-line axes on `plane`. */
  showRevolveAxisCandidates(plane: SketchPlane, candidates: AxisCandidate[]): void {
    if (this.disposed) return;
    const rp = this.ensureRevolvePreview();
    rp.setPlane(plane);
    rp.setCandidates(candidates);
    rp.setHover(null);
    rp.clearLathe();
    rp.setVisible(true);
  }

  /** Highlight one candidate axis (hover), or clear when null. */
  setRevolveAxisHover(seg: AxisCandidate | null): void {
    this.revolvePreview?.setHover(seg);
  }

  /** Show the lathe L1 shell for a chosen axis at `angleDeg`; hides the candidates. */
  showRevolvePreview(
    plane: SketchPlane,
    ring: [number, number][],
    axis: LatheAxis,
    angleDeg: number,
  ): void {
    if (this.disposed) return;
    const rp = this.ensureRevolvePreview();
    rp.setPlane(plane);
    rp.hideCandidates();
    rp.setHover({ a: axis.a, b: axis.b });
    rp.setLathe(ring, axis, angleDeg);
    rp.setVisible(true);
  }

  /** Rebuild the lathe shell at a new angle during the drag. */
  setRevolveAngle(ring: [number, number][], axis: LatheAxis, angleDeg: number): void {
    this.revolvePreview?.setLathe(ring, axis, angleDeg);
  }

  /** True while any revolve L1 preview (candidates or lathe) is visible. */
  isRevolvePreviewVisible(): boolean {
    return this.revolvePreview?.visible ?? false;
  }

  hideRevolvePreview(): void {
    this.revolvePreview?.setVisible(false);
    this.revolvePreview?.setHover(null);
    this.revolvePreview?.clearLathe();
  }

  // ---- Ghost preview (pattern / mirror L1) ----

  /** Show translucent clones of `entry`'s geometry at each transform (pattern/mirror L1). */
  showGhostPreview(entry: MeshEntry, transforms: GhostTransform[]): void {
    if (this.disposed) return;
    if (!this.ghostLayer) {
      this.ghostLayer = new GhostLayer({ root: this.interactionRoot, invalidate: () => this.invalidate() });
    }
    this.ghostLayer.show(entry, transforms);
  }

  hideGhostPreview(): void {
    this.ghostLayer?.hide();
  }

  /** True while any pattern/mirror ghost clones are visible. */
  isGhostPreviewVisible(): boolean {
    return this.ghostLayer?.visible ?? false;
  }

  // ---- Sketch plane picker (origin-plane pick gizmo, before entering a sketch) ----

  /**
   * Show/hide the three origin-plane pick quads. Lazy-constructs on first
   * show. If the camera is nearly axis-aligned (e.g. TOP view — a plane would
   * be edge-on / invisible) it nudges to the iso home view so all three
   * planes are visible.
   */
  setPlanePickerVisible(visible: boolean): void {
    if (this.disposed) return;
    if (visible) {
      if (!this.overlayEl) return; // not initialized yet
      if (!this.planePicker) {
        this.planePicker = new PlanePicker({
          root: this.interactionRoot,
          overlay: this.overlayDriver,
          overlayEl: this.overlayEl,
          invalidate: () => this.invalidate(),
        });
      }
      if (this.isViewAxisAligned()) this.controls?.homeView(true);
      this.planePicker.setVisible(true);
      // Size the quads now, not on the next frame: a hover/click can raycast
      // them before the render loop runs, and the quads ARE the hit geometry.
      this.planePicker.update(this.rig.getCamera(), this.viewportSize().height);
    } else {
      this.planePicker?.setHover(null);
      this.planePicker?.setVisible(false);
    }
    this.invalidate();
  }

  /** Hover the plane under a client point (updates the highlight + label chip). */
  planePickerHover(clientX: number, clientY: number): PickablePlane | null {
    if (!this.planePicker) return null;
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc) {
      this.planePicker.setHover(null);
      return null;
    }
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    const hit = this.planePicker.hitTest(this.raycaster);
    this.planePicker.setHover(hit);
    return hit?.kind ?? null;
  }

  /** Pure hit-test at a client point (no hover mutation) — also the orbit-gate probe. */
  planePickerHitTest(clientX: number, clientY: number): PickablePlane | null {
    if (!this.planePicker) return null;
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc) return null;
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    return this.planePicker.hitTest(this.raycaster)?.kind ?? null;
  }

  /** Drop the plane picker's hover highlight + label (a datum won the pointer). */
  clearPlanePickerHover(): void {
    this.planePicker?.setHover(null);
  }

  // ---- Datum (reference) planes — DATUM W1 ----
  //
  // Persistent document content, so the layer is always live in model mode (the
  // datumSync controller drives it from documentStore.datums). Deliberately NOT
  // in the orbit gate (SketchStaticLayer convention): clicking empty space near a
  // datum still orbits.

  /** The datum layer (lazy). Null before `init()` — there is no overlay host yet. */
  private ensureDatumLayer(): DatumLayer | null {
    if (this.disposed || !this.overlayEl) return null;
    if (!this.datumLayer) {
      this.datumLayer = new DatumLayer({
        root: this.interactionRoot,
        overlay: this.overlayDriver,
        overlayEl: this.overlayEl,
        invalidate: () => this.invalidate(),
      });
    }
    return this.datumLayer;
  }

  /** Reconcile the rendered datum planes against the projection (add/remove/move). */
  syncDatums(metas: readonly DatumVisual[]): void {
    const layer = this.ensureDatumLayer();
    if (!layer) return;
    layer.syncDatums(metas);
    // Size the quads now, not on the next frame: a hit-test can raycast them
    // before the render loop runs, and the quads ARE the hit geometry.
    layer.update(this.rig.getCamera(), this.viewportSize().height);
  }

  /** The datum plane under a client point, or null. Pure (no hover mutation). */
  datumHitTest(clientX: number, clientY: number): string | null {
    if (!this.datumLayer) return null;
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc || Math.abs(ndc.x) > 1 || Math.abs(ndc.y) > 1) return null;
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    return this.datumLayer.hitTest(this.raycaster);
  }

  setDatumHover(id: string | null): void {
    this.datumLayer?.setHover(id);
  }

  setDatumSelected(ids: readonly string[]): void {
    this.datumLayer?.setSelected(ids);
  }

  /**
   * Show the datum TOOL's live ghost quad for `baseKind` offset by `offset`, or
   * hide it with `baseKind: null`. The frame comes from `datumGhostPlane` — the
   * one place the frontend derives a datum basis, because nothing is committed
   * yet and there is no backend frame to read (see DatumLayer's module doc).
   */
  setDatumGhost(baseKind: PickablePlane | null, offset: number, label?: string): void {
    if (baseKind === null) {
      this.datumLayer?.setGhost(null);
      return;
    }
    const layer = this.ensureDatumLayer();
    if (!layer) return;
    layer.setGhost(datumGhostPlane(baseKind, offset), label);
    layer.update(this.rig.getCamera(), this.viewportSize().height);
  }

  /** True while the datum tool ghost is on screen (gate/introspection probe). */
  isDatumGhostVisible(): boolean {
    return this.datumLayer?.ghostVisible ?? false;
  }

  /** Raycast a client point onto an ARBITRARY sketch plane → plane (u,v). */
  screenToPlaneOn(plane: SketchPlane, clientX: number, clientY: number): Point2 | null {
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc) return null;
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    planeGeometry(plane, this._plane);
    const hit = new THREE.Vector3();
    if (!this.raycaster.ray.intersectPlane(this._plane, hit)) return null;
    return worldToPlanePoint(plane, hit);
  }

  /** True when a client point hits the extrude drag handle. */
  hitExtrudeHandle(clientX: number, clientY: number): boolean {
    if (!this.canvas || !this.dragHandle) return false;
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc) return false;
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    return this.dragHandle.raycast(this.raycaster);
  }

  /** Pointer ray in WORLD space, for the depth-projection math. */
  screenRay(clientX: number, clientY: number): { origin: Vec3; dir: Vec3 } | null {
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc) return null;
    this.raycaster.setFromCamera(ndc, this.rig.getCamera());
    const o = this.raycaster.ray.origin;
    const d = this.raycaster.ray.direction;
    return { origin: [o.x, o.y, o.z], dir: [d.x, d.y, d.z] };
  }

  private clientToNdc(clientX: number, clientY: number): THREE.Vector2 | null {
    if (!this.canvas) return null;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    return new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
  }

  /**
   * Swap the L2 preview body for `entry.bodyId` under previewRoot (keyed by bodyId
   * so Wave 2's N regions each keep their own preview body). Re-applies the current
   * Cut tint via the shared preview material.
   */
  setPreviewBody(entry: MeshEntry): void {
    if (this.disposed) return;
    if (!this.previewMaterials) this.previewMaterials = createBodyMaterials();
    const prev = this.previewBodies.get(entry.bodyId);
    if (prev) this.previewRoot.remove(prev.group);
    const handle = buildBodyObject(entry, this.previewMaterials);
    this.previewBodies.set(entry.bodyId, handle);
    this.previewRoot.add(handle.group);
    if (this.extrudePreviewCut) this.previewMaterials.face.color.copy(palette.destructive());
    this.invalidate();
  }

  /** Hide committed bodies replaced/deleted by the exact candidate operation. */
  setPreviewReplacedBodyIds(bodyIds: string[]): void {
    this.restorePreviewHiddenBodies();
    const ids = new Set(bodyIds);
    for (const child of this.bodiesRoot.children) {
      if (!ids.has(String(child.userData.bodyId ?? ""))) continue;
      this.previewHiddenBodies.set(child, child.visible);
      child.visible = false;
    }
    this.invalidate();
  }

  private restorePreviewHiddenBodies(): void {
    for (const [body, visible] of this.previewHiddenBodies) body.visible = visible;
    this.previewHiddenBodies.clear();
  }

  /** Remove ALL L2 preview bodies from the scene (registry entries dropped by the caller). */
  clearPreviewBody(): void {
    for (const handle of this.previewBodies.values()) this.previewRoot.remove(handle.group);
    this.previewBodies.clear();
    this.restorePreviewHiddenBodies();
    this.invalidate();
  }

  /** Register a DOM chip in the overlay, positioned at `world` each frame. */
  mountChip(id: string, el: HTMLElement, world: Vec3): void {
    if (!this.overlayEl) return;
    this.overlayEl.appendChild(el);
    this.overlayDriver.register(id, el, new THREE.Vector3().fromArray(world));
    this.invalidate();
  }

  moveChip(id: string, world: Vec3): void {
    this.overlayDriver.setWorldPos(id, new THREE.Vector3().fromArray(world));
    this.invalidate();
  }

  unmountChip(id: string, el: HTMLElement): void {
    this.overlayDriver.unregister(id);
    if (el.parentElement === this.overlayEl) el.remove();
    this.invalidate();
  }

  // ---- Always-visible sketch layer (Fusion-style static sketches, MODEL mode) ----

  /** The static sketch layer (lazy). The sketchStaticSync controller drives it. */
  getSketchStaticLayer(): SketchStaticLayer {
    if (!this.sketchStatic) {
      this.sketchStatic = new SketchStaticLayer({
        sketchRoot: this.sketchRoot,
        invalidate: () => this.invalidate(),
      });
    }
    return this.sketchStatic;
  }

  /** Exact region/whole-sketch target under a client point, or null. Deliberately
   *  NOT in the orbit gate — clicking empty space near a sketch still orbits. */
  sketchStaticHitTest(clientX: number, clientY: number): SketchStaticHit | null {
    if (!this.sketchStatic) return null;
    const ndc = this.clientToNdc(clientX, clientY);
    if (!ndc || Math.abs(ndc.x) > 1 || Math.abs(ndc.y) > 1) return null;
    const camera = this.rig.getCamera();
    this.raycaster.setFromCamera(ndc, camera);
    this.raycaster.params.Line = {
      threshold: linePickThreshold(camera, this.viewportSize().height, this.getCameraDistance(), SKETCH_PICK_PX),
    };
    return this.sketchStatic.hitTest(this.raycaster);
  }

  // ---- Picking / highlighting (F-WP5) ----

  /** Wire picking to the store-facing handlers (viewport bridge supplies these). */
  configurePicking(handlers: PickHandlers | null): void {
    this.pickHandlers = handlers;
  }

  /** Push the current hover + selection into the highlight layer. */
  setHighlightState(hover: EntityRef | null, selected: EntityRef[]): void {
    this.highlights?.setState(hover, selected);
  }

  /** Rebuild highlights against the current registry (after a mesh swap). */
  refreshHighlights(): void {
    this.highlights?.refresh();
  }

  // ---- Region pick (multi-region extrude/revolve selection) ----
  //
  // When a finished sketch yields several closed regions, the ModelToolController
  // shows one translucent fill per region on the sketch plane and hover/click-picks
  // one. Hit-testing is pure (tools/preview/regionPick) — the controller raycasts the
  // plane and resolves (u,v) → regionId, so this facade is show / tint / hide only.

  private ensureRegionPickLayer(): RegionPickLayer {
    if (!this.regionPickLayer) {
      this.regionPickLayer = new RegionPickLayer({
        root: this.interactionRoot,
        invalidate: () => this.invalidate(),
      });
    }
    return this.regionPickLayer;
  }

  /** Show the multi-region pick fills on `plane` (one translucent mesh per region). */
  showRegionPick(plane: SketchPlane, regions: SketchRegion[]): void {
    if (this.disposed) return;
    const layer = this.ensureRegionPickLayer();
    layer.setPlane(plane);
    layer.setRegions(regions);
    layer.setHover(null);
    layer.setVisible(true);
  }

  /** Tint the hovered region (or clear when null). */
  setRegionHover(id: string | null): void {
    this.regionPickLayer?.setHover(id);
  }

  /** Tint the currently-selected regions (Wave 2 multi-select; empty clears). */
  setRegionSelected(ids: string[]): void {
    this.regionPickLayer?.setSelected(ids);
  }

  /** True while the region-pick layer is visible (gate/introspection probe). */
  isRegionPickVisible(): boolean {
    return this.regionPickLayer?.visible ?? false;
  }

  /** Hide the region-pick layer (kept for reuse; disposed with the engine). */
  hideRegionPick(): void {
    this.regionPickLayer?.setVisible(false);
    this.regionPickLayer?.setHover(null);
  }

  // ---- Teardown ----

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.initialized = false;

    if (this.rafPending) cancelAnimationFrame(this.rafId);
    this.rafPending = false;

    this.resizeObserver?.disconnect();
    this.resizeObserver = null;

    this.picker?.dispose();
    this.picker = null;
    this.pickHandlers = null;

    this.sketch?.dispose();
    this.sketch = null;
    this.snapIndicator?.dispose();
    this.snapIndicator = null;
    this.ghostEl?.remove();
    this.ghostEl = null;
    this.overlayEl = null;

    this.highlights?.dispose();
    this.highlights = null;

    this.sketchStatic?.dispose();
    this.sketchStatic = null;

    this.datumLayer?.dispose();
    this.datumLayer = null;

    // Model-tool previews.
    this.previewMesh?.dispose();
    this.previewMesh = null;
    this.dragHandle?.dispose();
    this.dragHandle = null;
    this.revolvePreview?.dispose();
    this.revolvePreview = null;
    this.ghostLayer?.dispose();
    this.ghostLayer = null;
    this.regionPickLayer?.dispose();
    this.regionPickLayer = null;
    this.planePicker?.dispose();
    this.planePicker = null;
    for (const handle of this.previewBodies.values()) this.previewRoot.remove(handle.group);
    this.previewBodies.clear();
    this.restorePreviewHiddenBodies();
    this.previewMaterials?.dispose();
    this.previewMaterials = null;

    this.controls?.dispose();
    this.controls = null;

    this.grid?.dispose();
    this.grid = null;

    this.triad?.dispose();
    this.triad = null;

    this.overlayDriver.clear();
    this.cameraListeners.clear();

    this.rendererHandle?.dispose();
    this.rendererHandle = null;

    // Discard the owned canvas so a remount starts from a fresh context.
    this.canvas?.removeEventListener("webglcontextlost", this.onContextLost);
    this.canvas?.removeEventListener("webglcontextrestored", this.onContextRestored);
    this.canvas?.remove();
    this.canvas = null;
    this.container = null;
  }
}
