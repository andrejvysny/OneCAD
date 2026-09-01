/**
 * Recursive-descent parser — turns the lexer's token stream into an `Expr`
 * tree. Mirrors `src-tauri/crates/onecad-core/src/expr/parser.rs`.
 *
 * ```text
 * expression = term ;
 * term       = factor , { ("+"|"-") , factor } ;
 * factor     = unary , { ("*"|"/") , unary } ;
 * unary      = ["+"|"-"] , power ;
 * power      = atom , ["^" , unary] ;   (* right-assoc: -2^2 = -(2^2) *)
 * atom       = literal | ident "(" [args] ")" | ident | "(" expression ")" ;
 * args       = expression , { "," , expression } ;
 * ```
 */
import type { Value } from "./dimension";
import { literalValue } from "./dimension";
import { ExprError, MAX_DEPTH, MAX_SOURCE_LEN } from "./errors";
import type { SpannedToken, Token } from "./lexer";
import { lex } from "./lexer";

/** A unary prefix operator. `"plus"` is a structural no-op (kept so `+2`
 * has a clean node/span rather than being silently dropped by the parser). */
export type UnaryOp = "plus" | "neg";

/** A binary infix operator. */
export type BinaryOp = "add" | "sub" | "mul" | "div" | "pow";

/** The parsed expression tree. A literal's number + unit is folded to a
 * canonical `Value` at parse time (units are context-free); everything
 * else is resolved at evaluation time against an `Env` and a call-site
 * `Dimension`. */
export type Expr =
  | { type: "number"; value: Value; span: [number, number] }
  | { type: "ident"; name: string; span: [number, number] }
  | { type: "unary"; op: UnaryOp; expr: Expr; span: [number, number] }
  | { type: "binary"; op: BinaryOp; lhs: Expr; rhs: Expr; span: [number, number] }
  | { type: "call"; name: string; args: Expr[]; span: [number, number] };

interface ParserState {
  tokens: SpannedToken[];
  pos: number;
  depth: number;
  srcLen: number;
}

/**
 * Parses `src` into an `Expr` tree. Rejects sources over `MAX_SOURCE_LEN`
 * chars before lexing at all, and expressions nested past `MAX_DEPTH`
 * parens/call-args deep while parsing.
 */
export function parse(src: string): Expr {
  if (src.length > MAX_SOURCE_LEN) {
    throw new ExprError("TooLong", [0, src.length], `expression exceeds MAX_SOURCE_LEN (${MAX_SOURCE_LEN} bytes)`);
  }
  const tokens = lex(src);
  const state: ParserState = { tokens, pos: 0, depth: 0, srcLen: src.length };
  const expr = parseExpression(state);
  const trailing = state.tokens[state.pos];
  if (trailing !== undefined) {
    throw new ExprError("Parse", trailing.span, "unexpected trailing input");
  }
  return expr;
}

function peek(state: ParserState): Token | undefined {
  return state.tokens[state.pos]?.tok;
}

function peekSpan(state: ParserState): [number, number] {
  const t = state.tokens[state.pos];
  return t ? t.span : [state.srcLen, state.srcLen];
}

function bump(state: ParserState): SpannedToken | undefined {
  const t = state.tokens[state.pos];
  if (t !== undefined) state.pos += 1;
  return t;
}

function eofError(state: ParserState, what: string): ExprError {
  return new ExprError("Parse", peekSpan(state), `expected ${what}, found end of input`);
}

function expectRParen(state: ParserState): number {
  const t = bump(state);
  if (t === undefined) throw eofError(state, "`)`");
  if (t.tok.type !== "rparen") throw new ExprError("Parse", t.span, "expected `)`");
  return t.span[1];
}

/** Bounds nesting via `MAX_DEPTH`: called for the top-level expression,
 * each parenthesized sub-expression, and each function call argument —
 * exactly the places a source can nest. */
function parseExpression(state: ParserState): Expr {
  state.depth += 1;
  if (state.depth > MAX_DEPTH) {
    const span = peekSpan(state);
    state.depth -= 1;
    throw new ExprError("TooDeep", span, `expression nested past MAX_DEPTH (${MAX_DEPTH})`);
  }
  try {
    return parseTerm(state);
  } finally {
    state.depth -= 1;
  }
}

function parseTerm(state: ParserState): Expr {
  let lhs = parseFactor(state);
  for (;;) {
    const t = peek(state);
    const op: BinaryOp | undefined = t?.type === "plus" ? "add" : t?.type === "minus" ? "sub" : undefined;
    if (op === undefined) break;
    bump(state);
    const rhs = parseFactor(state);
    lhs = { type: "binary", op, lhs, rhs, span: [lhs.span[0], rhs.span[1]] };
  }
  return lhs;
}

function parseFactor(state: ParserState): Expr {
  let lhs = parseUnary(state);
  for (;;) {
    const t = peek(state);
    const op: BinaryOp | undefined = t?.type === "star" ? "mul" : t?.type === "slash" ? "div" : undefined;
    if (op === undefined) break;
    bump(state);
    const rhs = parseUnary(state);
    lhs = { type: "binary", op, lhs, rhs, span: [lhs.span[0], rhs.span[1]] };
  }
  return lhs;
}

function parseUnary(state: ParserState): Expr {
  const t = peek(state);
  const op: UnaryOp | undefined = t?.type === "plus" ? "plus" : t?.type === "minus" ? "neg" : undefined;
  if (op === undefined) return parsePower(state);
  const start = peekSpan(state)[0];
  bump(state);
  const operand = parsePower(state);
  return { type: "unary", op, expr: operand, span: [start, operand.span[1]] };
}

/** Right-associative: recursing back into `parseUnary` for the exponent
 * makes `2^3^2` parse as `2^(3^2)` and `2^-3` parse the unary minus as
 * part of the exponent, not a separate top-level negation. */
function parsePower(state: ParserState): Expr {
  const base = parseAtom(state);
  if (peek(state)?.type !== "caret") return base;
  bump(state);
  const exponent = parseUnary(state);
  return { type: "binary", op: "pow", lhs: base, rhs: exponent, span: [base.span[0], exponent.span[1]] };
}

function parseAtom(state: ParserState): Expr {
  const spanned = bump(state);
  if (spanned === undefined) throw eofError(state, "an expression");
  const tok = spanned.tok;
  switch (tok.type) {
    case "literal":
      return { type: "number", value: literalValue(tok.value, tok.unit), span: spanned.span };
    case "ident":
      return parseIdentAtom(state, tok.name, spanned.span);
    case "lparen": {
      const inner = parseExpression(state);
      expectRParen(state);
      return inner; // parens group; they are not their own node
    }
    default:
      throw new ExprError("Parse", spanned.span, "expected a number, identifier, or `(`");
  }
}

function parseIdentAtom(state: ParserState, name: string, nameSpan: [number, number]): Expr {
  if (peek(state)?.type !== "lparen") {
    return { type: "ident", name, span: nameSpan };
  }
  bump(state);
  const args = parseArgs(state);
  const end = expectRParen(state);
  return { type: "call", name, args, span: [nameSpan[0], end] };
}

function parseArgs(state: ParserState): Expr[] {
  if (peek(state)?.type === "rparen") return [];
  const args = [parseExpression(state)];
  while (peek(state)?.type === "comma") {
    bump(state);
    args.push(parseExpression(state));
  }
  return args;
}

/** Identifiers referenced by `expr` (bare variable names, not call names),
 * in first-appearance order, deduplicated — feeds a future variable
 * dependency graph / rename. */
export function references(expr: Expr): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  collectReferences(expr, seen, out);
  return out;
}

function collectReferences(expr: Expr, seen: Set<string>, out: string[]): void {
  switch (expr.type) {
    case "number":
      return;
    case "ident":
      if (!seen.has(expr.name)) {
        seen.add(expr.name);
        out.push(expr.name);
      }
      return;
    case "unary":
      collectReferences(expr.expr, seen, out);
      return;
    case "binary":
      collectReferences(expr.lhs, seen, out);
      collectReferences(expr.rhs, seen, out);
      return;
    case "call":
      for (const arg of expr.args) collectReferences(arg, seen, out);
      return;
  }
}
