import { formatArea, formatLengthWithUnit, formatUnitless } from "@/units/format";
import type { MeasurePick, MeasureSummary } from "@/tools/modelTools/measureTool";

function kindLabel(kind: string): string {
  const trimmed = kind.trim();
  return trimmed.length > 0 ? trimmed : "Element";
}

function bodyLabel(bodyId: string): string {
  const trimmed = bodyId.trim();
  return trimmed.length > 0 ? trimmed : "body unavailable";
}

/** Factual, anchored readout for the fixed panel; no inferred geometry. */
export function pickReadout(pick: MeasurePick): string {
  const kind = kindLabel(pick.kind);
  const body = bodyLabel(pick.bodyId);
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
