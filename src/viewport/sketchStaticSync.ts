/*
 * SketchStaticSync — the app-level glue that keeps the always-visible sketch layer
 * (SketchStaticLayer) in step with the stores, mirroring the MeshIngest pattern.
 *
 * Flow:
 *   - documentStore.sketches → diff added / removed / visibility → for each visible
 *     sketch fetch `getSketch(id)` (geometry) + `finishSketch(id)` (fill regions,
 *     graceful-degraded to none) → layer.setSketch.
 *   - toolStore.mode + viewportStore.activeSketchId → hide the ONE sketch being
 *     edited (its live SketchObject owns it) and REFETCH it on exit (geometry likely
 *     changed while editing).
 *   - selectionStore → mirror sketch selection + hover into the layer's tint.
 *
 * Per-sketch fetch is latest-wins (a superseded fetch is dropped, meshSync loadSeq
 * pattern) and detach-guarded (StrictMode-safe). The engine owns the layer's
 * disposal (engine.dispose); detach just unsubscribes.
 */
import type { CadClient } from "@/ipc/client";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore } from "@/stores/selectionStore";
import type { ViewportEngine } from "./engine/ViewportEngine";
import type { SketchStaticLayer } from "./engine/SketchStaticLayer";

export class SketchStaticSync {
  private client: CadClient | null = null;
  private layer: SketchStaticLayer | null = null;
  private detached = false;
  private readonly unsubs: Array<() => void> = [];
  /** Per-sketch monotonic fetch token (latest-wins). */
  private readonly fetchSeq = new Map<string, number>();
  /** Sketch ids currently built into the layer. */
  private readonly loaded = new Set<string>();

  attach(engine: ViewportEngine, client: CadClient): void {
    this.client = client;
    this.layer = engine.getSketchStaticLayer();
    this.detached = false;

    // documentStore sketches (identity change ⇒ diff added / removed / visibility).
    let prevSketches = documentStore.getState().sketches;
    this.syncSketches({}, prevSketches); // initial sweep
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.sketches !== prevSketches) {
          this.syncSketches(prevSketches, s.sketches);
          prevSketches = s.sketches;
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
      this.layer?.setSelected(sel.selected.filter((r) => r.kind === "sketch").map((r) => r.id));
      this.layer?.setHover(sel.hover?.kind === "sketch" ? sel.hover.id : null);
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
      }
    }
    for (const [id, meta] of Object.entries(next)) {
      const before = prev[id];
      if (!before) {
        if (meta.visible) void this.loadSketch(id);
      } else if (before.visible !== meta.visible) {
        if (this.loaded.has(id)) this.layer?.setVisible(id, meta.visible);
        else if (meta.visible) void this.loadSketch(id); // lazy-load on first show
      }
    }
  }

  private async loadSketch(id: string): Promise<void> {
    const client = this.client;
    if (!client || this.detached) return;
    const token = (this.fetchSeq.get(id) ?? 0) + 1;
    this.fetchSeq.set(id, token);

    let session;
    try {
      session = await client.getSketch(id);
    } catch {
      return; // unknown / not readable — skip (curves appear once it is readable)
    }
    if (this.detached || this.fetchSeq.get(id) !== token) return;

    // Fill is best-effort: a reject or zero regions degrades to curves-only.
    const finish = await client.finishSketch(id).catch(() => ({ regions: [] }));
    if (this.detached || this.fetchSeq.get(id) !== token) return;

    this.loaded.add(id);
    this.layer?.setSketch(id, { plane: session.plane, entities: session.entities, regions: finish.regions });
    // Re-assert the current tree visibility (the editing-hide override is applied by
    // the layer against its tracked editing id).
    this.layer?.setVisible(id, documentStore.getState().sketches[id]?.visible ?? true);
  }

  /** Refetch a sketch's geometry (e.g. after exiting its edit session). */
  private refetch(id: string): void {
    if (documentStore.getState().sketches[id]) void this.loadSketch(id);
  }

  private applyEditing(mode: string, activeId: string | null): void {
    this.layer?.setEditingSketch(mode === "sketch" ? activeId : null);
  }

  detach(): void {
    this.detached = true;
    for (const u of this.unsubs.splice(0)) u();
    this.fetchSeq.clear();
    this.loaded.clear();
    // The engine owns the layer's disposal (engine.dispose); nothing to free here.
    this.layer = null;
    this.client = null;
  }
}
