/*
 * Document projection store (F-WP3 mock).
 *
 * Per plan, projection stores are "written only by backend events". For the
 * chrome-only WP there is no worker yet, so `seedMockDocument()` stands in for
 * the first `applySnapshot`, and `applySnapshot` / `applyChange` are the write
 * path a later IPC WP will call. Shape mirrors the Rust projection DTO the plan
 * describes (bodies / sketches registries + a linear feature timeline).
 */
import { createStore, useStore } from "zustand";

import type { FeaturePrimaryKind, Rgba, SketchHostFace, SketchPlane } from "@/ipc/types";

export type DocStatus = "empty" | "loading" | "ready";

/**
 * Where the geometry on screen came from (backend-authoritative; mirrors the
 * Rust `DocumentProjection::geometry_source`).
 *
 * - `"live"` — a regen has published; this IS the document's geometry.
 * - `"cached"` — nothing has regenerated yet, so the container's LAST-SAVED
 *   meshes are being served (a reopened document paints instantly instead of
 *   waiting on the worker). Correct as of the last save and possibly OLDER than
 *   the timeline the tree shows, which is exactly why the viewport labels it.
 * - `"none"` — nothing to paint.
 */
export type GeometrySource = "none" | "cached" | "live";

export interface BodyMeta {
  id: string;
  name: string;
  visible: boolean;
  /** User-authored body color as `[r, g, b, a]`. Absent = theme neutral body fill. */
  color?: Rgba;
  /** User-authored face colors keyed by persistent `ElementId`. */
  faceColors?: Record<string, Rgba>;
}

/** 'under' = under-constrained, 'over' = over-constrained (mock statuses). */
export type SketchStatus = "ok" | "under" | "over" | "error";

export interface SketchMeta {
  id: string;
  name: string;
  visible: boolean;
  dof: number;
  status: SketchStatus;
  /** Stable authoritative geometry identity; metadata-only changes preserve it. */
  geometryToken: string;
  /**
   * The model face this sketch is hosted on ({@link SketchHostFace}), when it is
   * face-attached. Backend-owned (`SketchDto.hostFace`); the ONE place the
   * frontend writes it itself is the optimistic row `SketchController` mints for
   * a sketch it has just created on a face, exactly as it fabricates `name` /
   * `dof` / `geometryToken` there — the next projection overwrites all four with
   * backend truth. Absent for world- and datum-hosted sketches.
   */
  hostFace?: SketchHostFace;
}

/**
 * A datum (reference) plane in the tree.
 *
 * `basePlaneId`/`offset` are the DEFINITION the user authored; `plane` is the
 * **backend-resolved** frame and is authoritative. The core stamps a sketch
 * attached to this datum with exactly this basis, so anything that draws or
 * previews on a datum must read `plane` — re-deriving it from `basePlaneId +
 * offset` on the frontend would reintroduce the two-sources-of-truth bug the
 * migration exists to remove. (The bases are also NON-STANDARD: repo-kind "XZ"
 * has world normal +X and "YZ" has +Y, so "XZ offset 10" moves along world +X.)
 *
 * `plane.kind` is always `"custom"` — a datum frame has an arbitrary origin, and
 * `custom` is exactly what the backend sends for a datum-hosted sketch
 * (SCHEMA §7.3).
 */
export interface DatumMeta {
  id: string;
  name: string;
  /** `"XY"` | `"XZ"` | `"YZ"`, or another datum's id (chained offsets). */
  basePlaneId: string;
  offset: number;
  plane: SketchPlane;
  /** `false` ⇒ the definition did not resolve; the datum cannot host a sketch. */
  resolvedValid: boolean;
}

export type FeatureKind =
  | "sketch"
  | "extrude"
  | "revolve"
  | "fillet"
  | "boolean"
  | "shell"
  | "linearPattern"
  | "circularPattern"
  | "mirror";

export type FeatureStatus = "ok" | "dirty" | "error" | "needsRepair";

export interface FeatureMeta {
  id: string;
  kind: FeatureKind;
  /** Exact authored `opType` — see `FeatureRecord.opType`; re-edits route on it. */
  opType?: string;
  label: string;
  /** Mono value shown on the right of the history chip, e.g. "83.3 mm".
   *  MILLIMETRE-FIXED (see `FeatureRecord.valueText`) — the row RENDERS
   *  {@link FeatureMeta.primaryValue} through the display unit instead, and falls
   *  back to this string only when there is no primary dimension. */
  valueText: string;
  /** The ONE dimension the history row edits in place (H3), document domain: mm
   *  for a length/diameter, DEGREES for an angle. Absent ⇒ read-only row. */
  primaryValue?: number;
  /** Domain of {@link FeatureMeta.primaryValue}; absent whenever it is. */
  primaryValueKind?: FeaturePrimaryKind;
  status: FeatureStatus;
  /** Worker failure reason for an errored feature (`status === "error"`), shown as
   *  the HistoryList row tooltip (MODEL-HARDEN W0.5). Absent for any other status. */
  statusMessage?: string;
  /** Whether the feature is suppressed (backend-authoritative; see `FeatureRecord.suppressed`). */
  suppressed?: boolean;
}

/** The full document projection (everything the chrome renders from). */
export interface DocumentProjection {
  status: DocStatus;
  /**
   * The document this projection describes (backend-authoritative). Revisions
   * are meaningful only within one documentId — the hydration guard resets on a
   * documentId change instead of comparing revisions across documents. Absent
   * in the mock lane (same-document semantics).
   */
  documentId?: string;
  revision: number;
  title: string;
  dirty: boolean;
  bodies: Record<string, BodyMeta>;
  sketches: Record<string, SketchMeta>;
  /** Datum planes keyed by id (backend-authoritative; see {@link DatumMeta}). */
  datums: Record<string, DatumMeta>;
  features: FeatureMeta[];
  /**
   * Applied op count (the timeline CURSOR): `features[0, appliedOps)` are applied,
   * `[appliedOps, features.length)` are drafts beyond the rollback bar. Backend-
   * authoritative — hydrated from every `projection-updated`.
   *
   * H7b: an `ApplyOperationResult` now CARRIES the cursor (`ApplyOperationResult
   * .appliedOps`, taken from the command's own projection on the real lane and
   * from the mock cursor on the mock lane), so a rollback is visible immediately
   * instead of waiting for the next projection. {@link nextAppliedOps} stays as
   * the fallback for a result that has none.
   *
   * `totalOps` is deliberately NOT a separate field: the projection's `features`
   * array is built from ALL timeline records (`document_runtime.rs` maps
   * `timeline.records()` whole, and `total_ops` is `timeline.len()`), so
   * `features.length` IS the total — a second copy could only ever disagree.
   */
  appliedOps: number;
  /**
   * Provenance of the geometry the viewport is showing ({@link GeometrySource}).
   * Backend-authoritative. A payload from a backend older than the field — and
   * the mock lane, which omits it — hydrates as `"none"`, the reading that shows
   * no chip: claiming geometry is live when we were never told so is exactly the
   * silent-wrong-answer class this codebase refuses.
   */
  geometrySource: GeometrySource;
}

export interface DocumentState extends DocumentProjection {
  /**
   * How many regen jobs are in flight (H7b). NOT a projection fact — it is
   * transport state, so it deliberately survives `applySnapshot` (a projection
   * lands DURING a regen) and is cleared only by {@link DocumentState.regenIdle}.
   * The status bar shows "Rebuilding…" while it is > 0.
   */
  regenBusy: number;
  /** A regen job started (`regen-started`). */
  regenStarted(): void;
  /** A regen job completed (`regen-finished`). CLAMPED at zero: a no-op regen
   *  finishes without ever having started, so completions outnumber starts. */
  regenSettled(): void;
  /** Forget all in-flight regens (document replaced/closed — nothing will finish). */
  regenIdle(): void;
  /** Replace the whole projection (backend snapshot event). */
  applySnapshot(snapshot: DocumentProjection): void;
  /** Merge a partial projection delta (backend change event). */
  applyChange(change: Partial<DocumentProjection>): void;
  /** Local visibility flip for a body or sketch (drives the tree eye toggle). */
  setVisibility(id: string, visible: boolean): void;
  /** Register a sketch (e.g. a freshly created one) in the tree/registry. */
  addSketch(meta: SketchMeta): void;
  /** Remove a sketch from the tree/registry (idempotent; no-op when absent). */
  removeSketch(id: string): void;
  /** Register a datum plane in the tree/registry (mock lane / optimistic add). */
  addDatum(meta: DatumMeta): void;
  /** Remove a datum from the tree/registry (idempotent; no-op when absent). */
  removeDatum(id: string): void;
  /** Push a solver result onto a sketch (drives chrome bar + inspector DOF). */
  setSketchSolve(id: string, dof: number, status: SketchStatus): void;
  /**
   * Stamp a fresh LOCAL geometry token on a sketch (no-op when it is not registered).
   *
   * For the one case the frontend knows a sketch's geometry moved but no backend
   * projection will say so: the sketch→sketch switch closes A on the wire while the
   * mock lane publishes nothing, leaving A's always-visible static layer showing its
   * entry-time content. `local:<n>` is monotonic within a session and cannot collide
   * with a backend token — `applySnapshot` replaces the whole `sketches` record, so a
   * local stamp never has to coexist with a hydrated one for the same id.
   */
  bumpSketchGeometry(id: string): void;
}

/**
 * The rollback cursor to carry across a timeline REPLACEMENT that arrived on an
 * edit result rather than a projection.
 *
 * An `ApplyOperationResult` has no cursor of its own, so the two result-hydration
 * paths (`historyActions.applyEditResult`, `ModelToolController.applyResult`) have
 * to derive one. The rule, and why:
 *   - a FULLY-APPLIED timeline stays fully applied — an append (commit a new op)
 *     or a delete must not leave the newest row looking like an unapplied draft;
 *   - a ROLLED-BACK one keeps its cursor, clamped into the new length, because an
 *     edit result cannot tell us the bar moved.
 * The authoritative value rides the next `projection-updated` either way; the H3
 * inline-edit gate reads this in the meantime.
 */
export function nextAppliedOps(
  prevApplied: number,
  prevLength: number,
  nextLength: number,
): number {
  return prevApplied >= prevLength ? nextLength : Math.min(prevApplied, nextLength);
}

/** Monotonic counter behind the `local:<n>` tokens {@link DocumentState.bumpSketchGeometry} mints. */
let localGeometryTokenSeq = 0;

/** Map the wire solver state (SCHEMA §7.4) → the tree/inspector status. */
export function docSketchStatus(
  state: "UnderConstrained" | "FullyConstrained" | "OverConstrained" | "Conflicting",
): SketchStatus {
  switch (state) {
    case "FullyConstrained":
      return "ok";
    case "OverConstrained":
      return "over";
    case "Conflicting":
      return "error";
    default:
      return "under";
  }
}

/**
 * Next free "Sketch N" name — one past the highest existing `Sketch <n>` index
 * (min "Sketch 1"). Names that don't match the pattern are ignored, so a
 * freshly plane-picked sketch always gets a distinct, monotonic tree name.
 */
export function nextSketchName(sketches: Record<string, SketchMeta>): string {
  let max = 0;
  for (const s of Object.values(sketches)) {
    const m = /^Sketch (\d+)$/.exec(s.name);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return `Sketch ${max + 1}`;
}

/** Next free "Datum N" name — same rule as {@link nextSketchName}. */
export function nextDatumName(datums: Record<string, DatumMeta>): string {
  let max = 0;
  for (const d of Object.values(datums)) {
    const m = /^Datum (\d+)$/.exec(d.name);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return `Datum ${max + 1}`;
}

/**
 * Bracket-like demo document. The body/sketch registries mirror prototype 1c's
 * tree exactly (Body 1; Sketch 2 / 4 / 5) so the flagship screen renders
 * pixel-faithfully, while `features` carries the full timeline the plan lists
 * (Sketch 1 → Extrude 83.3 → Fillet 2.0 → Sketch 2 → Extrude 12.0 — two bodies
 * and two sketches at the history level). Values match the prototype inspector
 * HISTORY verbatim (83.3 mm / 2.0 mm / 12.0 mm).
 */
export function seedMockDocument(): DocumentProjection {
  return {
    status: "ready",
    revision: 5,
    title: "Bracket v2",
    dirty: false,
    bodies: {
      body1: { id: "body1", name: "Body 1", visible: true },
    },
    sketches: {
      sketch2: {
        id: "sketch2",
        name: "Sketch 2",
        visible: true,
        dof: 3,
        status: "under",
        geometryToken: "mock:sketch2:v1",
      },
      sketch4: {
        id: "sketch4",
        name: "Sketch 4",
        visible: true,
        dof: 0,
        status: "ok",
        geometryToken: "mock:sketch4:v1",
      },
      sketch5: {
        id: "sketch5",
        name: "Sketch 5",
        visible: false,
        dof: 0,
        status: "ok",
        geometryToken: "mock:sketch5:v1",
      },
    },
    // The demo document has no datums — the datum tool authors them at runtime.
    datums: {},
    features: [
      { id: "f1", kind: "sketch", opType: "Sketch", label: "Sketch 1", valueText: "", status: "ok" },
      { id: "f2", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "83.3 mm", primaryValue: 83.3, primaryValueKind: "length", status: "ok" },
      { id: "f3", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", primaryValue: 2, primaryValueKind: "length", status: "ok" },
      { id: "f4", kind: "sketch", opType: "Sketch", label: "Sketch 2", valueText: "", status: "ok" },
      { id: "f5", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "12.0 mm", primaryValue: 12, primaryValueKind: "length", status: "ok" },
    ],
    // The whole demo timeline is applied (no rollback bar in the seed).
    appliedOps: 5,
    // The demo's meshes are synthesized on the spot, not read back from a
    // container — "live" is the truthful label, and it keeps the cached chip out
    // of every mock-lane and e2e run.
    geometrySource: "live",
  };
}

/** The "no document open" projection (mirrors Rust `DocumentProjection::empty`). */
export function emptyDocument(): DocumentProjection {
  return {
    status: "empty",
    revision: 0,
    title: "",
    dirty: false,
    bodies: {},
    sketches: {},
    datums: {},
    features: [],
    appliedOps: 0,
    geometrySource: "none",
  };
}

/**
 * The store's initial projection. Under a real Tauri webview the app starts EMPTY
 * and hydrates from the backend's first `projection-updated` (no mock document);
 * in a plain browser / vitest / Playwright there is no backend, so the
 * `seedMockDocument()` demo drives the whole UI (tests depend on the seed).
 */
function initialDocument(): DocumentProjection {
  const underTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  return underTauri ? emptyDocument() : seedMockDocument();
}

export const documentStore = createStore<DocumentState>()((set) => ({
  ...initialDocument(),
  regenBusy: 0,

  regenStarted() {
    set((s) => ({ regenBusy: s.regenBusy + 1 }));
  },

  regenSettled() {
    set((s) => ({ regenBusy: Math.max(0, s.regenBusy - 1) }));
  },

  regenIdle() {
    set({ regenBusy: 0 });
  },

  applySnapshot(snapshot) {
    set(snapshot);
  },

  applyChange(change) {
    set(change);
  },

  setVisibility(id, visible) {
    set((s) => {
      if (s.bodies[id]) {
        return { bodies: { ...s.bodies, [id]: { ...s.bodies[id], visible } } };
      }
      if (s.sketches[id]) {
        return { sketches: { ...s.sketches, [id]: { ...s.sketches[id], visible } } };
      }
      return {};
    });
  },

  addSketch(meta) {
    set((s) => ({ sketches: { ...s.sketches, [meta.id]: meta } }));
  },

  removeSketch(id) {
    set((s) => {
      if (!s.sketches[id]) return {};
      const sketches = { ...s.sketches };
      delete sketches[id];
      return { sketches };
    });
  },

  addDatum(meta) {
    set((s) => ({ datums: { ...s.datums, [meta.id]: meta } }));
  },

  removeDatum(id) {
    set((s) => {
      if (!s.datums[id]) return {};
      const datums = { ...s.datums };
      delete datums[id];
      return { datums };
    });
  },

  setSketchSolve(id, dof, status) {
    set((s) => {
      const sketch = s.sketches[id];
      if (!sketch) return {};
      return { sketches: { ...s.sketches, [id]: { ...sketch, dof, status } } };
    });
  },

  bumpSketchGeometry(id) {
    set((s) => {
      const sketch = s.sketches[id];
      if (!sketch) return {};
      const geometryToken = `local:${++localGeometryTokenSeq}`;
      return { sketches: { ...s.sketches, [id]: { ...sketch, geometryToken } } };
    });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useDocumentStore<T>(selector: (s: DocumentState) => T): T {
  return useStore(documentStore, selector);
}

/**
 * True while the viewport is painting the container's LAST-SAVED meshes rather
 * than regenerated ones — the "Last saved geometry" chip's condition.
 *
 * A named selector, not an inline lambda, because the chip is not the only thing
 * that should care: anything offering to MEASURE or EXPORT off screen geometry
 * has to be able to ask the same question from the same place.
 */
export function selectGeometryCached(s: DocumentState): boolean {
  return s.geometrySource === "cached";
}
