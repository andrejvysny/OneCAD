/**
 * Shared error type + engine caps for the expression module — mirrors
 * `ExprError` / `ExprErrorKind` / `MAX_SOURCE_LEN` / `MAX_DEPTH` in
 * `src-tauri/crates/onecad-core/src/expr/mod.rs`.
 *
 * Rust keeps these in `mod.rs` alongside its submodules; here they live in
 * their own leaf file (no imports) so `lexer.ts` / `parser.ts` /
 * `dimension.ts` / `eval.ts` can all depend on them without an ESM import
 * cycle through `index.ts`.
 */

/** What went wrong. A distinct kind per failure mode — not one opaque
 * error — so callers/tests can assert on the FAILURE MODE (the golden
 * fixture's `"error": "<Kind>"` cases), never on message text. */
export type ExprErrorKind =
  | "Parse"
  | "UnknownFunction"
  | "Arity"
  | "UndefinedVariable"
  | "DimensionMismatch"
  | "DivideByZero"
  | "NonFinite"
  | "TooLong"
  | "TooDeep"
  /**
   * A variable is defined, directly or transitively, in terms of itself.
   *
   * **The evaluator never produces it.** An `Env` lookup is one opaque
   * callback, so nothing in `lexer`/`parser`/`eval` knows that a variable's
   * value is another variable's expression — a reference graph, and therefore a
   * cycle in one, does not exist at that layer. The kind lives in this single
   * taxonomy so callers route on ONE union covering every way an expression can
   * fail to produce a number; it is raised by `table.ts`'s
   * `resolveVariableTable`, which owns the variable graph (mirroring
   * `regen::variables::resolve_variable_table`).
   */
  | "Cycle";

/**
 * A span-carrying evaluation failure. `span` indexes into the source string
 * passed to `parse` / `evaluateStr` (start inclusive, end exclusive). The
 * grammar is ASCII-only end to end (digits, `A-Za-z_`, and the fixed
 * operator/paren set), so a JS string index here matches Rust's UTF-8 byte
 * offset 1:1 for every token the lexer can ever produce.
 */
export class ExprError extends Error {
  readonly kind: ExprErrorKind;
  readonly span: [number, number];

  constructor(kind: ExprErrorKind, span: [number, number], message: string) {
    super(message);
    this.name = "ExprError";
    this.kind = kind;
    this.span = span;
  }
}

/** Hard cap on source length (chars), checked before lexing. */
export const MAX_SOURCE_LEN = 512;

/** Hard cap on parenthesized/call-argument nesting depth, checked while
 * parsing. */
export const MAX_DEPTH = 32;
