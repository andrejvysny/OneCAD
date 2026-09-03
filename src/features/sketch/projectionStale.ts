/*
 * The `PROJECTION_STALE` verdict, derived from the feature timeline (WP-P B4).
 *
 * Rust hangs the diagnostic on the SKETCH's own timeline step as a WARNING with
 * structured `evidence: { sketchId, entityIds }` (`document_runtime.rs::
 * step_diagnostics`). The sketch id is what attributes a verdict to the OPEN
 * sketch when two sketches hold projections; the entity ids are backend uuids
 * (for a sketch the frontend minted, identical to the frontend ids — the backend
 * adopts them). The `message` is display text only; its leading count is used
 * solely as a fallback for a diagnostic without evidence.
 */
import { PROJECTION_STALE_CODE } from "@/ipc/types";
import type { FeatureMeta } from "@/stores/documentStore";

export interface ProjectionStaleVerdict {
  /** Stale entity count: `evidence.entityIds.length`, else the message's leading
   *  number, else `0` (the banner then omits the count rather than inventing one). */
  count: number;
  /** Backend entity uuids named by the evidence (empty without evidence). */
  entityIds: string[];
  /** The backend's sentence, verbatim: the detail line / tooltip. */
  message: string;
}

interface StaleEvidence {
  sketchId?: string;
  entityIds?: string[];
}

function evidenceOf(raw: unknown): StaleEvidence {
  if (!raw || typeof raw !== "object") return {};
  const rec = raw as Record<string, unknown>;
  const sketchId = typeof rec.sketchId === "string" ? rec.sketchId : undefined;
  const ids = Array.isArray(rec.entityIds)
    ? rec.entityIds.filter((v): v is string => typeof v === "string")
    : undefined;
  return { sketchId, entityIds: ids };
}

/**
 * The stale-projection verdict carried by the timeline, or `null`.
 *
 * With `activeSketchId`, a diagnostic whose evidence names ANOTHER sketch is
 * ignored; a diagnostic without evidence is attributed to the open sketch (the
 * pre-evidence behaviour), never dropped.
 */
export function projectionStaleVerdict(
  features: readonly FeatureMeta[],
  activeSketchId?: string,
): ProjectionStaleVerdict | null {
  for (const feature of features) {
    if (feature.kind !== "sketch") continue;
    const hit = feature.diagnostics?.find((d) => d.code === PROJECTION_STALE_CODE);
    if (!hit) continue;
    const ev = evidenceOf(hit.evidence);
    if (activeSketchId && ev.sketchId && ev.sketchId !== activeSketchId) continue;
    const entityIds = ev.entityIds ?? [];
    const leading = /^(\d+)\b/.exec(hit.message);
    const count = ev.entityIds ? entityIds.length : leading ? Number(leading[1]) : 0;
    return { count, entityIds, message: hit.message };
  }
  return null;
}
