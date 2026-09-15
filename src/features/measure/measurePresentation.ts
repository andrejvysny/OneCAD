import { formatArea, formatLengthWithUnit, formatUnitless } from "@/units/format";
import type { MeasurePick, MeasureSummary } from "@/tools/modelTools/measureTool";
import type { BodyMeta } from "@/stores/documentStore";

function kindLabel(kind: string): string {
  const trimmed = kind.trim();
  return trimmed.length > 0 ? trimmed : "Element";
}

/**
 * The body's display name — same facts `ActiveToolTargets` reads off
 * `documentStore.bodies` — falling back to the raw id only when the body is
 * gone from the projection (docs/qa/UX_REVIEW_2026-09-14.md N5: a UUID is not
 * a fact the user asked for).
 */
export function bodyLabel(bodyId: string, bodies?: Record<string, BodyMeta>): string {
  const trimmed = bodyId.trim();
  if (trimmed.length === 0) return "body unavailable";
  const name = bodies?.[trimmed]?.name;
  return name && name.trim().length > 0 ? name : trimmed;
}

/** Factual, anchored readout for the fixed panel; no inferred geometry. */
export function pickReadout(pick: MeasurePick, bodies?: Record<string, BodyMeta>): string {
  const kind = kindLabel(pick.kind);
  const body = bodyLabel(pick.bodyId, bodies);
  if (pick.kind === "face") {
    const radius =
      pick.radius === null ? "" : ` · Radius ${formatLengthWithUnit(pick.radius)}`;
    return `${kind} · Body ${body}${radius} · Area ${formatArea(pick.magnitude)}`;
  }
  if (pick.kind === "edge") {
    const diameter =
      pick.radius === null ? "" : ` · Diameter Ø ${formatLengthWithUnit(pick.radius * 2)}`;
    return `${kind} · Body ${body}${diameter} · Length ${formatLengthWithUnit(pick.magnitude)}`;
  }
  return `${kind} · Body ${body} · Value ${formatUnitless(pick.magnitude)}`;
}

export function deltaReadout(summary: MeasureSummary): string {
  return summary.delta
    .map((value, i) => `${["ΔX", "ΔY", "ΔZ"][i]} ${formatLengthWithUnit(value)}`)
    .join("  ");
}

/** Relationship shown only when both picks provide plane evidence. */
export function planePairLabel(summary: MeasureSummary): string | null {
  if (summary.planeOffset !== null) {
    return `Parallel planes, ${formatLengthWithUnit(summary.planeOffset)} apart`;
  }
  const angle = summary.angle;
  if (!angle) return null;
  const deg = (value: number) => `${formatUnitless(value)}°`;
  return angle.isRight
    ? `Angle: ${deg(angle.acuteDeg)}`
    : `Angle: ${deg(angle.acuteDeg)} / ${deg(angle.obtuseDeg)}`;
}
