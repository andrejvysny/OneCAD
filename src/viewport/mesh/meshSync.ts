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
 * SELF-HEALING (SAVE/OPEN hardening): `document-changed` is the *fast path*, not
 * a load-bearing single point of failure. Every applied projection additionally
 * runs `reconcile()` — any store-visible body with no scene object (and no fetch
 * in flight) is loaded, and any scene object whose body left the store is
 * dropped. A `get_mesh` miss (mesh not regenerated yet) also retries itself a
 * few times, so a publish that lands with no further frontend event still
 * renders. Failures are surfaced (statusHint + console), never swallowed.
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
import { trace } from "@/debug/trace";
import { logError } from "@/debug/log";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import type { ViewportEngine } from "../engine/ViewportEngine";
import { buildBodyObject, type BodyObjectHandle } from "../engine/BodyObject";
import { BodyMaterialLibrary } from "../engine/bodyMaterials";
import { coerceRenderMode, RENDER_MODES, type RenderModeDef } from "../engine/renderModes";
import { parseMeshPayload } from "./parseMeshPayload";
import { buildBodyObjects, disposeAll, getEntry, refreshFaceColors, remove, swap } from "./meshRegistry";
import { rebindSelectionForBody } from "./rebindPick";

const DEFAULT_LOD: Lod = "coarse";

/** Bounded retry for a `get_mesh` miss (mesh not regenerated/cached yet). */
const EMPTY_MESH_RETRIES = 3;
const EMPTY_MESH_RETRY_MS = 300;

export class MeshIngest {
  private engine: ViewportEngine | null = null;
  private client: CadClient | null = null;
  private materials: BodyMaterialLibrary | null = null;
  private readonly bodyObjects = new Map<string, BodyObjectHandle>();
  private readonly unsubs: Array<() => void> = [];
  private meshRev = 0;
  /** Per-body monotonic fetch token — a resolved fetch older than the latest is discarded. */
  private readonly loadSeq = new Map<string, number>();
  /** Bodies with a fetch in flight — `reconcile()` skips these (no fetch storms). */
  private readonly pending = new Set<string>();
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

    // Visibility flips come through the document store (tree eye toggle) — and
    // EVERY applied projection reconciles scene ↔ store, so a body whose mesh
    // event was missed (or whose first fetch hit the pre-publish window) is
    // picked up on the next projection instead of staying invisible forever.
    let prev = documentStore.getState().bodies;
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.bodies !== prev) {
          const old = prev;
          prev = s.bodies;
          this.onVisibilityChanged(old, s.bodies);
          this.reconcile();
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
    this.reconcile();
  }

  /**
   * Scene ↔ store reconciliation (the self-healing pass): load every
   * store-visible body with no scene object and no fetch in flight; drop every
   * scene object whose body is gone from the store. Runs at attach and after
   * every applied projection — `document-changed` stays the fast path, but a
   * missed event can no longer strand a body invisible.
   */
  private reconcile(): void {
    const bodies = documentStore.getState().bodies;
    for (const [id, meta] of Object.entries(bodies)) {
      if (meta.visible && !this.bodyObjects.has(id) && !this.pending.has(id)) {
        void this.loadBody(id, DEFAULT_LOD);
      }
    }
    for (const id of [...this.bodyObjects.keys()]) {
      if (!(id in bodies)) this.dropBody(id);
    }
    this.updateGeometryPending();
  }

  /**
   * "Rebuilding geometry…" chip (viewportStore.geometryPending): true while the
   * document is READY but at least one VISIBLE body still has no scene object —
   * the empty-viewport window after open, before the first mesh (or a retry)
   * lands. COUNT-based, not "any mesh landed": with N visible bodies, only some
   * of which have loaded, the chip must stay up until every one of them has.
   * Scoped to visible bodies only — a body the tree eye hid stays irrelevant.
   */
  private updateGeometryPending(): void {
    const { status, bodies } = documentStore.getState();
    let visible = 0;
    let loaded = 0;
    for (const [id, meta] of Object.entries(bodies)) {
      if (!meta.visible) continue;
      visible++;
      if (this.bodyObjects.has(id)) loaded++;
    }
    viewportStore.getState().setGeometryPending(status === "ready" && visible > loaded);
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

  /**
   * Theme change: re-read the palette into the COMMITTED bodies' material
   * library. The engine owns a separate library for previews and refreshes that
   * one itself; nothing can reach both, so ViewportRoot drives the pair.
   *
   * Deliberately not a store subscription of its own: the palette cache must be
   * dropped before any re-read, and independent subscribers would make that
   * ordering a race.
   *
   * A colored (imported) body needs MORE than a material re-read: its unset
   * faces have the body-fill token BAKED into a vertex attribute, so the
   * registry re-bakes those in place. Authored colors are data and stay put.
   */
  refreshColors(): void {
    this.materials?.refreshColors();
    refreshFaceColors();
    this.engine?.invalidate();
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

  private async loadBody(bodyId: string, lod: Lod, attempt = 0): Promise<void> {
    if (!this.client || !this.engine || !this.materials) return;
    const token = (this.loadSeq.get(bodyId) ?? 0) + 1;
    this.loadSeq.set(bodyId, token);
    this.pending.add(bodyId);

    try {
      const buffer = await this.client.getBodyMesh(bodyId, lod);
      // Discard if detached or superseded by a newer fetch for this body.
      if (this.detached || this.loadSeq.get(bodyId) !== token) return;
      // Empty = the mesh isn't regenerated/cached yet (Rust get_mesh miss). The
      // document-changed / reconcile paths re-trigger this fetch once published;
      // the bounded self-retry covers a publish that lands with no further event.
      if (buffer.byteLength === 0) {
        trace("mesh", `getBodyMesh miss body=${bodyId} attempt=${attempt} (not published yet)`);
        if (attempt < EMPTY_MESH_RETRIES) {
          setTimeout(() => {
            if (
              !this.detached &&
              this.loadSeq.get(bodyId) === token &&
              !this.bodyObjects.has(bodyId)
            ) {
              void this.loadBody(bodyId, lod, attempt + 1);
            }
          }, EMPTY_MESH_RETRY_MS);
        }
        return;
      }

      const view = parseMeshPayload(buffer);
      const entry = buildBodyObjects(view, bodyId, ++this.meshRev);
      const prevEntry = getEntry(bodyId); // read BEFORE the swap — the rebind's evidence
      swap(bodyId, entry);
      // A regen renumbers TopoKeys, so a selection made before it names nothing
      // after it and its highlight would silently vanish. Re-point the refs at the
      // geometry they still mean BEFORE the highlight rebuild below reads them.
      rebindSelectionForBody(bodyId, prevEntry, entry);

      // Rebuild the scene object (remove old, add new).
      const old = this.bodyObjects.get(bodyId);
      if (old) this.engine.bodiesRoot.remove(old.group);
      const handle = buildBodyObject(entry, this.materials);
      handle.setVisible(this.effectiveVisible(bodyId));
      handle.applyMode(this.currentModeDef());
      this.engine.bodiesRoot.add(handle.group);
      this.bodyObjects.set(bodyId, handle);
      this.updateGeometryPending();

      this.engine.refreshHighlights();
      this.engine.invalidate();
      for (const cb of [...this.bodyLoadedListeners]) cb(bodyId);
    } catch (e) {
      if (this.detached) return;
      // A fetch/parse/build failure must never be silent: without this the body
      // simply never appears and nothing anywhere says why (the SAVE/OPEN bug
      // class). Keep serving other bodies.
      const reason = e instanceof Error ? e.message : String(e);
      logError("mesh", `body mesh load FAILED body=${bodyId}`, { bodyId, error: e });
      viewportStore.getState().setStatusHint(`Body failed to load — ${reason}`, {
        severity: "error",
      });
    } finally {
      if (this.loadSeq.get(bodyId) === token) this.pending.delete(bodyId);
    }
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
    this.pending.delete(bodyId);
    remove(bodyId);
    this.updateGeometryPending();
    this.engine?.refreshHighlights();
    this.engine?.invalidate();
  }

  detach(): void {
    this.detached = true;
    // Force the chip off immediately — the store it targets outlives this
    // instance, and a stale `true` would strand it visible after teardown.
    viewportStore.getState().setGeometryPending(false);
    this.bodyLoadedListeners.clear();
    for (const u of this.unsubs.splice(0)) u();
    // Clear highlights BEFORE disposing geometry so no clone references freed buffers.
    this.engine?.setHighlightState(null, []);
    for (const handle of this.bodyObjects.values()) {
      this.engine?.bodiesRoot.remove(handle.group);
    }
    this.bodyObjects.clear();
    this.loadSeq.clear();
    this.pending.clear();
    disposeAll(); // registry empty + leak tripwire
    this.materials?.dispose();
    this.materials = null;
    this.engine = null;
    this.client = null;
  }
}
