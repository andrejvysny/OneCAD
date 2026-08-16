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
import type { DocumentChange, Lod, Rgba } from "@/ipc/types";
import type { BodyMeta } from "@/stores/documentStore";
import { trace } from "@/debug/trace";
import { logError } from "@/debug/log";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import type { ViewportEngine } from "../engine/ViewportEngine";
import { buildBodyObject, type BodyObjectHandle } from "../engine/BodyObject";
import { BodyMaterialLibrary } from "../engine/bodyMaterials";
import { BodyMaterialPool } from "../engine/bodyMaterialPool";
import type { BodyMaterialSource, PbrMaterialParams } from "../engine/pbrParams";
import { getBodyMaterialSource, subscribeBodyMaterialSource } from "../materialSourceBridge";
import { coerceRenderMode, RENDER_MODES, type RenderModeDef } from "../engine/renderModes";
import { parseMeshPayload } from "./parseMeshPayload";
import type { BodyMeshView } from "./parseMeshPayload";
import { TopoIndex } from "./faceRangeIndex";
import type { MaterialFaceColors } from "./faceColors";
import { buildBodyObjects, disposeAll, getEntry, refreshFaceColors, remove, swap } from "./meshRegistry";
import { rebindSelectionForBody } from "./rebindPick";

/** Committed bodies use the worker's display-quality tier; drag previews choose separately. */
const DEFAULT_LOD: Lod = "fine";

/** Bounded retry for a `get_mesh` miss (mesh not regenerated/cached yet). */
const EMPTY_MESH_RETRIES = 3;
const EMPTY_MESH_RETRY_MS = 300;

function colorsEqual(
  a: Rgba | undefined,
  b: Rgba | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2] && a[3] === b[3];
}

function faceColorsEqual(
  a: Record<string, Rgba> | undefined,
  b: Record<string, Rgba> | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  const ak = Object.keys(a);
  const bk = Object.keys(b);
  if (ak.length !== bk.length) return false;
  for (const k of ak) {
    if (!colorsEqual(a[k], b[k])) return false;
  }
  return true;
}

/** Every face id of a mesh, in FACE_RANGES order. Decoded lazily by TopoIndex + memoised there. */
function faceIdsOf(topo: TopoIndex): string[] {
  const out: string[] = new Array<string>(topo.count);
  for (let f = 0; f < topo.count; f++) out[f] = topo.idOf(f);
  return out;
}

export class MeshIngest {
  private engine: ViewportEngine | null = null;
  private client: CadClient | null = null;
  private materials: BodyMaterialLibrary | null = null;
  /**
   * The pooled (assigned-material) counterpart of {@link materials}: one three
   * material per distinct document material, shared across every body wearing
   * it. Parallel to the library rather than folded into it — the library's
   * materials are TOKEN-colored and follow the theme, these are DATA-colored and
   * do not.
   */
  private materialPool: BodyMaterialPool | null = null;
  private materialSource: BodyMaterialSource | null = null;
  private materialSourceUnsub: (() => void) | null = null;
  /**
   * Per-body face-override ids this session has attributed to that body, bound
   * or not.
   *
   * The persisted override map is face-keyed and GLOBAL — which body a face
   * belongs to is tessellation evidence only this controller holds. Without this
   * memory an override whose face vanished in a regen would simply stop being
   * mentioned, and the "this override no longer binds" signal (the H5-B failure
   * mode the whole migration exists to surface) could never be reported at all.
   */
  private readonly knownFaceOverrides = new Map<string, Set<string>>();
  /** bodyId → the pool key its faces currently draw with. The sweep's live set. */
  private readonly poolKeys = new Map<string, string>();
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
    this.materialPool = new BodyMaterialPool();
    this.detached = false;

    this.unsubs.push(client.onDocumentChanged((c) => this.onDocumentChanged(c)));

    // Appearance source (`onecad.render`, via the viewport-owned bridge). Read
    // AND subscribed, because the two lifetimes do not nest: the module
    // activates once at bootstrap while this controller attaches with the
    // engine, so either can be first. The guard keeps a source injected
    // DIRECTLY before attach (tests, embedders) from being silently replaced by
    // an empty bridge — an explicit set outranks the ambient registry.
    if (!this.materialSource) this.setMaterialSource(getBodyMaterialSource());
    this.unsubs.push(subscribeBodyMaterialSource((s) => this.setMaterialSource(s)));

    // Visibility flips come through the document store (tree eye toggle) — and
    // EVERY applied projection reconciles scene ↔ store, so a body whose mesh
    // event was missed (or whose first fetch hit the pre-publish window) is
    // picked up on the next projection instead of staying invisible forever.
    // Body-color edits are metadata-only (no regen), so the mesh is rebuilt here.
    let prev = documentStore.getState().bodies;
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.bodies !== prev) {
          const old = prev;
          prev = s.bodies;
          this.onVisibilityChanged(old, s.bodies);
          this.onColorChanged(old, s.bodies);
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
    prev: Record<string, BodyMeta>,
    next: Record<string, BodyMeta>,
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

  private onColorChanged(
    prev: Record<string, BodyMeta>,
    next: Record<string, BodyMeta>,
  ): void {
    for (const [id, meta] of Object.entries(next)) {
      const was = prev[id]?.color;
      const now = meta.color;
      const wasFaces = prev[id]?.faceColors;
      const nowFaces = meta.faceColors;
      if (colorsEqual(was, now) && faceColorsEqual(wasFaces, nowFaces)) continue;
      // Rebuild only bodies that are already in the scene; reconcile handles the rest.
      if (this.bodyObjects.has(id) || (next[id]?.visible ?? true)) {
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
   *
   * The POOL's contribution is a documented no-op — its materials are colored
   * from document data, never from a token — and the re-bake it does not do is
   * covered by `refreshFaceColors()` above: that runs each entry's full
   * effective-color ladder, so an assigned material's colors are rewritten
   * identically while the token fallback underneath them follows the theme.
   */
  refreshColors(): void {
    this.materials?.refreshColors();
    this.materialPool?.refreshColors();
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

  // ── Assigned materials (PBR) ───────────────────────────────────────────────

  /**
   * Point this controller at an appearance source, or `null` to go back to the
   * shared library look for every body. Idempotent; swapping sources unsubscribes
   * the old one first.
   */
  setMaterialSource(source: BodyMaterialSource | null): void {
    if (this.materialSource === source) return;
    this.materialSourceUnsub?.();
    this.materialSourceUnsub = null;
    this.materialSource = source;
    if (source) this.materialSourceUnsub = source.subscribe(() => this.syncMaterials());
    this.syncMaterials();
  }

  /**
   * Re-resolve every live body's material, then sweep the pool. One pass over
   * the scene per source notification — a material edit changes the pool key of
   * every body wearing it, and there is no index from material back to body on
   * this side of the seam.
   */
  private syncMaterials(): void {
    if (!this.materialPool) return;
    for (const [bodyId, handle] of [...this.bodyObjects]) this.syncBodyMaterial(bodyId, handle);
    this.sweepPool();
    this.engine?.invalidate();
  }

  /**
   * Bring one body's face material in line with the source.
   *
   * A layout change (the body's FIRST face override, or its last one leaving)
   * cannot be done in place — it is a de-index/re-index of geometry that live
   * highlight clones share by reference — so it goes back through the normal
   * load path, exactly as a body-color edit does.
   */
  private syncBodyMaterial(bodyId: string, handle: BodyObjectHandle): void {
    const entry = getEntry(bodyId);
    if (!this.materialPool || !entry) return;

    const resolved = this.resolveBodyMaterial(bodyId, entry.faceIndex);
    if (!entry.setMaterialColors(resolved?.colors)) {
      // Deliberately gated on "no fetch already in flight", like reconcile():
      // the entry that fetch produces is built with the current colors, so
      // queueing a second rebuild behind it would be pure churn.
      if (!this.pending.has(bodyId)) void this.loadBody(bodyId, DEFAULT_LOD);
      return;
    }
    this.applyPooledMaterial(bodyId, handle, entry, resolved);
  }

  /**
   * Point a body's faces at its pooled material (or back at the library) and
   * record which key it is holding, so {@link sweepPool} knows what is live.
   *
   * `entry.hasVertexColors` is the geometry's own answer and the callers above
   * have already reconciled it with the colors being baked, so the `:vc` twin
   * choice can never disagree with what the attribute actually holds.
   */
  private applyPooledMaterial(
    bodyId: string,
    handle: BodyObjectHandle,
    entry: { hasVertexColors: boolean },
    resolved: { key: string; params: PbrMaterialParams } | null,
  ): void {
    const pool = this.materialPool;
    if (!pool || !resolved) {
      handle.setMaterialOverride(null);
      this.poolKeys.delete(bodyId);
      return;
    }
    const vertexColored = entry.hasVertexColors;
    const key = vertexColored ? `${resolved.key}:vc` : resolved.key;
    handle.setMaterialOverride(pool.acquire(key, resolved.params, vertexColored));
    this.poolKeys.set(bodyId, key);
  }

  /** Drop pooled materials nothing on screen is holding any more. */
  private sweepPool(): void {
    this.materialPool?.sweep(new Set(this.poolKeys.values()));
  }

  /**
   * The body's assigned-material inputs, or `null` when it has none (in which
   * case every path above falls back to today's library behaviour, unchanged).
   *
   * SIDE EFFECT, and the only place it happens: this reports which of the body's
   * face overrides the current tessellation can bind. `bound` are the ones whose
   * face id is in this mesh; `unbound` are the rest — ids the source still says
   * are overridden, which this session has previously attributed to this body,
   * and which this tessellation no longer has a face for.
   */
  private resolveBodyMaterial(
    bodyId: string,
    faces: TopoIndex,
  ): { key: string; params: PbrMaterialParams; colors: MaterialFaceColors } | null {
    const source = this.materialSource;
    if (!source) {
      this.knownFaceOverrides.delete(bodyId);
      return null;
    }
    const key = source.poolKeyForBody(bodyId);
    const params = key === null ? null : source.paramsForBody(bodyId);
    if (key === null || params === null) {
      // A per-face override on a body with no BODY material is not rendered in
      // V1 — there is no program to hang it off — so nothing is queried here,
      // and the whole-face-table id decode below is skipped with it.
      this.clearFaceOverrideReport(bodyId, source);
      return null;
    }

    const faceIds = faceIdsOf(faces);
    const remembered = this.knownFaceOverrides.get(bodyId);
    const asked =
      remembered && remembered.size > 0 ? [...new Set([...faceIds, ...remembered])] : faceIds;
    const overridden = source.faceOverrideBaseColors(bodyId, asked);

    const inMesh = new Set(faceIds);
    const bound: string[] = [];
    const unbound: string[] = [];
    const overrides = new Map<string, readonly [number, number, number]>();
    for (const [faceId, color] of Object.entries(overridden)) {
      if (inMesh.has(faceId)) {
        bound.push(faceId);
        // Reported as BOUND even where a functional face color will outrank it
        // in the bake: binding is about the face EXISTING, not about who wins
        // the modeling view's two-track precedence.
        overrides.set(faceId, color);
      } else {
        unbound.push(faceId);
      }
    }
    source.reportFaceOverrideBindings(bodyId, bound, unbound);
    if (bound.length > 0 || unbound.length > 0) {
      this.knownFaceOverrides.set(bodyId, new Set([...bound, ...unbound]));
    } else {
      this.knownFaceOverrides.delete(bodyId);
    }

    return { key, params, colors: { overrides, bodyBase: params.base_color } };
  }

  /** Retract every binding claim for a body (it lost its material, or left). */
  private clearFaceOverrideReport(bodyId: string, source: BodyMaterialSource | null): void {
    if (this.knownFaceOverrides.delete(bodyId)) {
      source?.reportFaceOverrideBindings(bodyId, [], []);
    }
  }

  private async resolveAuthoredFaceColors(
    bodyId: string,
    view: BodyMeshView,
    faceColorsMeta: Record<string, Rgba> | undefined,
  ): Promise<Map<string, Rgba> | undefined> {
    if (!faceColorsMeta || Object.keys(faceColorsMeta).length === 0) return undefined;
    const out = new Map<string, Rgba>();
    if (view.idsHaveElementIds) {
      // The mesh ids ARE persistent ElementIds, so authored colors map straight on.
      for (const [elementId, rgba] of Object.entries(faceColorsMeta)) out.set(elementId, rgba);
    } else if (this.client) {
      // Mesh ids are snapshot TopoKeys; resolve each persisted ElementId to its
      // current TopoKey. This is the fallback path for bodies whose partition has
      // not yet been stamped with ElementIds.
      for (const [elementId, rgba] of Object.entries(faceColorsMeta)) {
        const info = await this.client.elementInfo(bodyId, elementId);
        if (info?.topoKey) out.set(info.topoKey, rgba);
      }
    }
    return out.size > 0 ? out : undefined;
  }

  private async loadBody(bodyId: string, lod: Lod, attempt = 0): Promise<void> {
    if (!this.client || !this.engine || !this.materials) return;
    const token = (this.loadSeq.get(bodyId) ?? 0) + 1;
    this.loadSeq.set(bodyId, token);
    this.pending.add(bodyId);
    const bodyColor = documentStore.getState().bodies[bodyId]?.color;
    const faceColorsMeta = documentStore.getState().bodies[bodyId]?.faceColors;

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
      const authoredFaceColors = await this.resolveAuthoredFaceColors(bodyId, view, faceColorsMeta);
      // Resolved BEFORE the build, not patched on after: whether the geometry is
      // de-indexed with a color attribute at all depends on the answer, and that
      // is a construction-time decision (see meshRegistry.setMaterialColors).
      const material = this.resolveBodyMaterial(
        bodyId,
        new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars),
      );
      const entry = buildBodyObjects(
        view,
        bodyId,
        ++this.meshRev,
        bodyColor,
        authoredFaceColors,
        material?.colors,
      );
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
      this.applyPooledMaterial(bodyId, handle, entry, material);
      this.engine.bodiesRoot.add(handle.group);
      this.bodyObjects.set(bodyId, handle);
      // The mesh this replaced may have been the last holder of its pool key.
      this.sweepPool();
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
   *
   * The POOL is dimmed in lockstep, or a body with an assigned material would
   * stay at full strength while every unassigned body faded — the focus cue is
   * a property of the mode, not of who owns the material.
   */
  private setDimmed(dimmed: boolean): void {
    if (!this.materials) return;
    this.materials.setDimmed(dimmed);
    this.materialPool?.setDimmed(dimmed);
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
    // The body is gone, so its override bindings are no longer evidence of
    // anything — retract them rather than leave a stale claim in the store.
    this.clearFaceOverrideReport(bodyId, this.materialSource);
    this.poolKeys.delete(bodyId);
    this.sweepPool();
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
    // Unsubscribed, but the REPORTS are left standing: the source outlives this
    // controller (the module owns it), and a StrictMode remount re-reports the
    // same bindings a moment later. Retracting them here would flicker the
    // override count to zero on every viewport remount.
    this.materialSourceUnsub?.();
    this.materialSourceUnsub = null;
    this.materialSource = null;
    this.knownFaceOverrides.clear();
    this.poolKeys.clear();
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
    this.materialPool?.dispose();
    this.materialPool = null;
    this.engine = null;
    this.client = null;
  }
}
