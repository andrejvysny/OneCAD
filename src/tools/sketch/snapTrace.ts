/*
 * snapTrace — the observability lane for snap arbitration (SNAP P0).
 *
 * Every accepted candidate, every REJECTED candidate and the reason it lost,
 * the camera metric the distances were measured through, and the latch that was
 * in force. Without this, "why did it snap there?" is unanswerable after the
 * fact, and the later phases' behaviour changes are unreviewable.
 *
 * EMISSION POLICY (log.ts forbids drag-frequency logging, and this sits on the
 * pointer path): a trace is emitted only when
 *   - the accepted candidate ID set CHANGED since the last emission, or
 *   - the sample is a COMMITTED click, or
 *   - the latch was reset, or
 *   - projection failed.
 * An identical pointer frame emits nothing. The latest trace is always RETAINED
 * in memory regardless, so `?vpdebug` and the e2e specs can read it without the
 * log ring being involved at all.
 *
 * Nothing here carries credentials, document paths or raw geometry payloads —
 * candidate rows are ids, kinds, refs and numbers.
 */
import { logDebug } from "@/debug/log";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { gridReachPx, metricNorm } from "./snapTypes";
import type {
  PlaneScreenMetric,
  RejectedSnapCandidate,
  SnapCandidate,
  SnapDecisionTrace,
  SnapLatch,
  SnapSource,
} from "./snapTypes";

/** Monotonic trace id, process-wide. Never reset — a trace id is only ever
 *  compared for ordering/equality, and restarting it would alias two traces. */
let nextTraceId = 1;

export function mintTraceId(): number {
  return nextTraceId++;
}

/** Test-only: rewind the counter so trace-id assertions are stable. */
export function __resetSnapTraceForTests(): void {
  nextTraceId = 1;
  latest = null;
  lastEmittedKey = null;
}

let latest: SnapDecisionTrace | null = null;
let lastEmittedKey: string | null = null;

/** The most recent trace, whether or not it was logged. */
export function latestSnapTrace(): SnapDecisionTrace | null {
  return latest;
}

/** Flatten candidates into the trace's log-safe row shape. */
export function traceCandidates(
  candidates: readonly SnapCandidate[],
): SnapDecisionTrace["candidates"] {
  return candidates.map((c) => ({
    id: c.id,
    kind: c.kind,
    source: c.source,
    claims: c.claims,
    errorPx: round3(c.errorPx),
    biasPx: c.semanticBiasPx,
    scorePx: round3(c.scorePx),
    refs: c.refs.map((r) => `${r.entityId}.${r.position}`),
  }));
}

const round3 = (v: number): number =>
  Number.isFinite(v) ? Math.round(v * 1000) / 1000 : v;

/**
 * Retain `trace`, and log it when the emission policy above says to.
 *
 * `latchReset` is passed explicitly rather than inferred: a latch that reset
 * AND re-acquired the same primary in one frame is still a reset worth seeing.
 */
export function recordSnapTrace(
  trace: SnapDecisionTrace,
  opts: { latchReset?: boolean } = {},
): void {
  latest = trace;
  const key = emissionKey(trace);
  const interesting =
    opts.latchReset === true ||
    trace.origin === "click" ||
    trace.metric === null ||
    key !== lastEmittedKey;
  lastEmittedKey = key;
  if (!interesting) return;
  logDebug("snap", `snap ${trace.origin}: ${describe(trace)}`, {
    traceId: trace.traceId,
    epoch: trace.interactionEpoch,
    gen: trace.sessionGeneration,
    accepted: trace.acceptedIds,
    rejected: trace.rejected.map((r) => `${r.candidateId}:${r.reason}`),
    // FLATTENED on purpose: `log.ts` caps ctx nesting at 2 levels, so a row's
    // `claims`/`refs` arrays would collapse to a placeholder if they rode as
    // objects. The structured rows stay on the retained trace for `?vpdebug`.
    candidates: trace.candidates.map(compactCandidate),
    metric: trace.metric,
    projection: trace.projection,
    gridStep: trace.gridStep,
    cellPx: [round3(trace.uCellPx), round3(trace.vCellPx)],
    radiusPx: [trace.pointRadiusPx, round3(trace.gridRadiusPx)],
    sources: trace.sources,
    priorPrimary: trace.priorLatch.primaryId,
    alt: trace.altHeld,
    ...(opts.latchReset === true ? { latchReset: true } : {}),
  });
}

/** One candidate as a single log line: `id kind/source claims err+bias=score [refs]`. */
export function compactCandidate(c: SnapDecisionTrace["candidates"][number]): string {
  const refs = c.refs.length > 0 ? ` [${c.refs.join(",")}]` : "";
  return `${c.id} ${c.kind}/${c.source} {${c.claims.join("+")}} ${c.errorPx}${
    c.biasPx >= 0 ? "+" : ""
  }${c.biasPx}=${c.scorePx}${refs}`;
}

/** Identity of the DECISION, not of the frame — two consecutive frames that
 *  accept the same candidates collapse to one emission. */
function emissionKey(t: SnapDecisionTrace): string {
  return `${t.acceptedIds.join(",")}|${t.projection}|${t.metric === null ? "nometric" : "ok"}`;
}

function describe(t: SnapDecisionTrace): string {
  if (t.metric === null) return "projection failed";
  if (t.acceptedIds.length === 0) return "no candidate";
  return t.acceptedIds.join(" + ");
}

/**
 * Build a trace from the LEGACY single-winner `SnapResult` (SNAP P0).
 *
 * P0 ships the observability lane BEFORE the arbitration rewrite, so that the
 * before/after of P2 is reviewable against the same trace format. The legacy
 * result carries one winner and no scores, so the synthesized row reports the
 * kind, the source it came from, and a zero bias — the honest encoding of "this
 * engine did not rank on distance-plus-bias". P2 replaces this function with
 * the real accepted set.
 */
export function legacyDecisionTrace(args: {
  result: { point: Point2; kind: string; label: string | null; snapped: boolean };
  raw: Point2;
  client: { x: number; y: number };
  origin: "hover" | "click";
  interactionEpoch: number;
  sessionGeneration: number;
  projection: "perspective" | "orthographic" | "none";
  metric: PlaneScreenMetric | null;
  gridStep: number;
  pointRadiusPx: number;
  sources: Readonly<Record<string, boolean>>;
  altHeld: boolean;
  priorLatch: SnapLatch;
}): SnapDecisionTrace {
  const id = args.result.snapped ? `legacy:${args.result.kind}` : "";
  const m = args.metric;
  const errorPx = m ? metricNorm(m, args.result.point.x - args.raw.x, args.result.point.y - args.raw.y) : 0;
  return {
    traceId: mintTraceId(),
    interactionEpoch: args.interactionEpoch,
    sessionGeneration: args.sessionGeneration,
    origin: args.origin,
    raw: args.raw,
    client: args.client,
    projection: args.projection,
    metric: m ? { ...m } : null,
    gridStep: args.gridStep,
    uCellPx: m ? metricNorm(m, args.gridStep, 0) : 0,
    vCellPx: m ? metricNorm(m, 0, args.gridStep) : 0,
    pointRadiusPx: args.pointRadiusPx,
    gridRadiusPx: m ? gridReachPx(m, args.gridStep, args.pointRadiusPx) : 0,
    sources: args.sources,
    candidates: id
      ? [
          {
            id,
            kind: args.result.kind as SnapDecisionTrace["candidates"][number]["kind"],
            source: legacySourceOf(args.result.kind),
            claims: ["point"],
            errorPx: round3(errorPx),
            biasPx: 0,
            scorePx: round3(errorPx),
            refs: [],
          },
        ]
      : [],
    priorLatch: args.priorLatch,
    acceptedIds: id ? [id] : [],
    rejected: [],
    point: args.result.point,
    altHeld: args.altHeld,
  };
}

function legacySourceOf(kind: string): SnapSource {
  switch (kind) {
    case "onCurve":
      return "curve";
    case "alignH":
    case "alignV":
    case "alignHV":
      return "guide";
    case "polar":
      return "polar";
    case "grid":
      return "grid";
    default:
      return "geometryPoint";
  }
}

/** A trace for the "the cursor could not be projected at all" case, which has
 *  no metric, no candidates and no point — but still has to be explainable. */
export function projectionFailureTrace(args: {
  interactionEpoch: number;
  sessionGeneration: number;
  origin: "hover" | "click";
  client: { x: number; y: number } | null;
  altHeld: boolean;
  rejected?: readonly RejectedSnapCandidate[];
  priorLatch: SnapLatch;
}): SnapDecisionTrace {
  return {
    traceId: mintTraceId(),
    interactionEpoch: args.interactionEpoch,
    sessionGeneration: args.sessionGeneration,
    origin: args.origin,
    raw: null,
    client: args.client,
    projection: "none",
    metric: null,
    gridStep: 0,
    uCellPx: 0,
    vCellPx: 0,
    pointRadiusPx: 0,
    gridRadiusPx: 0,
    sources: {},
    candidates: [],
    priorLatch: args.priorLatch,
    acceptedIds: [],
    rejected: args.rejected ?? [],
    point: null,
    altHeld: args.altHeld,
  };
}
