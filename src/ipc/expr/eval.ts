/**
 * Evaluator — walks an `Expr` tree against an `Env` and a call-site
 * `Dimension`, applying the dimension algebra in `dimension.ts`. Mirrors
 * `src-tauri/crates/onecad-core/src/expr/eval.rs`.
 *
 * Every recursive step returns through `finite`, so a `NaN`/`±Infinity` is
 * caught at the exact node that produced it (tight error span) rather than
 * discovered later after it has propagated through more arithmetic.
 */
import { degToRad, radToDeg } from "../angleUnits";
import * as dim from "./dimension";
import type { Dimension, Value } from "./dimension";
import { ExprError } from "./errors";
import { parse } from "./parser";
import type { BinaryOp, Expr } from "./parser";

/** Variable resolution is the CALLER's job — this engine only evaluates
 * against whatever `env` returns. */
export type Env = (name: string) => Value | undefined;

/**
 * Evaluates `expr` against `env`, coercing the result into `site`'s
 * canonical unit. A `"scalar"` result coerces into any site verbatim (no
 * rescale — the number is read as already being in the site's canonical
 * unit); a mismatched non-`"scalar"` dimension is a loud
 * `"DimensionMismatch"` naming both dimensions.
 */
export function evaluate(expr: Expr, env: Env, site: Dimension): number {
  const value = evalNode(expr, env);
  if (value.dim === site || value.dim === "scalar") return value.number;
  throw new ExprError(
    "DimensionMismatch",
    expr.span,
    `expected ${dim.label(site)} (or a dimensionless number), found ${dim.label(value.dim)}`,
  );
}

/**
 * Evaluates `expr` with NO call site, keeping the result's own dimension —
 * mirrors `evaluate_value` in `onecad-core`'s `expr::eval`.
 *
 * The entry point for a caller that needs the DIMENSION as well as the number:
 * a variable's own value (whose dimension is inferred from its expression) and
 * an authoring preview that reports what it resolved to. Otherwise identical to
 * {@link evaluate}, minus the site-boundary mismatch it cannot raise.
 */
export function evaluateValue(expr: Expr, env: Env): Value {
  return evalNode(expr, env);
}

/** Parses then evaluates `src` in one call. Parse failures (`"Parse"`,
 * `"TooLong"`, `"TooDeep"`) surface the same way as evaluation failures. */
export function evaluateStr(src: string, env: Env, site: Dimension): number {
  return evaluate(parse(src), env, site);
}

function finite(number: number, d: Dimension, span: [number, number]): Value {
  if (Number.isFinite(number)) return { number, dim: d };
  throw new ExprError("NonFinite", span, "result is not finite");
}

function evalNode(expr: Expr, env: Env): Value {
  switch (expr.type) {
    case "number":
      return finite(expr.value.number, expr.value.dim, expr.span);
    case "ident": {
      const v = env(expr.name);
      if (v === undefined) {
        throw new ExprError("UndefinedVariable", expr.span, `undefined variable \`${expr.name}\``);
      }
      return finite(v.number, v.dim, expr.span);
    }
    case "unary": {
      const v = evalNode(expr.expr, env);
      const number = expr.op === "plus" ? v.number : -v.number;
      return finite(number, v.dim, expr.span);
    }
    case "binary":
      return evalBinary(expr.op, expr.lhs, expr.rhs, env, expr.span);
    case "call":
      return evalCall(expr.name, expr.args, env, expr.span);
  }
}

function evalBinary(op: BinaryOp, lhs: Expr, rhs: Expr, env: Env, span: [number, number]): Value {
  const a = evalNode(lhs, env);
  const b = evalNode(rhs, env);
  switch (op) {
    case "add":
      return finite(a.number + b.number, dim.addLike(a.dim, b.dim, span, "+"), span);
    case "sub":
      return finite(a.number - b.number, dim.addLike(a.dim, b.dim, span, "-"), span);
    case "mul":
      return finite(a.number * b.number, dim.mul(a.dim, b.dim, span), span);
    case "div": {
      if (b.number === 0) {
        throw new ExprError("DivideByZero", span, "division by zero");
      }
      return finite(a.number / b.number, dim.div(a.dim, b.dim, span), span);
    }
    case "pow":
      return finite(Math.pow(a.number, b.number), dim.pow(a.dim, b.dim, span), span);
  }
}

/** The fixed builtin function set — no addon-defined expression function. */
function evalCall(name: string, args: Expr[], env: Env, span: [number, number]): Value {
  const vals = args.map((a) => evalNode(a, env));
  switch (name) {
    case "sin":
    case "cos":
    case "tan":
      return evalTrig(name, vals, span);
    case "asin":
    case "acos":
    case "atan":
      return evalInverseTrig(name, vals, span);
    case "sqrt":
      return evalSqrt(vals, span);
    case "abs":
    case "floor":
    case "ceil":
    case "round":
      return evalDimPreserving(name, vals, span);
    case "min":
    case "max":
      return evalMinMax(name, vals, span);
    default:
      throw new ExprError("UnknownFunction", span, `unknown function \`${name}\``);
  }
}

function checkArity(vals: Value[], want: number, name: string, span: [number, number]): void {
  if (vals.length !== want) {
    throw new ExprError("Arity", span, `\`${name}\` takes ${want} argument(s), got ${vals.length}`);
  }
}

/** `sin|cos|tan`: argument must be `"angle"`, result is `"scalar"`. */
function evalTrig(name: "sin" | "cos" | "tan", vals: Value[], span: [number, number]): Value {
  checkArity(vals, 1, name, span);
  dim.requireAngle(vals[0].dim, span);
  const rad = degToRad(vals[0].number);
  const n = name === "sin" ? Math.sin(rad) : name === "cos" ? Math.cos(rad) : Math.tan(rad);
  return finite(n, "scalar", span);
}

/** `asin|acos|atan`: argument must be `"scalar"`, result is `"angle"`
 * (degrees). */
function evalInverseTrig(name: "asin" | "acos" | "atan", vals: Value[], span: [number, number]): Value {
  checkArity(vals, 1, name, span);
  dim.requireScalar(vals[0].dim, span, name);
  const rad = name === "asin" ? Math.asin(vals[0].number) : name === "acos" ? Math.acos(vals[0].number) : Math.atan(vals[0].number);
  return finite(radToDeg(rad), "angle", span);
}

/** `sqrt`: argument must be `"scalar"`, result is `"scalar"`. */
function evalSqrt(vals: Value[], span: [number, number]): Value {
  checkArity(vals, 1, "sqrt", span);
  dim.requireScalar(vals[0].dim, span, "sqrt");
  return finite(Math.sqrt(vals[0].number), "scalar", span);
}

/** Rust's `f64::round`: nearest integer, ties round AWAY FROM ZERO — unlike
 * JS's `Math.round`, which rounds ties toward +Infinity (`Math.round(-2.5)
 * === -2`, but Rust's `(-2.5_f64).round() == -3.0`). */
function roundHalfAwayFromZero(x: number): number {
  return x < 0 ? -Math.round(-x) : Math.round(x);
}

/** `abs|floor|ceil|round`: any dimension in, same dimension out — these
 * operate on the canonical number only. */
function evalDimPreserving(name: "abs" | "floor" | "ceil" | "round", vals: Value[], span: [number, number]): Value {
  checkArity(vals, 1, name, span);
  const n =
    name === "abs"
      ? Math.abs(vals[0].number)
      : name === "floor"
        ? Math.floor(vals[0].number)
        : name === "ceil"
          ? Math.ceil(vals[0].number)
          : roundHalfAwayFromZero(vals[0].number);
  return finite(n, vals[0].dim, span);
}

/** `min|max`: same dimension algebra as `+`/`-` (`"scalar"` coerces). */
function evalMinMax(name: "min" | "max", vals: Value[], span: [number, number]): Value {
  checkArity(vals, 2, name, span);
  const d = dim.addLike(vals[0].dim, vals[1].dim, span, name);
  const n = name === "min" ? Math.min(vals[0].number, vals[1].number) : Math.max(vals[0].number, vals[1].number);
  return finite(n, d, span);
}
