/**
 * TypeScript mirror of onecad-core's expression engine
 * (`src-tauri/crates/onecad-core/src/expr/mod.rs`), for the mock/local-only
 * IPC lane. Semantics MUST match the Rust evaluator exactly: grammar,
 * dimension algebra, and error kinds. Parity is proven by
 * `docs/qa/expressions.fixture.json`, replayed by both `parity.test.ts`
 * (here) and `onecad-core/tests/expression_engine.rs` (there).
 *
 * Rust remains the sole hash authority for anything that reaches geometry —
 * this module exists only to drive the mock UI without a worker round-trip.
 * `table.ts` adds the variable-GRAPH layer over it (chained references,
 * cycles, rename), mirroring `onecad-core`'s `regen::variables`, and is what
 * `mockClient` resolves its table with.
 */
export type { Dimension, Value } from "./dimension";
export type { Env } from "./eval";
export { evaluate, evaluateStr, evaluateValue } from "./eval";
export type { ExprErrorKind } from "./errors";
export { ExprError, MAX_DEPTH, MAX_SOURCE_LEN } from "./errors";
export type { BinaryOp, Expr, UnaryOp } from "./parser";
export { parse, references } from "./parser";
export type { EvalFailure, ResolvedTable, VariableEntry, VariableError } from "./table";
export {
  envOf,
  evaluateAtSite,
  MAX_VARIABLE_CHAIN,
  renameReference,
  resolveVariableTable,
} from "./table";
