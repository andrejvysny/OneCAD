/*
 * SketchStaticSync — the app-level glue that keeps the always-visible sketch layer
 * (SketchStaticLayer) in step with the stores, mirroring the MeshIngest pattern.
 *
 * Flow:
 *   - documentStore.sketches → diff added / removed / visibility / geometry token
 *     → for each visible sketch fetch `getSketch(id)` + read-only regions.
 *   - toolStore.mode + viewportStore.activeSketchId → hide the ONE sketch being
 *     edited (its live SketchObject owns it) and REFETCH it on exit (geometry likely
 *     changed while editing).
 *   - selectionStore → mirror sketch selection + hover into the layer's tint.
 *   - documentStore.revision → retry the sketches whose region fetch FAILED.
 *
 * Per-sketch fetch is latest-wins (a superseded fetch is dropped, meshSync loadSeq
 * pattern) and detach-guarded (StrictMode-safe). The engine owns the layer's
 * disposal (engine.dispose); detach just unsubscribes.
 *
 * Region-fetch retry: a rejected `getSketchRegions` degrades to curves-only, and the
 * sketch is then marked loaded — so without a retry a fetch that lost a race with a
 * not-yet-ready worker left that sketch permanently unfilled (nothing else refetches
 * it: `syncSketches` only reloads on a geometryToken change). `revision` is the retry
 * signal: it is the backend-authoritative publish stamp, hydrated from the
 * `projection-updated` that EVERY regen completion emits, and bumped by the mock
 * lane's commit path — one signal, both lanes. Worker-ready is deliberately NOT the
 * signal: it fires before the replay that produces the regions.
 */
import type { CadClient } from "@/ipc/client";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore } from "@/stores/selectionStore";
import type { ViewportEngine } from "./engine/ViewportEngine";
import type { SketchStaticLayer, SketchStaticTarget } from "./engine/SketchStaticLayer";

function staticHitForSelection(
  ref: ReturnType<typeof selectionStore.getState>["selected"][number],
): SketchStaticTarget | null {
  if (ref.kind === "sketch") return { kind: "sketch", sketchId: ref.id };
  if (ref.kind === "sketchRegion") {
    return {
      kind: "sketchRegion",
      sketchId: ref.sketchId,
      regionId: ref.regionId,
    };
  }
  return null;
}

export class SketchStaticSync {
  private client: CadClient | null = null;
  private layer: SketchStaticLayer | null = null;
  private detached = false;
  private readonly unsubs: Array<() => void> = [];
  /** Per-sketch latest global fetch token (latest-wins, safe across remove/re-add). */
  private readonly fetchSeq = new Map<string, number>();
  private fetchCounter = 0;
  /** Sketch ids currently built into the layer. */
  private readonly loaded = new Set<string>();
  /** Sketch ids whose region fetch rejected, awaiting the next published revision. */
  private readonly regionRetry = new Set<string>();
  /**
   * Sketch ids whose PICKABLE geometry is currently missing or being replaced.
   *
   * A token-driven reload tears the fills out of the layer and only puts them back
   * two awaited round trips later; in that window `sketchStaticHitTest` answers
   * `null`, which is indistinguishable from "the user clicked empty space". Polling
   * a readiness flag before the click cannot close that — the window can open
   * between the poll and the pick — so consumers ask for the SETTLED state at pick
   * time and defer instead (see `ViewportEngine.sketchStaticPickState`).
   */
  private readonly refilling = new Set<string>();
  /** One-shot callbacks waiting for `refilling` to drain. */
  private readonly settledWaiters = new Set<() => void>();

  attach(engine: ViewportEngine, client: CadClient): void {
    this.client = client;
    this.layer = engine.getSketchStaticLayer();
    this.detached = false;
    // The engine owns picking but not the fetch lifecycle, so it asks US whether the
    // pickable set is currently trustworthy.
    engine.setSketchStaticReadiness({
      isSettled: () => this.isSettled(),
      onSettled: (cb) => this.onSettled(cb),
    });
    this.unsubs.push(() => engine.setSketchStaticReadiness(null));

    // documentStore sketches (identity change ⇒ diff added / removed / visibility),
    // plus revision (⇒ retry the sketches whose regions never arrived).
    let prevSketches = documentStore.getState().sketches;
    let prevRevision = documentStore.getState().revision;
    this.syncSketches({}, prevSketches); // initial sweep
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.sketches !== prevSketches) {
          this.syncSketches(prevSketches, s.sketches);
          prevSketches = s.sketches;
        }
        if (s.revision !== prevRevision) {
          prevRevision = s.revision;
          this.retryFailedRegions();
        }
      }),
    );

    // Editing hide + refetch-on-exit. Track the sketch that WAS active so a
    // sketch→model transition can refetch it (activeSketchId is cleared to null on
    // exit, so we capture it before the viewportStore subscription runs).
    let prevMode = toolStore.getState().mode;
    let prevActive = viewportStore.getState().activeSketchId;
    this.applyEditing(prevMode, prevActive);
    this.unsubs.push(
      toolStore.subscribe((s) => {
        if (s.mode === prevMode) return;
        const wasSketch = prevMode === "sketch";
        prevMode = s.mode;
        const active = viewportStore.getState().activeSketchId;
        this.applyEditing(s.mode, active);
        if (wasSketch && s.mode === "model" && prevActive) this.refetch(prevActive);
      }),
    );
    this.unsubs.push(
      viewportStore.subscribe((s) => {
        if (s.activeSketchId === prevActive) return;
        prevActive = s.activeSketchId;
        this.applyEditing(toolStore.getState().mode, s.activeSketchId);
      }),
    );

    // Selection + hover → layer tint (single source of truth: the selection store,
    // fed by both tree clicks and viewport picks).
    const applySelection = (): void => {
      const sel = selectionStore.getState();
      this.layer?.setSelected(
        sel.selected
          .map(staticHitForSelection)
          .filter((hit): hit is SketchStaticTarget => hit !== null),
      );
      this.layer?.setHover(sel.hover ? staticHitForSelection(sel.hover) : null);
    };
    applySelection();
    this.unsubs.push(selectionStore.subscribe(applySelection));
  }

  private syncSketches(
    prev: Record<string, SketchMeta>,
    next: Record<string, SketchMeta>,
  ): void {
    for (const id of Object.keys(prev)) {
      if (!next[id]) {
        this.layer?.removeSketch(id);
        this.loaded.delete(id);
        this.fetchSeq.delete(id);
        this.regionRetry.delete(id);
        // A DELETED sketch is authoritative: its regions really are gone, so
        // reconciling here is correct (unlike a failed fetch, which is not).
        this.reconcileRegionRefs(id, new Set());
        this.clearRefilling(id);
      }
    }
    for (const [id, meta] of Object.entries(next)) {
      const before = prev[id];
      if (!before) {
        if (meta.visible) void this.loadSketch(id);
      } else if (before.geometryToken !== meta.geometryToken) {
        // Never leave old fills pickable while the replacement is in flight. The
        // sketch is UNSETTLED from here until `loadSketch` puts the new fills in,
        // so a pick landing in the gap defers rather than reading "empty space".
        this.layer?.removeSketch(id);
        this.loaded.delete(id);
        this.fetchSeq.delete(id);
        if (meta.visible && !this.isEditing(id)) {
          this.markRefilling(id);
          void this.loadSketch(id);
        }
      } else if (before.visible !== meta.visible) {
        if (this.loaded.has(id)) this.layer?.setVisible(id, meta.visible);
        else if (meta.visible) {
          this.markRefilling(id);
          void this.loadSketch(id); // lazy-load on first show
        }
      }
    }
  }

  private async loadSketch(id: string): Promise<void> {
    const client = this.client;
    if (!client || this.detached) return;
    const token = ++this.fetchCounter;
    this.fetchSeq.set(id, token);
    // An attempt is now in flight, so this id is no longer PENDING a retry — it is
    // re-armed below only if the region fetch rejects again. Clearing it here is what
    // keeps a retry that races a token-driven reload from double-fetching.
    this.regionRetry.delete(id);
    // Captured so a rejection can tell whether a publish landed DURING this fetch —
    // that publish already ran retryFailedRegions against an empty set, so waiting
    // for the next one could strand the sketch curves-only forever.
    const revisionAtStart = documentStore.getState().revision;

    let session;
    try {
      session = await client.getSketch(id);
    } catch {
      // Unknown / not readable — skip (curves appear once it is readable). The
      // refill will not land, so stop holding picks for it.
      this.clearRefilling(id);
      return;
    }
    if (this.detached || this.fetchSeq.get(id) !== token) {
      // Superseded: the NEWER load owns the refill flag, so do not clear it here or
      // a pick would be released against a layer that is still empty.
      if (this.detached) this.clearRefilling(id);
      return;
    }

    // Fill is best-effort: a reject or zero regions degrades to curves-only. A reject
    // can simply mean the fetch lost a race with a still-connecting worker, so the id
    // is queued for the next published revision rather than left unfilled forever.
    let finish: Awaited<ReturnType<CadClient["getSketchRegions"]>>;
    // A REJECTED region fetch is not evidence that regions disappeared, so it must
    // not be reconciled against the live selection — doing so dropped every
    // `sketchRegion` ref for this sketch on a transient failure.
    let regionsAreAuthoritative = true;
    try {
      finish = await client.getSketchRegions(id);
    } catch {
      finish = { regionIdentityVersion: 3, regions: [] };
      regionsAreAuthoritative = false;
      if (this.fetchSeq.get(id) === token) {
        this.regionRetry.add(id);
        // A publish that landed mid-fetch has already been consumed by an empty
        // retry sweep — grant its attempt now instead of waiting for a publish
        // that may never come. Bounded: the retry captures the NEW revision, so a
        // second rejection with no further publish just re-queues.
        if (documentStore.getState().revision !== revisionAtStart) {
          queueMicrotask(() => this.retryFailedRegions());
        }
      }
    }
    if (this.detached || this.fetchSeq.get(id) !== token) {
      if (this.detached) this.clearRefilling(id);
      return;
    }

    if (regionsAreAuthoritative) {
      this.reconcileRegionRefs(
        id,
        new Set(finish.regions.map((region) => region.regionId)),
      );
    }
    this.loaded.add(id);
    this.layer?.setSketch(id, { plane: session.plane, entities: session.entities, regions: finish.regions });
    // Re-assert the current tree visibility (the editing-hide override is applied by
    // the layer against its tracked editing id).
    this.layer?.setVisible(id, documentStore.getState().sketches[id]?.visible ?? true);
    // The fills are back and pickable — release any pick that deferred on them.
    this.clearRefilling(id);
  }

  /** Refetch a sketch's geometry (e.g. after exiting its edit session). */
  private refetch(id: string): void {
    if (documentStore.getState().sketches[id]?.visible) void this.loadSketch(id);
  }

  /**
   * Re-attempt every sketch whose regions never arrived. Driven by a NEW published
   * revision only, so a permanently failing fetch costs one attempt per publish and
   * can never spin.
   */
  private retryFailedRegions(): void {
    for (const id of [...this.regionRetry]) {
      if (this.isEditing(id)) continue; // the live SketchObject owns it
      this.refetch(id);
    }
  }

  private isEditing(id: string): boolean {
    return (
      toolStore.getState().mode === "sketch" &&
      viewportStore.getState().activeSketchId === id
    );
  }

  /** Drop only exact region refs that the refreshed sketch no longer publishes. */
  private reconcileRegionRefs(id: string, available: ReadonlySet<string>): void {
    const selection = selectionStore.getState();
    const next = selection.selected.filter(
      (ref) =>
        ref.kind !== "sketchRegion" ||
        ref.sketchId !== id ||
        available.has(ref.regionId),
    );
    if (next.length !== selection.selected.length) selection.set(next);
    const hover = selectionStore.getState().hover;
    if (
      hover?.kind === "sketchRegion" &&
      hover.sketchId === id &&
      !available.has(hover.regionId)
    ) {
      selectionStore.getState().setHover(null);
    }
  }

  private applyEditing(mode: string, activeId: string | null): void {
    this.layer?.setEditingSketch(mode === "sketch" ? activeId : null);
  }

  /** True when every sketch's pickable geometry is in the layer (no refill in flight). */
  isSettled(): boolean {
    return this.refilling.size === 0;
  }

  /**
   * Run `cb` once the layer is settled — immediately if it already is. Returns a
   * canceller, because a pick that is superseded (another pick, a mode change)
   * must not fire late.
   */
  onSettled(cb: () => void): () => void {
    if (this.isSettled()) {
      cb();
      return () => {};
    }
    this.settledWaiters.add(cb);
    return () => this.settledWaiters.delete(cb);
  }

  private markRefilling(id: string): void {
    this.refilling.add(id);
  }

  private clearRefilling(id: string): void {
    if (!this.refilling.delete(id)) return;
    if (!this.isSettled()) return;
    for (const cb of [...this.settledWaiters]) {
      this.settledWaiters.delete(cb);
      cb();
    }
  }

  detach(): void {
    this.detached = true;
    for (const u of this.unsubs.splice(0)) u();
    this.fetchSeq.clear();
    this.fetchCounter = 0;
    this.loaded.clear();
    this.regionRetry.clear();
    // Release every deferred pick: nothing will refill after a detach, and a waiter
    // that never fires is a click silently swallowed.
    this.refilling.clear();
    for (const cb of [...this.settledWaiters]) {
      this.settledWaiters.delete(cb);
      cb();
    }
    // The engine owns the layer's disposal (engine.dispose); nothing to free here.
    this.layer = null;
    this.client = null;
  }
}
