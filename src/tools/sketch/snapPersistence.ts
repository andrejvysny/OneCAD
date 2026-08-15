/*
 * snapPersistence — accepted snap INTENT → authored constraints (SNAP P3).
 *
 * The rule the whole intent pipeline exists to enforce: **only an accepted
 * relation persists.** Coordinate coincidence alone is not evidence — two
 * points landing on the same spot could be a deliberate weld or an accident,
 * and the difference is whether the snap engine ACCEPTED a candidate that said
 * so at click time.
 *
 * ── What is NOT here ─────────────────────────────────────────────────────────
 * Structural tool topology (a rectangle's four corner welds, a chain's
 * segment-to-segment join) is authored by the TOOL and never routed through
 * this module. It is not an inference and not a preference: without it the
 * committed shape mechanically falls apart, so it survives Alt and survives
 * auto-constrain mode `off`.
 */
import type { ConstraintPosition, SketchConstraint, SketchConstraintType } from "@/ipc/types";
import type { AutoConstrainMode } from "@/stores/settingsStore";
import type { FrontendPointRef, SnapDecision, SnapRelationIntent } from "./snapTypes";

/** One placement click, joined to the committed point it became. */
export interface PlacedIntent {
  decision: SnapDecision;
  /** Alt as captured in that click's own synchronous stack. */
  suppressInference: boolean;
  /** The committed point this click produced, or null when placement-only. */
  target: FrontendPointRef | null;
}

export interface PersistOptions {
  mode: AutoConstrainMode;
  nextConstraintId: () => string;
  /** Constraints already authored for this batch (tool-structural + prior
   *  intents) — the dedupe baseline. */
  existing: readonly SketchConstraint[];
  /** True when the sketch already carries an origin anchor. */
  originAnchored: boolean;
}

export interface PersistResult {
  constraints: SketchConstraint[];
  /** Intents that were NOT persisted, with the reason — trace fodder. */
  dropped: Array<{ kind: SnapRelationIntent["kind"]; reason: DropReason }>;
}

export type DropReason =
  | "alt-suppressed"
  | "mode-off"
  | "mode-minimal"
  | "placement-only"
  | "origin-already-anchored"
  | "duplicate"
  | "missing-ref";

/** Relations `minimal` allows: the ones that restate where the user clicked. */
const MINIMAL_KINDS: ReadonlySet<SnapRelationIntent["kind"]> = new Set([
  "Coincident",
  "Midpoint",
  "FixedOrigin",
  "OnCurve",
]);

/**
 * Author the constraints the accepted intents earn.
 *
 * Order is placement order, so a batch that snaps twice to the origin anchors
 * the FIRST one and drops the second — deterministically, not by array luck.
 */
export function persistAcceptedIntents(
  placed: readonly PlacedIntent[],
  opts: PersistOptions,
): PersistResult {
  const out: SketchConstraint[] = [];
  const dropped: PersistResult["dropped"] = [];
  const seen = new Set<string>(opts.existing.map(canonicalKey));
  let originAnchored = opts.originAnchored;

  for (const p of placed) {
    for (const intent of p.decision.accepted.flatMap((c) => [...c.relationIntents])) {
      const drop = (reason: DropReason): void => {
        dropped.push({ kind: intent.kind, reason });
      };
      // Alt is the user saying "place this freely". It suppresses INFERENCE
      // under every mode, and never touches structural topology (which is not
      // routed through here at all).
      if (p.suppressInference) {
        drop("alt-suppressed");
        continue;
      }
      if (opts.mode === "off") {
        drop("mode-off");
        continue;
      }
      if (opts.mode === "minimal" && !MINIMAL_KINDS.has(intent.kind)) {
        drop("mode-minimal");
        continue;
      }
      if (intent.kind === "FixedOrigin") {
        if (originAnchored) {
          drop("origin-already-anchored");
          continue;
        }
        if (!p.target) {
          // The origin snap landed on a placement-only click (a circle's radius
          // click, say). There is no point to anchor.
          drop("placement-only");
          continue;
        }
        const c = fixedAt(p.target, opts.nextConstraintId());
        if (pushUnique(out, seen, c)) originAnchored = true;
        else drop("duplicate");
        continue;
      }
      const built = buildConstraint(intent, p.target, opts.nextConstraintId);
      if (!built) {
        drop(p.target ? "missing-ref" : "placement-only");
        continue;
      }
      if (!pushUnique(out, seen, built)) drop("duplicate");
    }
  }
  return { constraints: out, dropped };
}

function fixedAt(target: FrontendPointRef, id: string): SketchConstraint {
  return { id, type: "Fixed", entities: [target.entityId], positions: [target.position] };
}

function buildConstraint(
  intent: SnapRelationIntent,
  target: FrontendPointRef | null,
  nextId: () => string,
): SketchConstraint | null {
  switch (intent.kind) {
    case "Coincident": {
      const other = intent.refs[0];
      if (!target || !other) return null;
      return {
        id: nextId(),
        type: "Coincident",
        entities: [target.entityId, other.entityId],
        positions: [target.position, other.position],
      };
    }
    case "Midpoint": {
      const line = intent.lineIds?.[0];
      if (!target || !line) return null;
      return {
        id: nextId(),
        type: "Midpoint",
        entities: [target.entityId, line],
        positions: [target.position],
      };
    }
    case "OnCurve": {
      // An intersection names TWO curves; the point lies on both, but only the
      // pair together pins it. Authoring one is a weaker (and different)
      // statement than the user made, so both are emitted.
      const curves = intent.curveIds ?? [];
      if (!target || curves.length === 0) return null;
      // One constraint per curve — the caller dedupes, and a second identical
      // OnCurve against the same curve cannot arise from one intent.
      return onCurveFor(target, curves[0], nextId());
    }
    case "HorizontalPoints":
    case "VerticalPoints": {
      const other = intent.refs[0];
      if (!target || !other) return null;
      return {
        id: nextId(),
        type: intent.kind,
        entities: [target.entityId, other.entityId],
        positions: [target.position, other.position],
      };
    }
    case "Parallel":
    case "Perpendicular": {
      const line = intent.lineIds?.[0];
      if (!target || !line) return null;
      // The relation is between the ENTITY the click just authored and the
      // reference line — the target names that entity.
      return { id: nextId(), type: intent.kind, entities: [target.entityId, line] };
    }
    case "Tangent": {
      const other = intent.curveIds?.[0] ?? intent.lineIds?.[0];
      if (!target || !other) return null;
      return { id: nextId(), type: "Tangent", entities: [target.entityId, other] };
    }
    default:
      return null;
  }
}

function onCurveFor(point: FrontendPointRef, curve: string, id: string): SketchConstraint {
  return {
    id,
    type: "OnCurve",
    entities: [point.entityId, curve],
    positions: [point.position],
  };
}

/**
 * The identity of a constraint for DEDUPE purposes: kind plus its refs, with
 * point-pair kinds order-normalized (a Coincident is symmetric, so `a↔b` and
 * `b↔a` are the same relation and must not both reach the solver as redundant
 * equations).
 */
export function canonicalKey(c: {
  type: SketchConstraintType;
  entities: readonly string[];
  positions?: readonly (ConstraintPosition | undefined)[];
}): string {
  const slots = c.entities.map((e, i) => `${e}.${c.positions?.[i] ?? ""}`);
  const symmetric =
    c.type === "Coincident" ||
    c.type === "HorizontalPoints" ||
    c.type === "VerticalPoints" ||
    c.type === "Parallel" ||
    c.type === "Perpendicular" ||
    c.type === "Concentric" ||
    c.type === "Equal" ||
    c.type === "Tangent";
  return `${c.type}|${(symmetric ? [...slots].sort() : slots).join("|")}`;
}

function pushUnique(
  out: SketchConstraint[],
  seen: Set<string>,
  c: SketchConstraint,
): boolean {
  const key = canonicalKey(c);
  if (seen.has(key)) return false;
  seen.add(key);
  out.push(c);
  return true;
}
