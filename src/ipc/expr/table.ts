/**
 * The VARIABLE GRAPH layer over the expression engine — mirrors
 * `src-tauri/crates/onecad-core/src/regen/variables.rs`
 * (`resolve_variable_table` / `evaluate_at_site` / `rename_reference`).
 *
 * `eval.ts` evaluates one expression against one opaque `Env`. Everything that
 * needs a whole TABLE — a variable defined in terms of another, a cycle, a
 * rename that must rewrite every reference — lives here, for the same reason
 * Rust splits it out: the engine has no notion of a variable graph, so it can
 * neither detect a cycle nor own a chain depth.
 *
 * Consumed by `mockClient`, so the mock lane answers `evaluateExpression` /
 * `upsertVariableExpr` / `renameVariable` with the SAME semantics the backend
 * applies rather than a second, drifting approximation. Rust stays the sole
 * authority for anything that reaches geometry.
 */
import type { Dimension, Value } from "./dimension";
import { ExprError } from "./errors";
import type { ExprErrorKind } from "./errors";
import { evaluate, evaluateValue } from "./eval";
import type { Env } from "./eval";
import type { Expr } from "./parser";
import { parse, references } from "./parser";

/** Hard cap on how deep a variable may reference another variable. Bounds the
 *  resolver's recursion; a chain past it is an error, not a stack overflow.
 *  MIRRORS `regen::variables::MAX_VARIABLE_CHAIN`. */
export const MAX_VARIABLE_CHAIN = 64;

/** The minimum a table row must carry to be resolvable: a name, the stored
 *  (cached) number, and the expression that drives it, if any. */
export interface VariableEntry {
  name: string;
  value: number;
  expr?: string;
}

/** A variable whose own expression could not be resolved against the rest of
 *  the table. Route on `kind`, never on `message` text. */
export interface VariableError {
  name: string;
  kind: ExprErrorKind;
  message: string;
}

/** A resolved table: the value of every variable that resolved, plus the ones
 *  that did not. An errored variable is ABSENT from `values`, so anything
 *  referencing it fails too rather than silently reading a stale number. */
export interface ResolvedTable {
  values: Map<string, Value>;
  errors: VariableError[];
}

/** `(kind, message)` — the failure MODE for machine routing plus the
 *  human-facing reason. Same pair Rust's `evaluate_at_site` returns. */
export interface EvalFailure {
  kind: ExprErrorKind;
  message: string;
}

/**
 * Resolves a whole variable table into `name → Value` plus the variables that
 * could not resolve.
 *
 * Depth-first with a visiting stack, so a variable may be defined in terms of
 * another one regardless of declaration order. Three deterministic refusals:
 * a CYCLE (every member gets the same `"Cycle"` error naming the whole path),
 * an over-long CHAIN (past {@link MAX_VARIABLE_CHAIN}), and a BAD EXPRESSION
 * (whatever the engine refused it for, verbatim).
 *
 * A variable with no expression is a bare number, and a bare number is
 * `"scalar"`: it coerces into whatever unit the call site means, so `depth = 10`
 * drives a length field as 10 mm and an angle field as 10°.
 */
export function resolveVariableTable(vars: readonly VariableEntry[]): ResolvedTable {
  const byName = new Map<string, VariableEntry>();
  for (const v of vars) byName.set(v.name, v);
  const table: ResolvedTable = { values: new Map(), errors: [] };
  const stack: string[] = [];
  for (const v of vars) resolveOne(v.name, byName, table, stack);
  return table;
}

/** First error per name wins: the cycle detection stamps every member of a
 *  cycle up front, and the unwinding evaluation must not overwrite that
 *  specific diagnosis with the generic "undefined variable" it then hits. */
function pushError(table: ResolvedTable, name: string, kind: ExprErrorKind, message: string): void {
  if (table.errors.some((e) => e.name === name)) return;
  table.errors.push({ name, kind, message });
}

function resolveOne(
  name: string,
  byName: Map<string, VariableEntry>,
  table: ResolvedTable,
  stack: string[],
): void {
  if (table.values.has(name) || table.errors.some((e) => e.name === name)) return;
  const entry = byName.get(name);
  // Not a variable at all — the evaluator reports it at the use site.
  if (entry === undefined) return;

  const at = stack.indexOf(name);
  if (at >= 0) {
    const path = [...stack.slice(at), name];
    const message = `variable cycle: ${path.join(" → ")}`;
    for (const member of stack.slice(at)) pushError(table, member, "Cycle", message);
    return;
  }
  if (stack.length >= MAX_VARIABLE_CHAIN) {
    pushError(
      table,
      name,
      "TooDeep",
      `variable reference chain exceeds ${MAX_VARIABLE_CHAIN}`,
    );
    return;
  }
  if (entry.expr === undefined) {
    table.values.set(name, { number: entry.value, dim: "scalar" });
    return;
  }

  let ast: Expr;
  try {
    ast = parse(entry.expr);
  } catch (e) {
    const err = asExprError(e);
    pushError(table, name, err.kind, err.message);
    return;
  }
  stack.push(name);
  for (const referenced of references(ast)) resolveOne(referenced, byName, table, stack);
  stack.pop();
  try {
    table.values.set(name, evaluateValue(ast, envOf(table)));
  } catch (e) {
    const err = asExprError(e);
    pushError(table, name, err.kind, err.message);
  }
}

/** An `Env` reading a resolved table — the only lookup any evaluation here
 *  uses, so an errored (absent) variable fails loudly at its use site. */
export function envOf(table: ResolvedTable): Env {
  return (name) => table.values.get(name);
}

/**
 * Evaluates one expression at its call-site dimension against a resolved
 * table, returning the dimensioned {@link Value} or an {@link EvalFailure}.
 *
 * The table's own errors are consulted BEFORE evaluation so a parameter bound
 * to a broken variable reports the real cause. Without that the evaluator would
 * say "undefined variable `w`" about a `w` that plainly exists and is merely
 * unresolvable — a message that sends the user looking in the wrong place.
 */
export function evaluateAtSite(
  text: string,
  site: Dimension,
  table: ResolvedTable,
): { ok: true; value: Value } | { ok: false; failure: EvalFailure } {
  let ast: Expr;
  try {
    ast = parse(text);
  } catch (e) {
    return { ok: false, failure: failureOf(e) };
  }
  const referenced = references(ast);
  const bad = table.errors.find((e) => referenced.includes(e.name));
  if (bad !== undefined) {
    return {
      ok: false,
      failure: {
        kind: bad.kind,
        message: `variable \`${bad.name}\` is itself unresolved: ${bad.message}`,
      },
    };
  }
  const env = envOf(table);
  try {
    const value = evaluateValue(ast, env);
    // The SITE boundary is enforced by the engine rather than re-derived here,
    // so "which dimensions may coerce into which site" stays defined once.
    evaluate(ast, env, site);
    return { ok: true, value };
  } catch (e) {
    return { ok: false, failure: failureOf(e) };
  }
}

/**
 * Rewrites every reference to `oldName` in `text` as `newName`, returning
 * `null` when `text` does not reference it at all.
 *
 * **Token-wise, via the parser's identifier spans — never textual search.** A
 * substring rewrite would corrupt `width2` while renaming `width`, and would
 * rename a FUNCTION call if a variable were ever named `sin`; the AST
 * distinguishes an identifier from a call name and from a unit suffix.
 *
 * THROWS the expression's own parse error when `text` cannot be parsed at all.
 * A rename must then be REFUSED rather than applied to some sites and not
 * others: a half-renamed document is a silently wrong document.
 */
export function renameReference(text: string, oldName: string, newName: string): string | null {
  const spans: [number, number][] = [];
  collectIdentSpans(parse(text), oldName, spans);
  if (spans.length === 0) return null;
  // Splice back-to-front so earlier spans keep their offsets.
  spans.sort((a, b) => b[0] - a[0]);
  let out = text;
  for (const [start, end] of spans) out = out.slice(0, start) + newName + out.slice(end);
  return out;
}

function collectIdentSpans(node: Expr, name: string, out: [number, number][]): void {
  switch (node.type) {
    case "number":
      return;
    case "ident":
      if (node.name === name) out.push(node.span);
      return;
    case "unary":
      collectIdentSpans(node.expr, name, out);
      return;
    case "binary":
      collectIdentSpans(node.lhs, name, out);
      collectIdentSpans(node.rhs, name, out);
      return;
    // `Call.name` is deliberately NOT a rewrite site: a builtin is not a
    // variable, so `sin(sin)` renames only the argument.
    case "call":
      for (const arg of node.args) collectIdentSpans(arg, name, out);
      return;
  }
}

/** Every throw on this path is an `ExprError` (the engine raises nothing
 *  else); the guard exists so a genuine programming fault surfaces as itself
 *  instead of being laundered into a fake `"Parse"` diagnosis. */
function asExprError(e: unknown): ExprError {
  if (e instanceof ExprError) return e;
  throw e;
}

function failureOf(e: unknown): EvalFailure {
  const err = asExprError(e);
  return { kind: err.kind, message: err.message };
}
