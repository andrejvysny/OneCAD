/*
 * The variable-GRAPH layer: table resolution, the site boundary, and the
 * token-wise rename. Mirrors the Rust unit tests over
 * `regen::variables::{resolve_variable_table, evaluate_expression,
 * rename_reference}`.
 *
 * Asserted on the failure KIND, never on message text, everywhere the kind is
 * the point — same discipline as the golden fixture the evaluator is pinned by.
 */
import { describe, expect, it } from "vitest";
import { evaluateAtSite, renameReference, resolveVariableTable, MAX_VARIABLE_CHAIN } from "./table";
import type { VariableEntry } from "./table";

const table = (...vars: VariableEntry[]) => resolveVariableTable(vars);

describe("resolveVariableTable", () => {
  /** A variable with no expression is a bare number, and a bare number is
   *  DIMENSIONLESS — it coerces into whatever unit the call site means. */
  it("reads a plain variable as a dimensionless number", () => {
    const t = table({ name: "depth", value: 10 });
    expect(t.values.get("depth")).toEqual({ number: 10, dim: "scalar" });
    expect(t.errors).toEqual([]);
  });

  it("infers a variable's dimension from its own expression", () => {
    const t = table(
      { name: "bar", value: 0, expr: "10mm" },
      { name: "tilt", value: 0, expr: "45deg" },
      { name: "ratio", value: 0, expr: "3 / 2" },
    );
    expect(t.values.get("bar")).toEqual({ number: 10, dim: "length" });
    expect(t.values.get("tilt")).toEqual({ number: 45, dim: "angle" });
    expect(t.values.get("ratio")).toEqual({ number: 1.5, dim: "scalar" });
  });

  /** Depth-first over the reference graph, not a single forward pass: a
   *  variable may be defined in terms of one declared AFTER it. */
  it("resolves a chain regardless of declaration order", () => {
    const t = table(
      { name: "c", value: 0, expr: "b * 2" },
      { name: "b", value: 0, expr: "w + 1" },
      { name: "w", value: 4 },
    );
    expect(t.errors).toEqual([]);
    expect(t.values.get("c")?.number).toBe(10);
  });

  /** Every member of the cycle gets the SAME diagnosis naming the whole path —
   *  and none of them gets a value, so nothing downstream reads a stale one. */
  it("reports a cycle once per member, naming the path, with no value", () => {
    const t = table(
      { name: "a", value: 0, expr: "b + 1" },
      { name: "b", value: 0, expr: "a + 1" },
    );
    expect(t.errors.map((e) => e.kind)).toEqual(["Cycle", "Cycle"]);
    expect(t.errors.map((e) => e.name).sort()).toEqual(["a", "b"]);
    expect(t.errors[0].message).toContain("→");
    expect(t.values.has("a")).toBe(false);
    expect(t.values.has("b")).toBe(false);
  });

  it("reports a self-reference as a cycle", () => {
    const t = table({ name: "w", value: 0, expr: "w + 1" });
    expect(t.errors.map((e) => e.kind)).toEqual(["Cycle"]);
  });

  /** A healthy variable is unaffected by a broken sibling. */
  it("keeps resolving the rest of the table around a broken variable", () => {
    const t = table({ name: "ok", value: 3 }, { name: "bad", value: 7, expr: "gone * 2" });
    expect(t.values.get("ok")?.number).toBe(3);
    expect(t.errors.map((e) => e.kind)).toEqual(["UndefinedVariable"]);
    // An errored variable is ABSENT, so anything referencing it fails too rather
    // than silently reading its stale cache.
    expect(t.values.has("bad")).toBe(false);
  });

  it("refuses a reference chain past MAX_VARIABLE_CHAIN", () => {
    const vars: VariableEntry[] = [{ name: "v0", value: 1 }];
    for (let i = 1; i <= MAX_VARIABLE_CHAIN + 2; i += 1) {
      vars.push({ name: `v${i}`, value: 0, expr: `v${i - 1} + 1` });
    }
    // Resolved from the DEEPEST end, so the stack is at its longest.
    const t = resolveVariableTable([...vars].reverse());
    expect(t.errors.some((e) => e.kind === "TooDeep")).toBe(true);
  });

  it("surfaces a variable's own parse failure verbatim", () => {
    const t = table({ name: "w", value: 0, expr: "2 +" });
    expect(t.errors.map((e) => e.kind)).toEqual(["Parse"]);
  });
});

describe("evaluateAtSite", () => {
  const vars = table({ name: "w", value: 20 }, { name: "tilt", value: 0, expr: "45deg" });

  it("evaluates against the table, keeping the value's own dimension", () => {
    const r = evaluateAtSite("w * 2 + 5mm", "length", vars);
    expect(r).toEqual({ ok: true, value: { number: 45, dim: "length" } });
  });

  /** A bare number inside an expression is ALREADY in the site's canonical
   *  unit, so a scalar result passes through with no rescale. */
  it("coerces a scalar result into the site verbatim", () => {
    expect(evaluateAtSite("w * 2", "length", vars)).toEqual({
      ok: true,
      value: { number: 40, dim: "scalar" },
    });
    expect(evaluateAtSite("w * 2", "angle", vars)).toEqual({
      ok: true,
      value: { number: 40, dim: "scalar" },
    });
  });

  /** The site boundary: an angle in a length field is a loud refusal, which is
   *  the whole reason the site is passed at all. */
  it("refuses a dimension the site cannot take", () => {
    const r = evaluateAtSite("tilt", "length", vars);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.failure.kind).toBe("DimensionMismatch");
  });

  /**
   * A parameter bound to a BROKEN variable must report the real cause. Saying
   * "undefined variable `plate`" about a `plate` that plainly exists sends the
   * user looking in the wrong place.
   */
  it("names the broken variable rather than calling it undefined", () => {
    const broken = table({ name: "plate", value: 8, expr: "gone * 2" });
    const r = evaluateAtSite("plate + 1mm", "length", broken);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.failure.message).toMatch(/`plate` is itself unresolved/);
  });

  it("reports an unknown reference as UndefinedVariable", () => {
    const r = evaluateAtSite("nope", "length", vars);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.failure.kind).toBe("UndefinedVariable");
  });
});

describe("renameReference", () => {
  /** Token-wise via the parser's identifier spans. A substring rewrite would
   *  corrupt `width2` while renaming `width`. */
  it("renames only whole identifiers", () => {
    expect(renameReference("width + width2", "width", "w")).toBe("w + width2");
    expect(renameReference("aw + wa", "w", "x")).toBe(null);
  });

  it("returns null when the text does not reference the name at all", () => {
    expect(renameReference("2 * depth", "w", "x")).toBe(null);
  });

  /** Multiple sites are spliced back-to-front, so earlier spans keep their
   *  offsets even when the new name is a different length. */
  it("rewrites every occurrence, whatever the length change", () => {
    expect(renameReference("w + w * w", "w", "width")).toBe("width + width * width");
    expect(renameReference("width + width", "width", "w")).toBe("w + w");
  });

  /** A builtin is not a variable: `sin(sin)` renames only the ARGUMENT. */
  it("never rewrites a function name", () => {
    expect(renameReference("sin(sin)", "sin", "s")).toBe("sin(s)");
  });

  it("leaves a unit suffix alone", () => {
    expect(renameReference("2mm + m", "m", "metres")).toBe("2mm + metres");
  });

  /** THROWS rather than half-renaming: a rename applied to some sites and not
   *  others is a silently wrong document. */
  it("throws on text it cannot parse", () => {
    expect(() => renameReference("w +", "w", "x")).toThrow();
  });
});
