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
import type * as THREE from "three";
import type { CadClient } from "@/ipc/client";
import type { DocumentChange, Lod, Rgba } from "@/ipc/types";
import type { BodyMeta } from "@/stores/documentStore";
import { trace } from "@/debug/trace";
import { logDebug, logError } from "@/debug/log";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import type { ViewportEngine } from "../engine/ViewportEngine";
import { buildBodyObject, type BodyObjectHandle } from "../engine/BodyObject";
import { BodyMaterialLibrary } from "../engine/bodyMaterials";
import { coerceRenderMode, RENDER_MODES, type RenderModeDef } from "../engine/renderModes";
import { parseMeshPayload } from "./parseMeshPayload";
import type { BodyMeshView } from "./parseMeshPayload";
import { buildBodyObjects, disposeAll, getEntry, refreshFaceColors, remove, setCurrentMeshPublication, swap, type MeshProvenance } from "./meshRegistry";
import { dropSelectionForBody, reconcileSelectionForBody } from "./rebindPick";

/** Committed bodies use the worker's display-quality tier; drag previews choose separately. */
const DEFAULT_LOD: Lod = "fine";

/**
 * Why a body's mesh is being fetched — the fact that decides whether the
 * selection has a regen to survive (D-5).
 *
 * `"regen"` is a `document-changed` publish: the topology may have been
 * renumbered, elements consumed, new ones created. `"cosmetic"` is every other
 * reload — a colour edit, a visibility flip, the self-healing `reconcile()` pass
 * — which re-publishes the SAME topology under the same snapshot. Reconciling
 * those would put every selected face through a backend round-trip (and drop
 * every unpromoted one) because the user changed a colour.
 */
type LoadReason = "regen" | "cosmetic";

interface RenderCorrelation {
  expectationId: number;
  documentId: string;
  revision: number;
  snapshotId: number;
}

/** Bounded retry for a `get_mesh` miss (mesh not regenerated/cached yet). */
const EMPTY_MESH_RETRIES = 3;
const EMPTY_MESH_RETRY_MS = 300;

/** Strict parser for normative `<bodyId>:<lod>:<generation>` mesh cache keys. */
export function meshGeneration(meshKey: string): number | null {
  const match = /^(.*):(coarse|medium|fine):(0|[1-9]\d*)$/.exec(meshKey);
  if (!match) return null;
  const generation = Number(match[3]);
  return Number.isSafeInteger(generation) ? generation : null;
}

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
  /** Token of the request currently owning the network slot for each body. */
  private readonly activeToken = new Map<string, number>();
  /** One latest-wins reload retained behind each in-flight body request. */
  private readonly queued = new Map<string, LoadReason>();
  /** Latest native publication revision expected to reach an actual rendered frame. */
  private readonly renderCorrelation = new Map<string, RenderCorrelation>();
  private detached = false;
  /** Latest successful live publication. Absent during bootstrap/cached-open. */
  private currentPublication: MeshProvenance | null = null;
  /** Fires after a body's mesh finishes loading into the scene (F-WP7 commit reconcile). */
  private readonly bodyLoadedListeners = new Set<(bodyId: string) => void>();

  private traceRendered(bodyId: string, token: number, startedAt: number): void {
    const engine = this.engine;
    if (!engine) return;
    const correlation = this.renderCorrelation.get(bodyId);
    if (!correlation && !import.meta.env.DEV) return;
    const revision = correlation?.revision ?? 0;
    let unsubscribe = () => {};
    const timeout = setTimeout(() => {
      unsubscribe();
      trace("mesh", `render completion timeout revision=${revision} body=${bodyId} token=${token} timeoutMs=2000`);
    }, 2000);
    unsubscribe = engine.onAfterRender(() => {
      unsubscribe();
      clearTimeout(timeout);
      if (
        this.detached ||
        this.loadSeq.get(bodyId) !== token ||
        !this.bodyObjects.has(bodyId)
      ) return;
      trace("mesh", `render completed revision=${revision} body=${bodyId} token=${token} elapsedMs=${(performance.now() - startedAt).toFixed(1)}`);
      if (correlation) {
        void this.client?.meshRenderCompleted({ ...correlation, bodyId }).catch((error: unknown) => {
          logDebug("mesh", "render acknowledgment unavailable", { bodyId, error });
        });
      }
    });
  }

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
    this.adoptPublication(client.getCurrentMeshPublication?.() ?? null);

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
        // Carry the CURRENT publication's generation, exactly as the
        // `document-changed` path does: a mesh installed without one records no
        // provenance, and `promote.ts`'s `installedProofIsCurrent` then refuses
        // every pick on it as stale. The bootstrap sweep (open/new/recover fill
        // the projection before the viewport exists) is the ordinary way a body
        // first reaches the scene, so it must be as pickable as a regen's.
        void this.loadBody(id, DEFAULT_LOD, "cosmetic", 0, this.currentPublication?.generation);
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
    trace("mesh", `publication delivered revision=${change.revision} changed=${change.changedBodies.length} removed=${change.removedBodies.length}`);
    const document = documentStore.getState();
    if (
      change.documentId === undefined ||
      change.runtimeSession === undefined ||
      change.documentId !== document.documentId ||
      change.runtimeSession !== document.runtimeSession
    ) return;
    this.adoptPublication(change);
    for (const id of change.removedBodies) this.dropBody(id);
    const bodies = documentStore.getState().bodies;
    for (const ref of change.changedBodies) {
      if (
        change.renderExpectationId !== undefined &&
        change.documentId !== undefined &&
        change.snapshotId !== undefined
      ) {
        this.renderCorrelation.set(ref.bodyId, {
          expectationId: change.renderExpectationId,
          documentId: change.documentId,
          revision: change.revision,
          snapshotId: change.snapshotId,
        });
      }
      const visible = bodies[ref.bodyId]?.visible ?? true;
      if (visible) {
        void this.loadBody(
          ref.bodyId,
          DEFAULT_LOD,
          "regen",
          0,
          this.currentPublication?.generation,
        );
      }
    }
  }

  private adoptPublication(change: DocumentChange | null): void {
    const liveDocumentId = documentStore.getState().documentId;
    const generations = change?.changedBodies.map((ref) => meshGeneration(ref.meshKey)) ?? [];
    const generation = generations[0];
    this.currentPublication =
      change?.documentId !== undefined &&
      change.documentId === liveDocumentId &&
      change.runtimeSession !== undefined &&
      change.runtimeSession === documentStore.getState().runtimeSession &&
      change.snapshotId !== undefined &&
      Number.isSafeInteger(change.snapshotId) &&
      change.snapshotId > 0 &&
      generation !== undefined &&
      generation !== null &&
      generations.every((candidate) => candidate === generation)
        ? {
            documentId: change.documentId,
            runtimeSession: change.runtimeSession,
            snapshotId: change.snapshotId,
            generation,
          }
        : null;
    setCurrentMeshPublication(this.currentPublication);
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
        void this.loadBody(id, DEFAULT_LOD, "cosmetic");
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
        void this.loadBody(id, DEFAULT_LOD, "cosmetic");
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
   * Section view: clip (or unclip, with `null`) every COMMITTED body.
   *
   * The engine owns the plane and its own preview library but cannot reach this
   * one, so `ViewportRoot` drives both — the same split the theme refresh has.
   */
  setClippingPlanes(planes: THREE.Plane[] | null): void {
    this.materials?.setClippingPlanes(planes);
    this.engine?.invalidate();
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
        void this.loadBody(id, DEFAULT_LOD, "cosmetic");
      }
    }
    this.engine?.invalidate();
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

  private async loadBody(
    bodyId: string,
    lod: Lod,
    reason: LoadReason,
    attempt = 0,
    requestedGeneration?: number,
  ): Promise<void> {
    if (!this.client || !this.engine || !this.materials) return;
    if (this.pending.has(bodyId) && attempt === 0) {
      this.loadSeq.set(bodyId, (this.loadSeq.get(bodyId) ?? 0) + 1);
      this.queued.set(bodyId, reason === "regen" ? "regen" : (this.queued.get(bodyId) ?? reason));
      trace("mesh", `mesh request coalesced body=${bodyId} reason=${reason}`);
      return;
    }
    const liveDocumentId = documentStore.getState().documentId;
    const currentPublication = this.currentPublication;
    const effectiveGeneration = requestedGeneration ?? (
      currentPublication !== null && currentPublication.documentId === liveDocumentId
        ? currentPublication.generation
        : undefined
    );
    const token = (this.loadSeq.get(bodyId) ?? 0) + 1;
    const startedAt = performance.now();
    this.loadSeq.set(bodyId, token);
    this.activeToken.set(bodyId, token);
    trace("mesh", `mesh request start body=${bodyId} token=${token} reason=${reason}`);
    this.pending.add(bodyId);
    const bodyColor = documentStore.getState().bodies[bodyId]?.color;
    const faceColorsMeta = documentStore.getState().bodies[bodyId]?.faceColors;

    try {
      const publication = this.currentPublication;
      const buffer = effectiveGeneration === undefined
        ? await this.client.getBodyMesh(bodyId, lod)
        : await this.client.getBodyMesh(
            bodyId,
            lod,
            effectiveGeneration,
            publication?.runtimeSession,
          );
      // Discard if detached or superseded by a newer fetch for this body.
      if (this.detached || this.loadSeq.get(bodyId) !== token) return;
      if (
        effectiveGeneration !== undefined &&
        (!publication || this.currentPublication !== publication || publication.generation !== effectiveGeneration)
      ) return;
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
              void this.loadBody(bodyId, lod, reason, attempt + 1, effectiveGeneration);
            }
          }, EMPTY_MESH_RETRY_MS);
        }
        return;
      }

      const view = parseMeshPayload(buffer);
      const authoredFaceColors = await this.resolveAuthoredFaceColors(bodyId, view, faceColorsMeta);
      // `resolveAuthoredFaceColors` can await one worker round-trip PER authored
      // colour, which is long enough for a newer publish to overtake this one.
      // Re-check before the swap, or an older mesh installs over a newer one.
      if (this.detached || this.loadSeq.get(bodyId) !== token) return;
      if (
        effectiveGeneration !== undefined &&
        (!publication ||
          this.currentPublication !== publication ||
          publication.documentId !== documentStore.getState().documentId ||
          publication.runtimeSession !== documentStore.getState().runtimeSession ||
          publication.generation !== effectiveGeneration)
      ) return;
      const entry = buildBodyObjects(
        view,
        bodyId,
        ++this.meshRev,
        bodyColor,
        authoredFaceColors,
        effectiveGeneration === undefined ? undefined : publication ?? undefined,
      );
      const prev = getEntry(bodyId);
      swap(bodyId, entry);
      // A regen renumbers TopoKeys and consumes elements outright, so a selection
      // made before it may name nothing after it. Decide each ref's fate against
      // the NEW mesh (and, for a promoted one, against the backend) BEFORE the
      // highlight rebuild below reads them — never by searching for a lookalike.
      // A cosmetic reload re-publishes the SAME topology: nothing to survive.
      if (reason === "regen") reconcileSelectionForBody(bodyId, prev, entry, this.client);

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
      trace("mesh", `mesh installed body=${bodyId} token=${token} elapsedMs=${(performance.now() - startedAt).toFixed(1)}`);
      this.traceRendered(bodyId, token, startedAt);
      for (const cb of [...this.bodyLoadedListeners]) cb(bodyId);
    } catch (e) {
      if (this.detached || this.loadSeq.get(bodyId) !== token) return;
      // A fetch/parse/build failure must never be silent: without this the body
      // simply never appears and nothing anywhere says why (the SAVE/OPEN bug
      // class). Keep serving other bodies.
      const reason = e instanceof Error ? e.message : String(e);
      logError("mesh", `body mesh load FAILED body=${bodyId}`, { bodyId, error: e });
      viewportStore.getState().setStatusHint(`Body failed to load — ${reason}`, {
        severity: "error",
      });
    } finally {
      if (this.activeToken.get(bodyId) === token) {
        this.activeToken.delete(bodyId);
        this.pending.delete(bodyId);
        const queued = this.queued.get(bodyId);
        this.queued.delete(bodyId);
        if (queued && !this.detached) void this.loadBody(bodyId, lod, queued);
      }
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
    this.loadSeq.set(bodyId, (this.loadSeq.get(bodyId) ?? 0) + 1);
    this.renderCorrelation.delete(bodyId);
    this.queued.delete(bodyId);
    remove(bodyId);
    // The geometry is gone for good, so a face/edge ref naming it can never draw
    // again — and must never author an op either (D-5, drawable-or-gone).
    dropSelectionForBody(bodyId);
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
    this.activeToken.clear();
    this.queued.clear();
    this.renderCorrelation.clear();
    this.currentPublication = null;
    setCurrentMeshPublication(null);
    disposeAll(); // registry empty + leak tripwire
    this.materials?.dispose();
    this.materials = null;
    this.engine = null;
    this.client = null;
  }
}
