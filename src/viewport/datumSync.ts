/*
 * DatumSync — the app-level glue that keeps the datum-plane layer in step with
 * the stores (DATUM W1). Same shape as SketchStaticSync, minus the async fetch:
 * a datum's frame is already IN the projection (`DatumMeta.plane`, resolved by
 * the backend), so there is nothing to go and get.
 *
 *   - documentStore.datums  → diff add / remove / frame change → sync the layer
 *   - selectionStore        → mirror datum selection + hover into the layer tint
 *
 * The LAYER is a viewport contribution now (`modules/modeling/datumViewport`),
 * so this file no longer talks to the engine at all: it drives whatever visuals
 * the contribution has published, and does nothing while none are attached. The
 * contribution owns disposal; detach just unsubscribes.
 */
import { getDatumVisuals } from "@/modules/modeling/datumViewport";
import { documentStore, type DatumMeta } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
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
  private readonly unsubs: Array<() => void> = [];

  attach(): void {
    // documentStore.datums (identity change ⇒ re-sync; the LAYER diffs by id +
    // frame, so an unrelated projection that replaces the record object is cheap).
    let prevDatums = documentStore.getState().datums;
    getDatumVisuals()?.sync(datumVisuals(prevDatums)); // initial sweep
    this.unsubs.push(
      documentStore.subscribe((s) => {
        if (s.datums === prevDatums) return;
        prevDatums = s.datums;
        getDatumVisuals()?.sync(datumVisuals(s.datums));
      }),
    );

    // Selection + hover → layer tint (single source of truth: the selection
    // store, fed by both tree clicks and viewport picks).
    const applySelection = (): void => {
      const sel = selectionStore.getState();
      const visuals = getDatumVisuals();
      visuals?.setSelected(sel.selected.filter((r) => r.kind === "datum").map((r) => r.id));
      visuals?.setHover(sel.hover?.kind === "datum" ? sel.hover.id : null);
    };
    applySelection();
    this.unsubs.push(selectionStore.subscribe(applySelection));
  }

  detach(): void {
    for (const u of this.unsubs.splice(0)) u();
  }
}
