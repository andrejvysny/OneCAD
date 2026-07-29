/*
 * projectionHydration — the `projection-updated` → documentStore bridge (F-WP8
 * flag 2). Per the plan the frontend owns the projection stores and they are
 * "written only by backend events"; this pure function is the write path.
 *
 * ── Revision reconciliation ───────────────────────────────────────────────────
 * A regen emits several projections around one edit (pre-regen, then post-regen),
 * and a tool may have applied an OPTIMISTIC result already. To avoid clobbering
 * newer state with a stale projection, a payload is applied ONLY when its revision
 * is >= the store's current revision (newer-or-equal authoritative wins; a stale
 * lower-revision projection is dropped). The empty projection (status "empty",
 * revision 0 — emitted on close) always resets the store.
 */
import { documentStore, type DocumentProjection, type SketchStatus } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import type { DocumentProjectionWire, FeatureRecord } from "./types";

/** Coerce a wire sketch status token to the store's `SketchStatus`. */
function sketchStatus(s: string): SketchStatus {
  return s === "ok" || s === "under" || s === "over" || s === "error" ? s : "under";
}

/** Map the wire projection to the store projection (field-identical shapes). */
export function projectionToStore(p: DocumentProjectionWire): DocumentProjection {
  const sketches: DocumentProjection["sketches"] = {};
  for (const [id, s] of Object.entries(p.sketches)) {
    sketches[id] = {
      id: s.id,
      name: s.name,
      visible: s.visible,
      dof: s.dof,
      status: sketchStatus(s.status),
      geometryToken: s.geometryToken,
    };
  }
  const features = p.features.map((f: FeatureRecord) => ({
    id: f.id,
    kind: f.kind,
    // Re-edits route on the exact authored opType, not the coarse `kind` bucket
    // (the backend folds Chamfer into "fillet", patterns/mirror into "boolean").
    opType: f.opType,
    label: f.label,
    valueText: f.valueText,
    status: f.status,
    // Carry the worker failure reason through to the store so the HistoryList row
    // tooltips it (MODEL-HARDEN W0.5). `undefined` for any non-error feature.
    statusMessage: f.statusMessage,
  }));
  return {
    status: p.status,
    revision: p.revision,
    title: p.title,
    dirty: p.dirty,
    bodies: { ...p.bodies },
    sketches,
    features,
  };
}

/**
 * On the transition into a FRESHLY-OPENED document (store was empty → now ready),
 * surface a one-shot info hint when ops sit beyond the rollback bar
 * (`appliedOps < totalOps`) — the legacy-draft recovery affordance (MODEL-HARDEN
 * W0 bug: documents saved under the append-draft defect reopen with unapplied ops).
 * Guarded to the open transition so a normal post-edit snapshot never hints.
 */
function maybeHintUnappliedDrafts(p: DocumentProjectionWire): void {
  const applied = p.appliedOps;
  const total = p.totalOps;
  if (applied === undefined || total === undefined || applied >= total) return;
  const n = total - applied;
  viewportStore.getState().setStatusHint(
    `${n} operation${n === 1 ? "" : "s"} not applied — use history roll-to-here to apply`,
    { severity: "info" },
  );
}

/**
 * Apply an authoritative projection to `documentStore`, reconciling by revision.
 * The empty projection (close) always resets; otherwise a payload is written only
 * when it is newer-or-equal to the store's revision. Returns whether it applied.
 */
export function applyProjectionToStore(p: DocumentProjectionWire): boolean {
  const store = documentStore.getState();
  const isEmpty = p.status === "empty";
  if (!isEmpty && p.revision < store.revision) return false; // stale — drop
  const wasEmpty = store.status === "empty"; // the fresh-open transition detector
  store.applySnapshot(projectionToStore(p));
  if (!isEmpty && wasEmpty) maybeHintUnappliedDrafts(p);
  return true;
}
