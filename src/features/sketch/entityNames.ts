/*
 * Sketch entity display names and constraint row labels (WP-2, S10).
 *
 * The wire carries no label for either (`SketchConstraint` in `ipc/types.ts` is
 * id + type + refs, and adding a field would put a presentation concern on the
 * protocol), so the names are DERIVED here — "Line 2", "Arc 1" by type ordinal
 * in session order. Stable for the life of a session: the ordinal is the
 * entity's position among its own kind in `session.entities`, and that array
 * only ever grows at the end.
 *
 * PURE. No store, no React, no display unit — a dimensional value reaches the
 * label pre-formatted through `opts.value`, because the unit is the caller's
 * live setting (`settingsStore.displayUnit`) and this module must stay
 * renderable from a test with no store at all.
 *
 * An id this session does not know renders as NO name rather than as itself: a
 * raw `e7` in the inspector is the thing S10 exists to remove, and a row that
 * silently loses its operand clause is strictly better than one that leaks an
 * internal id.
 */
import { CONSTRAINT_PRESENTATION } from "@/features/sketch/constraintCatalog";
import type { ConstraintPosition, SketchConstraint, SketchEntity } from "@/ipc/types";

/** Display word per entity kind — the noun the ordinal attaches to. */
const ENTITY_NOUN: Readonly<Record<SketchEntity["type"], string>> = {
  Point: "Point",
  Line: "Line",
  Arc: "Arc",
  Circle: "Circle",
  Ellipse: "Ellipse",
};

/** Lower-case word for the point a positional constraint selects. */
const POSITION_WORD: Readonly<Record<ConstraintPosition, string>> = {
  Start: "start",
  End: "end",
  Center: "center",
  Midpoint: "midpoint",
};

/**
 * "Line 2" for `id`, or `null` when this session holds no such entity.
 *
 * O(n) per call by design: a constraint list is a few dozen rows over a few
 * dozen entities, and a cached map would need an invalidation story the callers
 * (pure renders off the live session array) do not have.
 */
export function entityDisplayName(entities: readonly SketchEntity[], id: string): string | null {
  const target = entities.find((e) => e.id === id);
  if (!target) return null;
  let ordinal = 0;
  for (const e of entities) {
    if (e.type === target.type) ordinal++;
    if (e.id === id) return `${ENTITY_NOUN[target.type]} ${ordinal}`;
  }
  return null;
}

/** The separator between two operands, per relation kind. */
function operandJoin(type: SketchConstraint["type"]): string {
  if (type === "Perpendicular") return " ⟂ ";
  if (type === "Parallel") return " ∥ ";
  return " – ";
}

/**
 * The operand clause: every referenced slot named, with its point word when the
 * constraint selects one. `null` as soon as ANY operand is unknown — a
 * half-named relation ("Line 1 end – "), or one that falls back to the raw id,
 * is worse than no clause at all.
 */
function operandClause(c: SketchConstraint, entities: readonly SketchEntity[]): string | null {
  if (c.entities.length === 0) return null;
  const slots: string[] = [];
  for (let i = 0; i < c.entities.length; i++) {
    const name = entityDisplayName(entities, c.entities[i]);
    if (name === null) return null;
    const position = c.positions?.[i];
    slots.push(position ? `${name} ${POSITION_WORD[position]}` : name);
  }
  return slots.join(operandJoin(c.type));
}

/**
 * One constraint's human row label: "Horizontal · Line 2",
 * "Coincident · Line 1 end – Line 2 start", "Perpendicular · Line 2 ⟂ Line 3".
 *
 * `opts.value` is the already-formatted dimensional value; pass it where the
 * label has to carry the number on its own (the conflicting-constraint
 * sentence), omit it where the surface renders a separate value column
 * (`ConstraintList`).
 */
export function constraintRowLabel(
  constraint: SketchConstraint,
  entities: readonly SketchEntity[],
  opts?: { value?: string | null },
): string {
  const kind = CONSTRAINT_PRESENTATION[constraint.type].label;
  const value = opts?.value;
  const head = value ? `${kind} ${value}` : kind;
  const operands = operandClause(constraint, entities);
  return operands === null ? head : `${head} · ${operands}`;
}

/**
 * "Line 2 is already vertical" — one witness constraint stated as the fact it
 * asserts, for the hint that explains a refused direction candidate (B3).
 * `null` when the operands cannot be named, so the caller can fall back to a
 * hint that claims nothing it cannot show.
 */
function directionFactPhrase(
  constraint: SketchConstraint,
  entities: readonly SketchEntity[],
): string | null {
  const a = entityDisplayName(entities, constraint.entities[0] ?? "");
  if (a === null) return null;
  if (constraint.type === "Horizontal") return `${a} is already horizontal`;
  if (constraint.type === "Vertical") return `${a} is already vertical`;
  const b = entityDisplayName(entities, constraint.entities[1] ?? "");
  if (b === null) return null;
  if (constraint.type === "Parallel") return `${a} is already parallel to ${b}`;
  if (constraint.type === "Perpendicular") return `${a} is already perpendicular to ${b}`;
  return null;
}

/**
 * The status hint for a direction candidate the sketch already contradicts
 * (B3): "Perpendicular skipped — Line 2 is already vertical".
 *
 * `witness` is the existing constraints the parity graph blamed, in path order,
 * each stated as the fact it asserts. When none of them can be named (geometry
 * the session no longer holds) the hint still says what happened without
 * claiming a reason it cannot show.
 */
export function directionSkippedHint(
  candidateType: SketchConstraint["type"],
  witness: readonly SketchConstraint[],
  entities: readonly SketchEntity[],
): string {
  const kind = CONSTRAINT_PRESENTATION[candidateType].label;
  const facts = witness
    .map((c) => directionFactPhrase(c, entities))
    .filter((phrase): phrase is string => phrase !== null);
  return facts.length > 0
    ? `${kind} skipped — ${facts.join(" and ")}`
    : `${kind} skipped — it contradicts the sketch's existing directions`;
}
