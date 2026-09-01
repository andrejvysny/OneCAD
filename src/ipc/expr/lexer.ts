/**
 * Tokenizer — turns source text into a flat token stream with spans.
 * Mirrors `src-tauri/crates/onecad-core/src/expr/lexer.rs`.
 *
 * Unit adjacency (`2m` vs `2 m` vs `2*m`) is resolved HERE, once: a unit
 * suffix is recognised only when its alpha run immediately follows the
 * number's digits with zero intervening whitespace, and only when that run
 * is an EXACT match for one of the six keywords — no prefix matching, so
 * `1radius` is a lex error rather than a silent `1rad` + stray `ius`
 * identifier. Any other adjacency (`2 m`, with whitespace) leaves the
 * letters as a separate `ident` token, which the parser then rejects as two
 * atoms with no operator between them.
 */
import type { Unit } from "./dimension";
import { unitFromSuffix } from "./dimension";
import { ExprError } from "./errors";

export type Token =
  | { type: "literal"; value: number; unit: Unit | undefined }
  | { type: "ident"; name: string }
  | { type: "plus" }
  | { type: "minus" }
  | { type: "star" }
  | { type: "slash" }
  | { type: "caret" }
  | { type: "lparen" }
  | { type: "rparen" }
  | { type: "comma" };

export interface SpannedToken {
  tok: Token;
  span: [number, number];
}

function isDigit(c: string): boolean {
  return c >= "0" && c <= "9";
}

function isAlpha(c: string): boolean {
  return (c >= "a" && c <= "z") || (c >= "A" && c <= "Z");
}

function isWhitespace(c: string): boolean {
  return c === " " || c === "\t" || c === "\n" || c === "\r" || c === "\f" || c === "\v";
}

function singleCharToken(c: string): Token | undefined {
  switch (c) {
    case "+":
      return { type: "plus" };
    case "-":
      return { type: "minus" };
    case "*":
      return { type: "star" };
    case "/":
      return { type: "slash" };
    case "^":
      return { type: "caret" };
    case "(":
      return { type: "lparen" };
    case ")":
      return { type: "rparen" };
    case ",":
      return { type: "comma" };
    default:
      return undefined;
  }
}

/** `number = digits ["." [digits]] [exponent] | "." digits [exponent]`,
 * `exponent = ("e"|"E") ["+"|"-"] digits`. Never fails — the caller parses
 * the sliced text with `Number(...)`, which does the actual validation. */
function scanNumberDigits(src: string, start: number, n: number): number {
  let i = start;
  while (i < n && isDigit(src[i])) i += 1;
  if (i < n && src[i] === ".") {
    i += 1;
    while (i < n && isDigit(src[i])) i += 1;
  }
  if (i < n && (src[i] === "e" || src[i] === "E")) {
    let j = i + 1;
    if (j < n && (src[j] === "+" || src[j] === "-")) j += 1;
    if (j < n && isDigit(src[j])) {
      while (j < n && isDigit(src[j])) j += 1;
      i = j;
      // else: the 'e'/'E' is not an exponent marker (no digit follows) —
      // leave it for the adjacent-unit check, where it fails as an
      // unrecognized suffix.
    }
  }
  return i;
}

/** Recognizes an adjacent (zero-whitespace) unit suffix right after a
 * number's digits — a WHOLE-keyword match only. */
function scanUnitSuffix(src: string, start: number, n: number): { unit: Unit | undefined; next: number } {
  let i = start;
  if (i >= n || !isAlpha(src[i])) return { unit: undefined, next: i };
  const suffixStart = i;
  while (i < n && isAlpha(src[i])) i += 1;
  const suffix = src.slice(suffixStart, i);
  const unit = unitFromSuffix(suffix);
  if (unit === undefined) {
    throw new ExprError(
      "Parse",
      [suffixStart, i],
      `\`${suffix}\` is not a recognized unit (mm, cm, m, in, deg, rad)`,
    );
  }
  return { unit, next: i };
}

function lexNumber(src: string, start: number, n: number): { tok: Token; next: number } {
  const afterDigits = scanNumberDigits(src, start, n);
  const text = src.slice(start, afterDigits);
  const value = Number(text);
  if (!Number.isFinite(value)) {
    throw new ExprError("Parse", [start, afterDigits], `invalid number literal \`${text}\``);
  }
  const { unit, next } = scanUnitSuffix(src, afterDigits, n);
  return { tok: { type: "literal", value, unit }, next };
}

/** `ident = (ALPHA|"_") {ALPHA|DIGIT|"_"}`. */
function lexIdentEnd(src: string, start: number, n: number): number {
  let i = start + 1; // first char already validated alpha|_ by the caller
  while (i < n && (isAlpha(src[i]) || isDigit(src[i]) || src[i] === "_")) i += 1;
  return i;
}

/** Tokenizes `src`. The caller has already checked `src.length` against
 * `MAX_SOURCE_LEN`. */
export function lex(src: string): SpannedToken[] {
  const n = src.length;
  const out: SpannedToken[] = [];
  let i = 0;

  while (i < n) {
    const c = src[i];
    if (isWhitespace(c)) {
      i += 1;
      continue;
    }
    if (isDigit(c) || c === ".") {
      const start = i;
      const { tok, next } = lexNumber(src, i, n);
      out.push({ tok, span: [start, next] });
      i = next;
      continue;
    }
    if (isAlpha(c) || c === "_") {
      const start = i;
      const next = lexIdentEnd(src, i, n);
      out.push({ tok: { type: "ident", name: src.slice(start, next) }, span: [start, next] });
      i = next;
      continue;
    }
    const tok = singleCharToken(c);
    if (tok === undefined) {
      throw new ExprError("Parse", [i, i + 1], `unexpected character \`${c}\``);
    }
    out.push({ tok, span: [i, i + 1] });
    i += 1;
  }
  return out;
}
