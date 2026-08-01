/*
 * DatumSync — the app-level glue that keeps the datum-plane layer in step with
 * the stores (DATUM W1). Same shape as SketchStaticSync, minus the async fetch:
 * a datum's frame is already IN the projection (`DatumMeta.plane`, resolved by
 * the backend), so there is nothing to go and get.
 *
 *   - documentStore.datums  → diff add / remove / frame change → engine.syncDatums
 *   - selectionStore        → mirror datum selection + hover into the layer tint
 *
 * The engine owns the layer's disposal (engine.dispose); detach just unsubscribes.
 */
import { documentStore, type DatumMeta } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import type { ViewportEngine } from "./engine/ViewportEngine";
import type { DatumVisual } from "./engine/DatumLayer";

/** Projection datums → the layer's minimal visual shape (stable ordering). */
export function datumVisuals(datums: Record<string, DatumMeta>): DatumVisual[] {
  return Object.values(datums).map((d) => ({
    id: d.id,
    name: d.name,
    plane: d.plane,
    resolvedValid: d.resolvedValid,
  }));
}

export class DatumSync {
  private engine: ViewportEngine | null = null;
  private readonly unsubs: Array<() => void> = [];

  attach(engine: ViewportEngine): void {
    this.engine = engine;

    // documentStore.datums (identity change ⇒ re-sync; the LAYER diffs by id +
    // frame, so an unrelated projection that replaces the record object is cheap).
    let prevDatums = documentStore.getState().datums;
    engine.syncDatums(datumVisuals(prevDatums)); // initial sweep
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.datums === prevDatums) return;
        prevDatums = s.datums;
        this.engine?.syncDatums(datumVisuals(s.datums));
      }),
    );

    // Selection + hover → layer tint (single source of truth: the selection
    // store, fed by both tree clicks and viewport picks).
    const applySelection = (): void => {
      const sel = selectionStore.getState();
      this.engine?.setDatumSelected(
        sel.selected.filter((r) => r.kind === "datum").map((r) => r.id),
      );
      this.engine?.setDatumHover(sel.hover?.kind === "datum" ? sel.hover.id : null);
    };
    applySelection();
    this.unsubs.push(selectionStore.subscribe(applySelection));
  }

  detach(): void {
    for (const u of this.unsubs.splice(0)) u();
    this.engine = null;
  }
}
