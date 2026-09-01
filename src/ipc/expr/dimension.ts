/**
 * Canonical units and the dimension algebra — mirrors
 * `src-tauri/crates/onecad-core/src/expr/dimension.rs`.
 *
 * Every `Value` carries its number in the CANONICAL unit for its dimension:
 * millimetres for `"length"`, degrees for `"angle"`, unitless for
 * `"scalar"`. A literal unit suffix (`mm cm m in deg rad`) converts to
 * canonical the moment the parser folds the literal (`literalValue`,
 * context-free — units need no environment); every operator below only
 * ever sees canonical numbers, so this algebra applies no conversion
 * factor of its own — only dimension bookkeeping. That is also why
 * Scalar-into-site coercion (`eval.ts`) is a pass-through: the number is
 * already in whatever unit the call site means.
 */
import { DEGREES_PER_RADIAN } from "../angleUnits";
import { ExprError } from "./errors";

/** The dimension a numeric `Value` is measured in. */
export type Dimension = "length" | "angle" | "scalar";

/** An evaluated number tagged with its dimension, always in canonical units
 * (mm / deg / unitless). */
export interface Value {
  number: number;
  dim: Dimension;
}

/** Human-facing label for error messages. */
export function label(d: Dimension): string {
  return d;
}

/** A recognized literal unit suffix. */
export type Unit = "mm" | "cm" | "m" | "in" | "deg" | "rad";

/** Matches an adjacent alpha run against a WHOLE unit keyword — no prefix
 * matching, so `1radius` is a lex error rather than a silent `1rad` + stray
 * `ius` identifier. */
export function unitFromSuffix(s: string): Unit | undefined {
  switch (s) {
    case "mm":
    case "cm":
    case "m":
    case "in":
    case "deg":
    case "rad":
      return s;
    default:
      return undefined;
  }
}

/** `(canonical-unit factor, dimension)`. */
function unitCanonical(u: Unit): { factor: number; dim: Dimension } {
  switch (u) {
    case "mm":
      return { factor: 1, dim: "length" };
    case "cm":
      return { factor: 10, dim: "length" };
    case "m":
      return { factor: 1000, dim: "length" };
    case "in":
      return { factor: 25.4, dim: "length" };
    case "deg":
      return { factor: 1, dim: "angle" };
    case "rad":
      return { factor: DEGREES_PER_RADIAN, dim: "angle" };
  }
}

/** Folds a literal's raw text value + optional unit suffix into its
 * canonical `Value` — context-free, so this runs at parse time. */
export function literalValue(value: number, unit: Unit | undefined): Value {
  if (unit === undefined) return { number: value, dim: "scalar" };
  const { factor, dim } = unitCanonical(unit);
  return { number: value * factor, dim };
}

function mismatch(a: Dimension, b: Dimension, span: [number, number], op: string): ExprError {
  return new ExprError("DimensionMismatch", span, `\`${op}\` cannot combine ${label(a)} and ${label(b)}`);
}

/** `+`, `-`, `min`, `max`: equal dims OK; either side `scalar` coerces to
 * the other; otherwise a loud mismatch. Result is the non-`scalar` dim (or
 * `scalar` if both are). */
export function addLike(a: Dimension, b: Dimension, span: [number, number], op: string): Dimension {
  if (a === b) return a;
  if (a === "scalar") return b;
  if (b === "scalar") return a;
  throw mismatch(a, b, span, op);
}

/** `*`: at most one operand may be non-`scalar` — no `mm^2`, no `deg*mm`
 * (and no `mm*mm` either: two equal non-scalar dims still mismatch). */
export function mul(a: Dimension, b: Dimension, span: [number, number]): Dimension {
  if (a === "scalar" && b === "scalar") return "scalar";
  if (a === "scalar") return b;
  if (b === "scalar") return a;
  throw mismatch(a, b, span, "*");
}

/** `/`: dividing by a `scalar` keeps the numerator's dim; dividing two
 * equal dims cancels to `scalar`; anything else (including a `scalar`
 * numerator over a dimensioned denominator) is a mismatch. */
export function div(a: Dimension, b: Dimension, span: [number, number]): Dimension {
  if (b === "scalar") return a;
  if (a === b) return "scalar";
  throw mismatch(a, b, span, "/");
}

/** `^`: both operands must be `scalar`. */
export function pow(a: Dimension, b: Dimension, span: [number, number]): Dimension {
  if (a === "scalar" && b === "scalar") return "scalar";
  throw mismatch(a, b, span, "^");
}

/** `sin|cos|tan` argument check: must be `angle`. A bare `scalar` gets a
 * specific nudge toward the fix; any other mismatch is generic. */
export function requireAngle(d: Dimension, span: [number, number]): void {
  if (d === "angle") return;
  if (d === "scalar") {
    throw new ExprError("DimensionMismatch", span, "trig needs an angle — write 30deg or 0.5rad");
  }
  throw mismatch(d, "angle", span, "trig");
}

/** `asin|acos|atan|sqrt` argument check: must be `scalar`. */
export function requireScalar(d: Dimension, span: [number, number], fnName: string): void {
  if (d === "scalar") return;
  throw mismatch(d, "scalar", span, fnName);
}
