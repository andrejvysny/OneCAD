/*
 * MeshIngest — the app-level glue that turns backend `document-changed` events
 * into scene geometry (pull model, SCHEMA §7.6 / plan "Mesh transfer").
 *
 * Flow: onDocumentChanged → for each changed + VISIBLE body, fetch its MESH1
 * blob (getBodyMesh) → parse (zero-copy) → build registry entry → double-buffer
 * swap → (re)build its BodyObject in bodiesRoot → refresh highlights + invalidate.
 * Removed bodies are dropped from the registry and the scene. Visibility flips in
 * the document store toggle the BodyObject (lazy-loading a body the first time it
 * becomes visible).
 *
 * The engine stays graphics-only; this controller owns the client, the document
 * store subscription, the shared body material library, and the bodyId→BodyObject
 * map. `detach` clears the scene, disposes the registry (leak tripwire) + library.
 *
 * It is also the single owner of the two body view states (W3): the DISPLAY
 * MODE (face/edge child visibility per body — persisted on settingsStore) and
 * the transient ISOLATION mask (viewportStore, session-only). Both are applied
 * here rather than in the engine because the bodyId→handle map lives here.
 */
import type { CadClient } from "@/ipc/client";
import type { DocumentChange, Lod } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import type { ViewportEngine } from "../engine/ViewportEngine";
import { buildBodyObject, type BodyObjectHandle } from "../engine/BodyObject";
import { BodyMaterialLibrary } from "../engine/bodyMaterials";
import { coerceRenderMode, RENDER_MODES, type RenderModeDef } from "../engine/renderModes";
import { parseMeshPayload } from "./parseMeshPayload";
import { buildBodyObjects, disposeAll, remove, swap } from "./meshRegistry";

const DEFAULT_LOD: Lod = "coarse";

export class MeshIngest {
  private engine: ViewportEngine | null = null;
  private client: CadClient | null = null;
  private materials: BodyMaterialLibrary | null = null;
  private readonly bodyObjects = new Map<string, BodyObjectHandle>();
  private readonly unsubs: Array<() => void> = [];
  private meshRev = 0;
  /** Per-body monotonic fetch token — a resolved fetch older than the latest is discarded. */
  private readonly loadSeq = new Map<string, number>();
  private detached = false;
  /** Fires after a body's mesh finishes loading into the scene (F-WP7 commit reconcile). */
  private readonly bodyLoadedListeners = new Set<(bodyId: string) => void>();

  /** Subscribe to "a body finished loading into bodiesRoot". Returns an unsubscribe. */
  onBodyLoaded(cb: (bodyId: string) => void): () => void {
    this.bodyLoadedListeners.add(cb);
    return () => this.bodyLoadedListeners.delete(cb);
  }

  attach(engine: ViewportEngine, client: CadClient): void {
    this.engine = engine;
    this.client = client;
    this.materials = new BodyMaterialLibrary();
    this.detached = false;

    this.unsubs.push(client.onDocumentChanged((c) => this.onDocumentChanged(c)));

    // Visibility flips come through the document store (tree eye toggle).
    let prev = documentStore.getState().bodies;
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.bodies !== prev) {
          this.onVisibilityChanged(prev, s.bodies);
          prev = s.bodies;
        }
      }),
    );

    // Dim the shared body face material while sketching (focus cue), restore on
    // exit. Covers attaching mid-sketch (e.g. a re-mount) by applying immediately
    // rather than waiting for the next mode transition.
    let prevMode = toolStore.getState().mode;
    this.unsubs.push(
      toolStore.subscribe((s) => {
        if (s.mode !== prevMode) {
          this.setDimmed(s.mode === "sketch");
          prevMode = s.mode;
        }
      }),
    );
    if (prevMode === "sketch") this.setDimmed(true);

    // Display mode (persisted, settingsStore) + isolation (transient, W3
    // viewportStore): both are applied to the handles this controller owns.
    // Separate prev-guards so a change to one never re-walks the scene for
    // the other.
    let prevDisplay = settingsStore.getState().displayMode;
    this.unsubs.push(
      settingsStore.subscribe((s) => {
        if (s.displayMode !== prevDisplay) {
          prevDisplay = s.displayMode;
          this.applyDisplayMode();
        }
      }),
    );
    let prevIsolate = viewportStore.getState().isolatedBodyIds;
    this.unsubs.push(
      viewportStore.subscribe((s) => {
        if (s.isolatedBodyIds !== prevIsolate) {
          prevIsolate = s.isolatedBodyIds;
          this.applyIsolation();
        }
      }),
    );

    // Initial sweep: bodies already in the store at attach time (open/new/recover
    // populate the projection before the viewport engine exists) never fire a
    // document-changed or visibility-flip event, so bootstrap them here. Idempotent
    // via loadSeq (a later document-changed for the same body just supersedes it).
    for (const [id, meta] of Object.entries(documentStore.getState().bodies)) {
      if (meta.visible) void this.loadBody(id, DEFAULT_LOD);
    }
  }

  private onDocumentChanged(change: DocumentChange): void {
    for (const id of change.removedBodies) this.dropBody(id);
    const bodies = documentStore.getState().bodies;
    for (const ref of change.changedBodies) {
      const visible = bodies[ref.bodyId]?.visible ?? true;
      if (visible) void this.loadBody(ref.bodyId, DEFAULT_LOD);
    }
  }

  private onVisibilityChanged(
    prev: Record<string, { visible: boolean }>,
    next: Record<string, { visible: boolean }>,
  ): void {
    for (const [id, meta] of Object.entries(next)) {
      const was = prev[id]?.visible;
      if (was === meta.visible) continue;
      const handle = this.bodyObjects.get(id);
      if (handle) {
        handle.setVisible(this.effectiveVisible(id));
        this.engine?.invalidate();
      } else if (meta.visible) {
        // Lazy-load on first show. Gated on the DOCUMENT fact, not the effective
        // one: fetching a body that isolation is currently masking costs one
        // mesh but keeps it fresh, and `loadBody` applies the mask on arrival.
        void this.loadBody(id, DEFAULT_LOD);
      }
    }
  }

  /**
   * Should this body render right now? The document's `visible` fact AND the
   * transient isolation mask — in that order of authority. Isolation can only
   * ever hide MORE than the tree eye does, so leaving isolation never resurrects
   * a body the user hid.
   */
  private effectiveVisible(bodyId: string): boolean {
    const docVisible = documentStore.getState().bodies[bodyId]?.visible ?? true;
    const isolated = viewportStore.getState().isolatedBodyIds;
    return docVisible && (isolated === null || isolated.includes(bodyId));
  }

  /**
   * The store's display mode as a render-mode descriptor. Coerced rather than
   * indexed blind so an unknown id (older persisted session) falls back instead
   * of leaving handles with an undefined mode.
   */
  private currentModeDef(): RenderModeDef {
    return RENDER_MODES[coerceRenderMode(settingsStore.getState().displayMode)];
  }

  /** Push the current display mode onto every live body handle. */
  private applyDisplayMode(): void {
    const def = this.currentModeDef();
    for (const handle of this.bodyObjects.values()) handle.applyMode(def);
    this.engine?.invalidate();
  }

  /**
   * Re-evaluate effective visibility for every body after an isolation change.
   * A body that becomes effectively visible with no scene object yet (never
   * loaded because it was hidden at attach time) is lazy-loaded here.
   */
  private applyIsolation(): void {
    for (const [id, handle] of this.bodyObjects) handle.setVisible(this.effectiveVisible(id));
    for (const id of Object.keys(documentStore.getState().bodies)) {
      if (!this.bodyObjects.has(id) && this.effectiveVisible(id)) {
        void this.loadBody(id, DEFAULT_LOD);
      }
    }
    this.engine?.invalidate();
  }

  private async loadBody(bodyId: string, lod: Lod): Promise<void> {
    if (!this.client || !this.engine || !this.materials) return;
    const token = (this.loadSeq.get(bodyId) ?? 0) + 1;
    this.loadSeq.set(bodyId, token);

    const buffer = await this.client.getBodyMesh(bodyId, lod);
    // Discard if detached or superseded by a newer fetch for this body.
    if (this.detached || this.loadSeq.get(bodyId) !== token) return;
    // Empty = the mesh isn't regenerated/cached yet (Rust get_mesh miss); the
    // later document-changed event re-triggers this fetch once it is published.
    if (buffer.byteLength === 0) return;

    const view = parseMeshPayload(buffer);
    const entry = buildBodyObjects(view, bodyId, ++this.meshRev);
    swap(bodyId, entry);

    // Rebuild the scene object (remove old, add new).
    const old = this.bodyObjects.get(bodyId);
    if (old) this.engine.bodiesRoot.remove(old.group);
    const handle = buildBodyObject(entry, this.materials);
    handle.setVisible(this.effectiveVisible(bodyId));
    handle.applyMode(this.currentModeDef());
    this.engine.bodiesRoot.add(handle.group);
    this.bodyObjects.set(bodyId, handle);

    this.engine.refreshHighlights();
    this.engine.invalidate();
    for (const cb of [...this.bodyLoadedListeners]) cb(bodyId);
  }

  /**
   * Dim (sketch mode) or restore (model mode) the body face materials — a focus
   * cue so the body isn't visually competing with the sketch on top of it. The
   * library owns the save/restore discipline (and applies the dim to material
   * sets it creates later); this only decides WHEN, and repaints.
   */
  private setDimmed(dimmed: boolean): void {
    if (!this.materials) return;
    this.materials.setDimmed(dimmed);
    this.engine?.invalidate();
  }

  private dropBody(bodyId: string): void {
    const handle = this.bodyObjects.get(bodyId);
    if (handle) {
      this.engine?.bodiesRoot.remove(handle.group);
      this.bodyObjects.delete(bodyId);
    }
    this.loadSeq.delete(bodyId);
    remove(bodyId);
    this.engine?.refreshHighlights();
    this.engine?.invalidate();
  }

  detach(): void {
    this.detached = true;
    this.bodyLoadedListeners.clear();
    for (const u of this.unsubs.splice(0)) u();
    // Clear highlights BEFORE disposing geometry so no clone references freed buffers.
    this.engine?.setHighlightState(null, []);
    for (const handle of this.bodyObjects.values()) {
      this.engine?.bodiesRoot.remove(handle.group);
    }
    this.bodyObjects.clear();
    this.loadSeq.clear();
    disposeAll(); // registry empty + leak tripwire
    this.materials?.dispose();
    this.materials = null;
    this.engine = null;
    this.client = null;
  }
}
