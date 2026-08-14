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
import { documentStore, type DocumentProjection, type FeatureMeta, type SketchStatus } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import type { DocumentProjectionWire, FeatureRecord } from "./types";

/** Coerce a wire sketch status token to the store's `SketchStatus`. */
function sketchStatus(s: string): SketchStatus {
  return s === "ok" || s === "under" || s === "over" || s === "error" ? s : "under";
}

/**
 * The ONE `FeatureRecord` (wire) → `FeatureMeta` (store) mapping — every site that
 * hydrates a feature timeline (projection snapshot, `ApplyOperationResult` in
 * `historyActions`, `ModelToolController.applyResult`) must go through this so a
 * field added to one never silently drops out of another (a past bug: two
 * hand-rolled copies dropped `statusMessage`).
 */
export function toFeatureMeta(f: FeatureRecord): FeatureMeta {
  return {
    id: f.id,
    kind: f.kind,
    // Re-edits route on the exact authored opType, not the coarse `kind` bucket
    // (the backend folds Chamfer into "fillet", patterns/mirror into "boolean").
    opType: f.opType,
    label: f.label,
    valueText: f.valueText,
    // The inline-editable primary dimension (H3). Carried as a PAIR — a value
    // without its kind would render a degree as a millimetre.
    primaryValue: f.primaryValue,
    primaryValueKind: f.primaryValueKind,
    // The variable binding behind that number (WP-VE.2). Carried here and NOWHERE
    // else: the row may only render `=name` from what the backend recorded.
    primaryExpr: f.primaryExpr,
    status: f.status,
    // Carry the worker failure reason through to the store so the HistoryList row
    // tooltips it (MODEL-HARDEN W0.5). `undefined` for any non-error feature.
    statusMessage: f.statusMessage,
    diagnostics: f.diagnostics,
    suppressed: f.suppressed,
  };
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
      // Face-hosted sketches only; the backend omits it everywhere else, and the
      // store treats `undefined` as "not hosted on a face" (SKETCH-ON-FACE W3).
      hostFace: s.hostFace,
    };
  }
  // Datums: the resolved `plane` is carried VERBATIM (never re-derived from
  // basePlaneId+offset — the backend is the basis authority) and tagged `custom`,
  // which is the plane kind a datum frame takes on the wire (SCHEMA §7.3).
  // `?? {}` is a belt-and-braces guard: the field is required on the wire, but a
  // dev webview pointed at an older backend build would otherwise take down ALL
  // hydration, not just the datums.
  const datums: DocumentProjection["datums"] = {};
  for (const [id, d] of Object.entries(p.datums ?? {})) {
    datums[id] = {
      id: d.id,
      name: d.name,
      basePlaneId: d.basePlaneId,
      offset: d.offset,
      plane: { kind: "custom", ...d.plane },
      resolvedValid: d.resolvedValid,
    };
  }
  const features = p.features.map(toFeatureMeta);
  return {
    status: p.status,
    documentId: p.documentId,
    revision: p.revision,
    title: p.title,
    dirty: p.dirty,
    bodies: { ...p.bodies },
    sketches,
    datums,
    features,
    // The rollback cursor, AUTHORITATIVE (H7b: it drives the grayed rows + the
    // "N operations rolled back" banner, not just the one-shot open hint). A
    // payload from a backend older than the field (or the mock lane's derived
    // projection) means "everything is applied" — the permissive reading the
    // legacy-draft hint already assumes when it is absent. CLAMPED, because a
    // cursor past the rows we can render would leave the marker off the end.
    //
    // `p.totalOps` is intentionally not carried into the store: the projection's
    // `features` are built from ALL timeline records and `total_ops` is
    // `timeline.len()` (document_runtime.rs), so `features.length` IS the total.
    appliedOps: Math.min(p.appliedOps ?? features.length, features.length),
    // Absent (mock lane, or a backend older than the field) ⇒ "none": no chip.
    // Anything unrecognised is treated the same way rather than trusted — the
    // chip must never be driven by a token this build does not understand.
    geometrySource:
      p.geometrySource === "cached" || p.geometrySource === "live"
        ? p.geometrySource
        : "none",
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
  // Revisions restart at 1 for every newly opened runtime, so the stale guard is
  // meaningful only WITHIN one document: a projection for a different documentId
  // is a document replacement and always applies (comparing revisions across
  // documents dropped every projection of a freshly opened document while the
  // store still held the old one's high revision). Either side missing its id
  // (mock lane, pre-hydration store) ⇒ same-document semantics.
  const sameDocument =
    p.documentId === undefined ||
    store.documentId === undefined ||
    p.documentId === store.documentId;
  if (!isEmpty && sameDocument && p.revision < store.revision) return false; // stale — drop
  const wasEmpty = store.status === "empty"; // the fresh-open transition detector
  store.applySnapshot(projectionToStore(p));
  if (!isEmpty && wasEmpty) maybeHintUnappliedDrafts(p);
  return true;
}
