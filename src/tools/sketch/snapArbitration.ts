/*
 * snapArbitration — DECISION half of the snap engine (PURE).
 *
 * Takes the candidates `snapCandidates.ts` generated plus the phase's numeric
 * fields, and answers ONE question: which compatible SET of intents wins, and
 * what point do they rebuild to?
 *
 * ── Why a set, not a winner ──────────────────────────────────────────────────
 * The legacy engine returned one `SnapKind` off a rigid priority ladder. That
 * shape cannot say "45° AND 25.0 long", or "x from that corner AND y from this
 * one" — the intents are compatible, they claim different degrees of freedom,
 * and collapsing them to one slot forced the engine to throw one away. Worse,
 * an absolute tier ladder lets a FAR endpoint beat a NEAR midpoint purely
 * because endpoints outrank midpoints.
 *
 * So: candidates declare what they CLAIM, are scored on screen distance plus a
 * bounded semantic preference, and the resolver enumerates the small, explicitly
 * enumerated family of compatible combinations.
 *
 * ── Why enumeration, not a greedy pass ───────────────────────────────────────
 * A greedy "take the best, then take the best compatible remainder" depends on
 * the order candidates arrive in, which is exactly the class of bug this
 * program exists to remove. The compatible family is bounded — a full point, a
 * single guide, a guide PAIR, a polar ray, or nothing, each carrying whichever
 * of the phase's numeric fields it leaves free — so it is enumerated in full and
 * ranked by total cost with deterministic tie-breaks.
 */
import type { Point2 } from "@/viewport/engine/sketchBasis";
import {
  aliasOf,
  roundTo,
  type DimFieldId,
  type DimLocks,
  type DimQuantum,
  type DimValues,
} from "./liveDimension";
import type { DimFrame } from "./liveDimFrames";
import { nearestOnRayMetric } from "./snapCandidates";
import {
  metricNorm,
  metricTensor,
  emptyLatch,
  CHALLENGER_FRAMES,
  HYSTERESIS_ADVANTAGE_PX,
  SEMANTIC_BIAS_PX,
  SET_COMPLEXITY_PENALTY_PX,
  SET_DISPLACEMENT_SLACK_PX,
  type PlaneScreenMetric,
  type RejectedSnapCandidate,
  type SnapCandidate,
  type SnapClaim,
  type SnapDecision,
  type SnapKind,
  type SnapLatch,
} from "./snapTypes";

// ── Numeric candidates (the phase's live-dimension fields) ───────────────────

/**
 * Which degree of freedom a numeric field actually CLAIMS in a given phase.
 *
 * Mirrors `liveDimFrames.dimFrame`'s phase table, and the mapping is the whole
 * trick behind the composition rules: a rectangle's `width` is not an abstract
 * "width" claim, it is literally the cursor's X — so an x guide and a rounded
 * width CONFLICT (correctly), while an x guide and a rounded height compose
 * (also correctly) as an ordinary x+y pair. `null` means the field steers no
 * geometry (a polygon's side count) and is not a snap candidate at all.
 */
export function numericClaim(
  toolId: string,
  anchorCount: number,
  field: DimFieldId,
  arcMode: boolean,
): SnapClaim | null {
  switch (toolId) {
    case "line":
      if (arcMode) return field === "radius" ? "radius" : null;
      return field === "length" ? "length" : field === "angle" ? "angle" : null;
    case "rect":
    case "centerRect":
      return field === "width" ? "x" : field === "height" ? "y" : null;
    case "circle":
      return field === "radius" || field === "diameter" ? "radius" : null;
    case "arc":
      if (anchorCount === 1) return field === "radius" ? "radius" : null;
      return field === "angle" ? "angle" : null;
    case "arc3p":
      if (anchorCount === 1)
        return field === "length" ? "length" : field === "angle" ? "angle" : null;
      return field === "radius" ? "radius" : null;
    case "ellipse":
      if (anchorCount === 1) return field === "major" ? "radius" : null;
      return field === "minor" ? "width" : null;
    case "polygon":
      return field === "radius" ? "radius" : null;
    case "slot":
      if (anchorCount === 1)
        return field === "length" ? "length" : field === "angle" ? "angle" : null;
      return field === "width" ? "width" : null;
    default:
      return null;
  }
}

/**
 * The phase's RADIAL composition, when it has one: a centre and the field whose
 * value is the distance from it.
 *
 * This is what makes "a remote x guide plus a rounded length" solvable — the
 * two together describe a circle about `anchor` meeting the line `x = guideX`.
 * Phases with no such centre (a 3-point arc's radius, a tangent arc's radius, a
 * box) return null, and a guide+numeric set there is simply rejected.
 */
export function radialComposition(
  toolId: string,
  anchors: readonly Point2[],
  arcMode: boolean,
): { anchor: Point2; field: DimFieldId } | null {
  const n = anchors.length;
  if (n === 0) return null;
  switch (toolId) {
    case "line":
      return arcMode ? null : { anchor: anchors[n - 1], field: "length" };
    case "slot":
    case "arc3p":
      return n === 1 ? { anchor: anchors[0], field: "length" } : null;
    case "circle":
      return n === 1 ? { anchor: anchors[0], field: "radius" } : null;
    case "arc":
      return n === 1 ? { anchor: anchors[0], field: "radius" } : null;
    case "polygon":
      return n === 1 ? { anchor: anchors[0], field: "radius" } : null;
    case "ellipse":
      return n === 1 ? { anchor: anchors[0], field: "major" } : null;
    default:
      return null;
  }
}

export interface NumericContext {
  frame: DimFrame | null;
  toolId: string;
  anchors: readonly Point2[];
  arcMode: boolean;
  locks: DimLocks;
  /** Null ⇒ cursor rounding is switched off; typed locks still apply. */
  quantum: DimQuantum | null;
}

const DOMAIN_QUANTUM: Record<DimFieldId, "length" | "angle" | "none"> = {
  length: "length",
  angle: "angle",
  width: "length",
  height: "length",
  radius: "length",
  diameter: "length",
  major: "length",
  minor: "length",
  sides: "none",
};

/**
 * Build the phase's numeric candidates.
 *
 * TYPED values are MANDATORY (`authority: "typed"`) — they are exact, never
 * re-rounded, and outrank every geometric candidate that would move the field
 * they own. CURSOR values are ordinary optional candidates whose cost is the
 * screen displacement the rounding introduces, which is what lets them be
 * compared against a geometry snap on one scale.
 *
 * Fields the phase HIDES still participate: a first line leg hides its ∠ chip,
 * yet its angle must still land on a whole degree.
 */
export function numericCandidates(
  ctx: NumericContext,
  raw: Point2,
  metric: PlaneScreenMetric,
): SnapCandidate[] {
  const frame = ctx.frame;
  if (!frame) return [];
  const measured = frame.measure(raw);
  const out: SnapCandidate[] = [];
  const fields = Object.keys(measured) as DimFieldId[];
  for (const field of fields) {
    const claim = numericClaim(ctx.toolId, ctx.anchors.length, field, ctx.arcMode);
    if (claim === null) continue;
    const rawValue = measured[field];
    if (rawValue === undefined || !Number.isFinite(rawValue)) continue;
    const locked = ctx.locks[field];
    const alias = aliasOf(field);
    if (locked === undefined) {
      // With one half of an alias pair LOCKED, the other's measured value is a
      // stale second opinion about the same geometry (see `DIM_ALIASES`).
      if (alias !== undefined && alias !== null && ctx.locks[alias] !== undefined) continue;
      // Both halves free: `rebuild` prefers `radius`, so offering `diameter`
      // too would be two candidates for one number.
      if (field === "diameter" && measured.radius !== undefined) continue;
    }
    const domain = DOMAIN_QUANTUM[field];
    if (domain === "none") continue;
    const q = ctx.quantum ? (domain === "angle" ? ctx.quantum.angle : ctx.quantum.length) : 0;
    const isTyped = locked !== undefined;
    if (!isTyped && !(q > 0)) continue;
    const targetValue = isTyped ? locked : roundTo(rawValue, q);
    const single: DimValues = {};
    single[field] = targetValue;
    const previewPoint = frame.rebuild(single, raw);
    if (!Number.isFinite(previewPoint.x) || !Number.isFinite(previewPoint.y)) continue;
    const errorPx = metricNorm(metric, previewPoint.x - raw.x, previewPoint.y - raw.y);
    if (!Number.isFinite(errorPx)) continue;
    out.push({
      id: `numeric:${field}:${isTyped ? "typed" : canonicalQuantum(q)}`,
      source: "numeric",
      kind: "numeric",
      projection: {
        kind: "numericField",
        field,
        rawValue,
        targetValue,
        quantum: isTyped ? 0 : q,
        authority: isTyped ? "typed" : "cursor",
      },
      previewPoint,
      claims: [claim],
      errorPx,
      semanticBiasPx: SEMANTIC_BIAS_PX.numeric,
      scorePx: errorPx + SEMANTIC_BIAS_PX.numeric,
      label: "",
      guides: [],
      refs: [],
      // Cursor rounding is a placement convenience, never a design assertion:
      // it persists nothing (SNAP §12).
      relationIntents: [],
    });
  }
  out.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return out;
}

const canonicalQuantum = (q: number): string => String(Math.round(q * 1e9) / 1e9);

// ── Resolution ───────────────────────────────────────────────────────────────

export interface ArbitrationInput {
  raw: Point2;
  metric: PlaneScreenMetric;
  /** Spatial candidates from `generateCandidates`. */
  candidates: readonly SnapCandidate[];
  /** Numeric candidates from `numericCandidates`. */
  numeric: readonly SnapCandidate[];
  acquirePx: number;
  releasePx: number;
  gridReachPx: number;
  latch: SnapLatch;
  frame: DimFrame | null;
  radial: { anchor: Point2; field: DimFieldId } | null;
  traceId: number;
}

export interface ArbitrationOutput {
  decision: SnapDecision;
  latch: SnapLatch;
}

interface CandidateSet {
  /** The SPATIAL shape this set was built from (diagnostics). */
  optional: SnapCandidate[];
  /** Every member including the mandatory typed ones. */
  members: SnapCandidate[];
  point: Point2;
  cost: number;
  primary: SnapCandidate | null;
  ids: string[];
}

/** The empty decision — nothing accepted, the raw cursor survives. */
function noSnap(raw: Point2, traceId: number, rejected: RejectedSnapCandidate[]): SnapDecision {
  return {
    raw,
    point: raw,
    accepted: [],
    rejected,
    primaryId: null,
    primaryKind: "none",
    label: null,
    guides: [],
    snapped: false,
    traceId,
  };
}

export function resolveSnap(input: ArbitrationInput): ArbitrationOutput {
  const rejected: RejectedSnapCandidate[] = [];

  // 1. Radius gate. The LATCHED primary keeps a wider (release) reach, which is
  //    the whole hysteresis mechanism: a target already held survives sub-pixel
  //    jitter that would otherwise drop it at the acquire boundary.
  const spatial: SnapCandidate[] = [];
  for (const c of input.candidates) {
    const reach = c.source === "grid" ? input.gridReachPx : input.acquirePx;
    if (c.errorPx <= reach) {
      spatial.push(c);
      continue;
    }
    if (c.id === input.latch.primaryId && c.errorPx <= input.releasePx) {
      spatial.push(c);
      continue;
    }
    rejected.push({
      candidateId: c.id,
      reason: c.id === input.latch.primaryId ? "outside-release-radius" : "outside-acquire-radius",
    });
  }

  // 1b. A retained target that fell out of GENERATION entirely (too far to be
  //     worth generating at all) still owes an explanation — otherwise the trace
  //     shows a latch silently vanishing.
  if (
    input.latch.primaryId !== null &&
    !input.candidates.some((c) => c.id === input.latch.primaryId)
  ) {
    rejected.push({ candidateId: input.latch.primaryId, reason: "outside-release-radius" });
  }

  // 2. Typed locks are MANDATORY: they enter resolution as authoritative fixed
  //    claims and are never rejected.
  const mandatory = input.numeric.filter(
    (c) => c.projection.kind === "numericField" && c.projection.authority === "typed",
  );
  const mandatoryClaims = new Set<SnapClaim>(mandatory.flatMap((c) => [...c.claims]));
  const cursorNumeric = input.numeric.filter((c) => !mandatory.includes(c));

  // 3. Anything conflicting with a typed lock is out before enumeration — a
  //    typed number is not up for negotiation.
  const usableSpatial: SnapCandidate[] = [];
  for (const c of spatial) {
    if (conflictsWithClaims(c, mandatoryClaims)) {
      rejected.push({ candidateId: c.id, reason: "higher-authority" });
      continue;
    }
    usableSpatial.push(c);
  }
  const usableNumeric = cursorNumeric.filter((c) => {
    if (!conflictsWithClaims(c, mandatoryClaims)) return true;
    rejected.push({ candidateId: c.id, reason: "higher-authority" });
    return false;
  });

  /*
   * 4. Enumerate the SPATIAL shapes (SNAP §5.7.1 step 4), then attach the
   *    numeric fields each shape leaves free. The shapes are exactly the ones
   *    §5.7 declares composable — there is no general powerset here, because an
   *    arbitrary combination of claims is not something the tool frames can
   *    solve simultaneously.
   */
  const points = usableSpatial.filter((c) => c.claims.includes("point"));
  const guidesX = usableSpatial.filter((c) => c.projection.kind === "axisX");
  const guidesY = usableSpatial.filter((c) => c.projection.kind === "axisY");
  const rays = usableSpatial.filter((c) => c.projection.kind === "ray");

  const shapes: SnapCandidate[][] = [[]];
  for (const p of points) shapes.push([p]);
  for (const g of guidesX) shapes.push([g]);
  for (const g of guidesY) shapes.push([g]);
  for (const gx of guidesX) for (const gy of guidesY) shapes.push([gx, gy]);
  for (const r of rays) shapes.push([r]);

  const sets: CandidateSet[] = [];
  for (const shape of shapes) {
    const numeric = numericFor(shape, usableNumeric, input);
    const set = buildSet([...shape, ...numeric], shape, mandatory, input, rejected);
    if (set) sets.push(set);
  }

  const valid = sets.filter((s) => s.members.length > 0);
  if (valid.length === 0) {
    return { decision: noSnap(input.raw, input.traceId, rejected), latch: emptyLatch() };
  }

  // 5. Hysteresis + deterministic choice.
  const chosen = chooseSet(valid, input);
  const set = chosen.set;

  for (const c of [...spatial, ...input.numeric]) {
    if (set.members.includes(c)) continue;
    if (rejected.some((r) => r.candidateId === c.id)) continue;
    rejected.push({
      candidateId: c.id,
      reason:
        chosen.retainedBlocked && c.id === chosen.challengerId ? "retained-target" : "claim-conflict",
    });
  }

  const visible = set.members.filter((c) => c.source !== "numeric");
  return {
    decision: {
      raw: input.raw,
      point: set.point,
      accepted: set.members,
      rejected,
      primaryId: set.primary?.id ?? null,
      primaryKind: primaryKindOf(visible, set.primary),
      label: labelOf(visible),
      guides: visible.flatMap((c) => [...c.guides]),
      // A numeric-only decision is NOT a "snap": cursor rounding is invisible
      // placement help, and showing a marker for it would claim the user aimed
      // at something they never saw.
      snapped: visible.length > 0,
      traceId: input.traceId,
    },
    latch: chosen.latch,
  };
}

/**
 * The numeric fields a spatial shape leaves free — the §5.7 composition table,
 * expressed as data rather than as a rule scattered over the resolver.
 *
 * A full point claims everything, so it takes none. Two guides pin both plane
 * coordinates, so they take none either. One guide takes the field that claims
 * the OTHER axis (a rectangle's height against an x guide) or, failing that,
 * the phase's RADIAL field — the analytic case. A polar ray takes the radial
 * field, which is the "45° plus a rounded length" composition the legacy engine
 * could not express at all.
 */
function numericFor(
  shape: readonly SnapCandidate[],
  numeric: readonly SnapCandidate[],
  input: ArbitrationInput,
): SnapCandidate[] {
  if (numeric.length === 0) return [];
  if (shape.some((c) => c.claims.includes("point"))) return [];
  if (shape.length === 0) return [...numeric]; // the whole bundle: today's rounding
  const claimed = new Set<SnapClaim>(shape.flatMap((c) => [...c.claims]));
  const free = numeric.filter((c) => !c.claims.some((x) => claimed.has(x)));
  if (free.length === 0) return [];
  const hasGuide = shape.some((c) => c.source === "guide");
  const hasRay = shape.some((c) => c.source === "polar");
  if (hasGuide && shape.length === 2) return []; // x + y already pin the point
  const radialField = input.radial?.field;
  const radial = radialField ? free.find((c) => fieldOf(c) === radialField) : undefined;
  if (hasGuide) {
    // Prefer the plain OTHER-AXIS field (a box's height) — it composes by
    // ordinary projection. Only fall back to the analytic radial pair.
    const otherAxis = free.find((c) => c.claims.includes("x") || c.claims.includes("y"));
    if (otherAxis) return [otherAxis];
    return radial ? [radial] : [];
  }
  if (hasRay) return radial ? [radial] : [];
  return [];
}

const fieldOf = (c: SnapCandidate): DimFieldId | null =>
  c.projection.kind === "numericField" ? c.projection.field : null;

/**
 * Two guides through the SAME reference point are a genuine 2D coincidence —
 * "Aligned" — while two guides from unrelated points merely happen to each land
 * on one axis, and calling that "Aligned" would claim a relationship that does
 * not exist. The composed label names both arms in that case.
 */
function primaryKindOf(visible: readonly SnapCandidate[], primary: SnapCandidate | null): SnapKind {
  if (isSameRefGuidePair(visible)) return "alignHV";
  return (primary?.kind ?? "none") as SnapKind;
}

function labelOf(visible: readonly SnapCandidate[]): string | null {
  if (isSameRefGuidePair(visible)) return "Aligned";
  const labels = visible.map((c) => c.label).filter((l) => l.length > 0);
  return labels.length > 0 ? labels.join(" · ") : null;
}

function isSameRefGuidePair(visible: readonly SnapCandidate[]): boolean {
  if (visible.length !== 2) return false;
  const [a, b] = visible;
  if (a.source !== "guide" || b.source !== "guide") return false;
  const ga = a.guides[0];
  const gb = b.guides[0];
  if (!ga || !gb || ga.orientation === "polar" || gb.orientation === "polar") return false;
  return Math.abs(ga.ref.x - gb.ref.x) < 1e-9 && Math.abs(ga.ref.y - gb.ref.y) < 1e-9;
}

function conflictsWithClaims(c: SnapCandidate, claims: ReadonlySet<SnapClaim>): boolean {
  if (claims.size === 0) return false;
  // A full-point candidate claims EVERYTHING, so any other claim conflicts.
  if (c.claims.includes("point")) return true;
  if (claims.has("point")) return true;
  return c.claims.some((x) => claims.has(x));
}

/**
 * Compose a member list into one point and cost it, or null when the set is
 * unsolvable.
 *
 * ── The unresolved-axis charge ───────────────────────────────────────────────
 * A candidate that fixes ONE degree of freedom moves the cursor less than one
 * that fixes both, simply because it constrains less — so ranking on
 * displacement alone systematically prefers the weaker answer. An x guide with
 * a zero axis miss would beat every endpoint in the sketch, forever, whenever
 * anything happened to share an x.
 *
 * So a set is charged the full acquire reach for each plane degree of freedom
 * it leaves unresolved. That is not a tier: a guide can still win outright when
 * nothing resolves both axes, and a guide PAIR (or a polar ray composed with a
 * rounded length) resolves both and pays nothing.
 */
function buildSet(
  members: SnapCandidate[],
  shape: readonly SnapCandidate[],
  mandatory: readonly SnapCandidate[],
  input: ArbitrationInput,
  rejected: RejectedSnapCandidate[],
): CandidateSet | null {
  const all = [...mandatory, ...members];
  if (all.length === 0) return null;
  const point = composePoint(all, input);
  if (!point) {
    for (const c of members) {
      if (c.source === "numeric" && !rejected.some((r) => r.candidateId === c.id)) {
        rejected.push({ candidateId: c.id, reason: "numeric-incompatible" });
      }
    }
    return null;
  }
  if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) {
    for (const c of members) rejected.push({ candidateId: c.id, reason: "invalid-projection" });
    return null;
  }
  // The rebuilt point must not run away from the reaches its members promised.
  // A typed lock is exempt: it is exact and authoritative, and may legitimately
  // place the point anywhere.
  if (mandatory.length === 0) {
    const allowed =
      Math.max(
        ...all.map((c) =>
          c.source === "grid"
            ? input.gridReachPx
            : c.source === "numeric"
              ? c.errorPx
              : // A LATCHED member promised the wider release reach — charging it
                // the acquire reach here would throw away the retention the
                // radius gate just granted.
                c.id === input.latch.primaryId
                ? input.releasePx
                : input.acquirePx,
        ),
      ) + SET_DISPLACEMENT_SLACK_PX;
    const moved = metricNorm(input.metric, point.x - input.raw.x, point.y - input.raw.y);
    if (moved > allowed) return null;
  }
  const unresolved = Math.max(0, 2 - resolvedDof(all));
  const cost =
    all.reduce((s, c) => s + c.scorePx, 0) +
    Math.max(0, members.length - 1) * SET_COMPLEXITY_PENALTY_PX +
    unresolved * input.acquirePx;
  const visible = all.filter((c) => c.source !== "numeric");
  const primary =
    visible.length > 0 ? visible.reduce((best, c) => (c.scorePx < best.scorePx ? c : best)) : null;
  return {
    optional: [...shape],
    members: all,
    point,
    cost,
    primary,
    ids: all.map((c) => c.id).sort(),
  };
}

/** How many of the plane's two degrees of freedom a set actually pins. */
function resolvedDof(members: readonly SnapCandidate[]): number {
  const claims = new Set<SnapClaim>();
  for (const c of members) for (const k of c.claims) claims.add(k);
  if (claims.has("point")) return 2;
  return Math.min(2, claims.size);
}

/**
 * Apply the members' projections in CANONICAL claim order — point, x, y, ray,
 * then the numeric fields through the live-dimension frame.
 *
 * The order is what makes composition well-defined: a ray is projected from the
 * axis-constrained point, and the numeric rebuild sees the direction the
 * geometry members already established.
 */
function composePoint(members: readonly SnapCandidate[], input: ArbitrationInput): Point2 | null {
  const pointProj = members.find((c) => c.projection.kind === "point");
  const numerics = members.filter((c) => c.projection.kind === "numericField");
  if (pointProj && pointProj.projection.kind === "point") {
    // A full point claims everything; nothing else may be in the set.
    return numerics.length > 0 ? null : { ...pointProj.projection.point };
  }

  const axisX = members.find((c) => c.projection.kind === "axisX");
  const axisY = members.find((c) => c.projection.kind === "axisY");
  const ray = members.find((c) => c.projection.kind === "ray");

  // ANALYTIC: one axis guide plus the phase's radial field. Neither can be
  // applied "after" the other — they are simultaneous equations.
  if ((axisX || axisY) && numerics.length > 0 && input.radial) {
    return composeGuideWithRadial(axisX, axisY, numerics, input);
  }

  let p: Point2 = { ...input.raw };
  if (axisX && axisX.projection.kind === "axisX") p.x = axisX.projection.x;
  if (axisY && axisY.projection.kind === "axisY") p.y = axisY.projection.y;
  if (ray && ray.projection.kind === "ray") {
    const A = metricTensor(input.metric);
    const onRay = nearestOnRayMetric(A, p, ray.projection.origin, ray.projection.dir, false);
    if (!onRay) return null;
    p = onRay;
  }
  if (numerics.length === 0) return p;
  const frame = input.frame;
  if (!frame) return null;
  const values = frame.measure(p);
  for (const c of numerics) {
    if (c.projection.kind !== "numericField") continue;
    const { field, authority, quantum } = c.projection;
    if (authority === "typed") {
      values[field] = c.projection.targetValue;
      continue;
    }
    // Re-round at the COMPOSED point, not at the raw cursor: after a polar ray
    // slid the cursor along the direction, the length to round is the length
    // along that ray.
    const measured = values[field];
    if (measured === undefined) return null;
    values[field] = roundTo(measured, quantum);
  }
  dropAliasedSiblings(values, members);
  return frame.rebuild(values, p);
}

/**
 * A remote x/y guide plus a rounded radial value: intersect the circle centred
 * on the gesture anchor at the rounded radius with the guide's line.
 *
 * No real intersection means the two statements are simply incompatible — the
 * guide (higher authority, it names a real point in the drawing) wins and the
 * numeric is rejected, rather than the point being dragged to the nearest
 * almost-solution.
 */
function composeGuideWithRadial(
  axisX: SnapCandidate | undefined,
  axisY: SnapCandidate | undefined,
  numerics: readonly SnapCandidate[],
  input: ArbitrationInput,
): Point2 | null {
  const radial = input.radial;
  if (!radial) return null;
  const numeric = numerics.find(
    (c) => c.projection.kind === "numericField" && c.projection.field === radial.field,
  );
  if (!numeric || numeric.projection.kind !== "numericField") return null;
  const r = Math.abs(numeric.projection.targetValue);
  const c = radial.anchor;
  const candidates: Point2[] = [];
  if (axisX && axisX.projection.kind === "axisX") {
    const dx = axisX.projection.x - c.x;
    const disc = r * r - dx * dx;
    if (disc < 0) return null;
    const dy = Math.sqrt(disc);
    candidates.push({ x: axisX.projection.x, y: c.y + dy }, { x: axisX.projection.x, y: c.y - dy });
  } else if (axisY && axisY.projection.kind === "axisY") {
    const dy = axisY.projection.y - c.y;
    const disc = r * r - dy * dy;
    if (disc < 0) return null;
    const dx = Math.sqrt(disc);
    candidates.push({ x: c.x + dx, y: axisY.projection.y }, { x: c.x - dx, y: axisY.projection.y });
  } else {
    return null;
  }
  let best: Point2 | null = null;
  let bestD = Infinity;
  for (const p of candidates) {
    const d = metricNorm(input.metric, p.x - input.raw.x, p.y - input.raw.y);
    if (d < bestD) {
      bestD = d;
      best = p;
    }
  }
  return best;
}

/** With exactly one half of an alias pair accepted, drop the other's measured
 *  value so `rebuild` cannot pick the wrong one (see `DIM_ALIASES`). */
function dropAliasedSiblings(values: DimValues, members: readonly SnapCandidate[]): void {
  for (const c of members) {
    if (c.projection.kind !== "numericField") continue;
    const alias = aliasOf(c.projection.field);
    if (alias) delete values[alias];
  }
}

/**
 * Pick the winning set, applying hysteresis.
 *
 * The retained target holds unless a challenger is at least
 * {@link HYSTERESIS_ADVANTAGE_PX} cheaper AND has been for
 * {@link CHALLENGER_FRAMES} consecutive samples. A click uses the SAME rule: a
 * one-frame challenger must not win just because the user happened to press the
 * button, or the commit would land somewhere the marker never showed.
 */
function chooseSet(
  sets: readonly CandidateSet[],
  input: ArbitrationInput,
): { set: CandidateSet; latch: SnapLatch; retainedBlocked: boolean; challengerId: string | null } {
  const byCost = [...sets].sort((a, b) => compareSets(a, b, input.latch));
  const best = byCost[0];
  const retainedId = input.latch.primaryId;
  const retained = retainedId ? byCost.find((s) => s.primary?.id === retainedId) : undefined;

  if (!retained) {
    return {
      set: best,
      latch: latchFor(best),
      retainedBlocked: false,
      challengerId: null,
    };
  }
  const challenger = byCost.find((s) => s.primary?.id !== retainedId);
  const challengerId = challenger?.primary?.id ?? null;
  if (!challenger || challenger.cost > retained.cost - HYSTERESIS_ADVANTAGE_PX) {
    // Nothing beats the held target by enough — keep it, and forget any
    // half-formed challenge (the streak must be CONSECUTIVE).
    return { set: retained, latch: latchFor(retained), retainedBlocked: false, challengerId: null };
  }
  const frames =
    input.latch.challengerId === challengerId ? input.latch.challengerFrames + 1 : 1;
  if (frames >= CHALLENGER_FRAMES) {
    return { set: challenger, latch: latchFor(challenger), retainedBlocked: false, challengerId };
  }
  return {
    set: retained,
    latch: { ...latchFor(retained), challengerId, challengerFrames: frames },
    retainedBlocked: true,
    challengerId,
  };
}

function latchFor(set: CandidateSet): SnapLatch {
  return {
    primaryId: set.primary?.id ?? null,
    acceptedIds: set.ids,
    challengerId: null,
    challengerFrames: 0,
  };
}

/**
 * Total order over sets: cost, then the RETAINED set, then the set that NAMES
 * more, then the lexicographic accepted-id list. Every tie-break is a function
 * of the candidates themselves, so no enumeration order can leak into the
 * answer.
 *
 * The "names more" rung matters in practice: a 45° polar ray and a cursor angle
 * rounded to 45° put the point in the same place and therefore cost the same,
 * but only the ray draws a guide the user can see and only the ray carries a
 * relation intent worth persisting. Cursor rounding persists nothing by design.
 */
function compareSets(a: CandidateSet, b: CandidateSet, latch: SnapLatch): number {
  if (Math.abs(a.cost - b.cost) > 1e-9) return a.cost - b.cost;
  const aRetained = a.primary?.id === latch.primaryId ? 0 : 1;
  const bRetained = b.primary?.id === latch.primaryId ? 0 : 1;
  if (aRetained !== bRetained) return aRetained - bRetained;
  const aNamed = a.members.filter((c) => c.source !== "numeric").length;
  const bNamed = b.members.filter((c) => c.source !== "numeric").length;
  if (aNamed !== bNamed) return bNamed - aNamed;
  const ak = a.ids.join("|");
  const bk = b.ids.join("|");
  return ak < bk ? -1 : ak > bk ? 1 : 0;
}
