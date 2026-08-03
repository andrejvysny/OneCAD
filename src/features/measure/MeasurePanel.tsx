/*
 * MeasurePanel — the docked reading for the Measure tool (WP-C1).
 *
 * The floating `MeasureOverlay` labels answer "what did I just click?" AT the
 * geometry. This panel answers the questions that have no single world anchor:
 * the BODY's mass properties (volume / surface area / centroid) and the derived
 * plane relationship (angle, or offset when the two faces are parallel). Those
 * belong to a whole body or to a pair, so pinning them to one pick's centre
 * would misattribute them.
 *
 * ── Why the mass fetch lives here, not in `ModelToolController` ──────────────
 * A pick is imperative and the controller owns it. A body's mass properties are
 * a DERIVED, async read keyed off the picks the controller already published —
 * nothing else in the app consumes them, and routing them through the controller
 * would add a relay with no reader. So this component owns the effect, writes
 * the result into `measureStore` (so the whole reading clears as one unit and
 * stays visible to e2e's `__stores` surface), and guards every render on
 * `mass.bodyId` matching the measured body, which is what makes a late response
 * for a body the user has since moved off unable to caption the wrong solid.
 *
 * ── "Clicking a body" ───────────────────────────────────────────────────────
 * There is no body-level pick gesture in the viewport: a click on a solid
 * raycasts to one of its FACES (`Picker.ts`), and the measure tool routes
 * face/edge refs. Every such pick names its owning body, so measuring anything
 * on a solid reports that solid's mass properties — which is the behaviour
 * "click a body, see its volume" describes, reached through the pick the app
 * actually has.
 */
import { useEffect } from "react";
import { createClient } from "@/ipc/client";
import { measureStore, useMeasureStore } from "@/stores/measureStore";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  currentLengthUnit,
  formatArea,
  formatLength,
  formatLengthWithUnit,
  formatUnitless,
  lengthSuffix,
} from "@/units/format";
import { LENGTH_UNITS, type LengthUnitId } from "@/units/lengthUnits";
import type { MeasureSummary } from "@/tools/modelTools/measureTool";
import { traceWarn } from "@/debug/trace";

/**
 * Format a VOLUME (given in mm³) with its unit — `"144000 mm³"`.
 *
 * The exact analogue of `units/format.formatArea`, one power up: the conversion
 * factor is `mmPer` CUBED. Routing a volume through `formatLength` would divide
 * by 25.4 once instead of thrice and under-report an inch³ by ~645×, which is
 * the whole reason area got its own function rather than a shared one.
 *
 * It lives here rather than in `units/format` only because that module is owned
 * by another work package in flight; the parallel is deliberate and it should
 * move there when the two land.
 */
export function formatVolume(mm3: number, unit: LengthUnitId = currentLengthUnit()): string {
  if (!Number.isFinite(mm3)) return String(mm3);
  // `formatLength` already divides by `mmPer` once (and applies THIS unit's
  // decimal precision + trailing-zero trim), so pre-dividing by mmPer² lands on
  // mmPer³ overall. Doing the full cube here and then calling `formatLength`
  // would divide a fourth time.
  const f = LENGTH_UNITS[unit].mmPer;
  return `${formatLength(mm3 / (f * f), unit)} ${lengthSuffix(unit)}³`;
}

/** The body the reading describes: the NEWEST pick's owner. */
export function measuredBodyId(picks: ReadonlyArray<{ bodyId: string }>): string | null {
  return picks.length === 0 ? null : picks[picks.length - 1].bodyId;
}

/** `"12.5, -3, 0"` — a point, each ordinate through the shared length formatter. */
export function formatPoint(p: readonly [number, number, number]): string {
  return `${formatLength(p[0])}, ${formatLength(p[1])}, ${formatLength(p[2])}`;
}

/**
 * The plane-pair line, or null when the two picks are not both planes.
 *
 * Both angle forms are shown (`"60° / 120°"`) because a pair of planes subtends
 * two supplementary dihedrals and the descriptor carries no evidence of which
 * side the material is on — see `measureTool.MeasureAngle`. At exactly 90° the
 * two coincide and only one is shown.
 */
export function planePairLabel(summary: MeasureSummary): string | null {
  if (summary.planeOffset !== null) {
    // Named for what it IS. "Distance" alone would be ambiguous next to the
    // centre-to-centre distance on the same card, and these are different numbers.
    return `Parallel planes, ${formatLengthWithUnit(summary.planeOffset)} apart`;
  }
  const angle = summary.angle;
  if (!angle) return null;
  // `formatUnitless`, NOT `formatLength`: an angle has no length domain, so the
  // display-unit preference must not touch it (60° would render as "2.362°"
  // under inches).
  const deg = (v: number) => `${formatUnitless(v)}°`;
  return angle.isRight
    ? `Angle: ${deg(angle.acuteDeg)}`
    : `Angle: ${deg(angle.acuteDeg)} / ${deg(angle.obtuseDeg)}`;
}

/** One `label: value` row. */
function Row({ label, value, testid }: { label: string; value: string; testid: string }) {
  return (
    <div className="flex items-baseline justify-between gap-4">
      <span className="text-ink-5">{label}</span>
      <span className="font-mono text-ink-2" data-testid={testid}>
        {value}
      </span>
    </div>
  );
}

export function MeasurePanel() {
  const picks = useMeasureStore((s) => s.picks);
  const summary = useMeasureStore((s) => s.summary);
  const mass = useMeasureStore((s) => s.mass);
  // Subscribed, not read: the formatters below resolve the display unit
  // themselves, but nothing re-renders this card when the preference changes
  // unless it depends on the store (the `units/format` note on live bindings).
  useSettingsStore((s) => s.displayUnit);
  const bodyId = measuredBodyId(picks);

  useEffect(() => {
    if (!bodyId) return;
    // Already loaded for this body — a re-pick on the same solid must not refetch.
    if (measureStore.getState().mass?.bodyId === bodyId) return;
    let live = true;
    void createClient()
      .massProperties(bodyId)
      .then((m) => {
        // Two guards, both needed: `live` drops a response whose effect has been
        // torn down, and the id re-check drops one that lost a race to a newer
        // pick on a DIFFERENT body (effects are not ordered by completion).
        if (!live) return;
        if (measuredBodyId(measureStore.getState().picks) !== bodyId) return;
        measureStore.getState().setMass(m);
      })
      .catch((e: unknown) => {
        // An absent body is a real refusal from the backend, not a reading. Leave
        // the card without a mass section rather than showing a fabricated zero.
        traceWarn("measure", "massProperties failed", e instanceof Error ? e.message : String(e));
      });
    return () => {
      live = false;
    };
  }, [bodyId]);

  if (!bodyId) return null;
  const pairLine = summary ? planePairLabel(summary) : null;
  const showMass = mass !== null && mass.bodyId === bodyId;

  return (
    <div
      data-testid="measure-panel"
      className="pointer-events-none absolute bottom-[46px] left-3 z-30 w-[228px] rounded-lg border border-border bg-surface px-3 py-2.5 text-[11.5px] shadow-panel"
    >
      <div className="mb-1.5 text-[10.5px] font-medium uppercase tracking-wide text-ink-5">
        Measurement
      </div>
      <div className="flex flex-col gap-1">
        {showMass && (
          <>
            <Row label="Volume" value={formatVolume(mass.volume)} testid="measure-volume" />
            <Row
              label="Surface area"
              // `formatArea` already carries its own unit — an area is not a
              // length and must not be squared through the length formatter.
              value={formatArea(mass.surfaceArea)}
              testid="measure-surface-area"
            />
            <Row label="Centroid" value={formatPoint(mass.centroid)} testid="measure-centroid" />
          </>
        )}
        {summary && (
          <Row
            label="Center ↔ center"
            value={formatLengthWithUnit(summary.distance)}
            testid="measure-panel-distance"
          />
        )}
        {pairLine && (
          <div className="pt-0.5 font-mono text-ink-2" data-testid="measure-plane-pair">
            {pairLine}
          </div>
        )}
      </div>
    </div>
  );
}
