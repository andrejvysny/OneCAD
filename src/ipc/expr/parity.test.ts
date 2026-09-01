/*
 * Golden-fixture replay of `docs/qa/expressions.fixture.json` — the TS
 * mirror of onecad-core's expr engine (`src-tauri/crates/onecad-core/src/expr/**`)
 * must evaluate every case identically to the Rust evaluator. The Rust side
 * asserts the SAME file, byte-for-byte, in
 * `src-tauri/crates/onecad-core/tests/expression_engine.rs`. Rust is the
 * sole hash authority (CLAUDE.md) — nothing this module computes reaches
 * geometry — so a last-ULP transcendental difference between the two
 * runtimes here is a checkpoint MISS to catch, never wrong geometry.
 *
 * Plus unit tests for adjacency, precedence, the depth cap, and
 * `references()` order — most of these are already exercised by fixture
 * cases, but are asserted directly here too since the brief calls them out
 * by name.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Dimension, ExprErrorKind, Value } from "./index";
import { ExprError, MAX_DEPTH, evaluateStr, parse, references } from "./index";

interface VarSpec {
  value: number;
  dim: Dimension;
}
interface ExpectValue {
  value: number;
  dim: Dimension;
}
interface Case {
  expr: string;
  site: Dimension;
  vars?: Record<string, VarSpec>;
  expect?: ExpectValue;
  error?: ExprErrorKind;
}
interface Fixture {
  version: number;
  cases: Case[];
}

const FIXTURE_PATH = join(dirname(fileURLToPath(import.meta.url)), "../../../docs/qa/expressions.fixture.json");

function loadFixture(): Fixture {
  return JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as Fixture;
}

function envFrom(vars: Record<string, VarSpec>): (name: string) => Value | undefined {
  return (name) => {
    const v = vars[name];
    return v ? { number: v.value, dim: v.dim } : undefined;
  };
}

describe("expression engine golden fixture parity", () => {
  const fixture = loadFixture();

  it("carries at least 40 cases (matches the Rust-side floor)", () => {
    expect(fixture.version).toBe(1);
    expect(fixture.cases.length).toBeGreaterThanOrEqual(40);
  });

  fixture.cases.forEach((c, i) => {
    it(`case ${i}: \`${c.expr}\` (site=${c.site})`, () => {
      const env = envFrom(c.vars ?? {});
      if (c.expect) {
        // Fixture self-consistency: expect.dim always mirrors site, since
        // `evaluate()` coerces its result into the site's canonical unit
        // and returns a bare number, not a dimension.
        expect(c.expect.dim).toBe(c.site);
        const got = evaluateStr(c.expr, env, c.site);
        const tol = 1e-12 * Math.max(1, Math.abs(c.expect.value));
        expect(Math.abs(got - c.expect.value)).toBeLessThanOrEqual(tol);
      } else if (c.error) {
        let err: ExprError | undefined;
        try {
          evaluateStr(c.expr, env, c.site);
        } catch (e) {
          err = e as ExprError;
        }
        expect(err, `case ${i} (\`${c.expr}\`) should have thrown`).toBeInstanceOf(ExprError);
        expect(err?.kind).toBe(c.error);
      } else {
        throw new Error(`case ${i} (\`${c.expr}\`): fixture must set exactly one of expect/error`);
      }
    });
  });
});

describe("unit adjacency", () => {
  it("adjacent digits+unit fold into one literal (2mm)", () => {
    expect(evaluateStr("2mm", () => undefined, "length")).toBeCloseTo(2, 12);
  });

  it("whitespace before a would-be suffix is a parse error, not adjacency (2 m)", () => {
    let err: ExprError | undefined;
    try {
      evaluateStr("2 m", () => undefined, "scalar");
    } catch (e) {
      err = e as ExprError;
    }
    expect(err).toBeInstanceOf(ExprError);
    expect(err?.kind).toBe("Parse");
  });

  it("2*m multiplies by the variable m, never reads it as a unit", () => {
    const env = (name: string): Value | undefined => (name === "m" ? { number: 3, dim: "length" } : undefined);
    expect(evaluateStr("2*m", env, "length")).toBeCloseTo(6, 12);
  });
});

describe("precedence and associativity", () => {
  it("-2^2 = -(2^2) = -4 (unary binds looser than power)", () => {
    expect(evaluateStr("-2^2", () => undefined, "scalar")).toBe(-4);
  });

  it("2^3^2 = 2^(3^2) = 512 (power is right-associative)", () => {
    expect(evaluateStr("2^3^2", () => undefined, "scalar")).toBe(512);
  });

  it("1+2*3 parses as Add(1, Mul(2,3)), not Mul(Add(1,2),3)", () => {
    const e = parse("1+2*3");
    expect(e.type).toBe("binary");
    if (e.type !== "binary") throw new Error("unreachable");
    expect(e.op).toBe("add");
    expect(e.rhs.type).toBe("binary");
    if (e.rhs.type !== "binary") throw new Error("unreachable");
    expect(e.rhs.op).toBe("mul");
  });
});

describe("depth cap", () => {
  it(`MAX_DEPTH-1 (${MAX_DEPTH - 1}) nested parens still parses`, () => {
    const src = `${"(".repeat(MAX_DEPTH - 1)}1${")".repeat(MAX_DEPTH - 1)}`;
    expect(() => parse(src)).not.toThrow();
  });

  it(`MAX_DEPTH (${MAX_DEPTH}) nested parens is TooDeep`, () => {
    const src = `${"(".repeat(MAX_DEPTH)}1${")".repeat(MAX_DEPTH)}`;
    let err: ExprError | undefined;
    try {
      parse(src);
    } catch (e) {
      err = e as ExprError;
    }
    expect(err).toBeInstanceOf(ExprError);
    expect(err?.kind).toBe("TooDeep");
  });
});

describe("references()", () => {
  it("returns names in first-appearance order, deduplicated", () => {
    const e = parse("a+b*a+c");
    expect(references(e)).toEqual(["a", "b", "c"]);
  });

  it("ignores call names", () => {
    const e = parse("sin(x)+y");
    expect(references(e)).toEqual(["x", "y"]);
  });
});
