/*
 * WP-VE.2 — the mock lane's document variable table, and the binding round trip
 * through `updateOperationParams`.
 *
 * The mock is the ONLY backend the e2e lane and most component tests ever see,
 * so its validation has to mirror `api::upsert_variable_command` /
 * `api::remove_variable_command` exactly. Where it drifts, a test that proves the
 * UI handles a refusal proves nothing about the real app.
 */
import { beforeEach, describe, it, expect } from "vitest";
import { mockClient, resetMockDocument, setMockLatency } from "./mockClient";

/** Bind the seeded f2 extrude's distance to an expression — the `=` gesture's
 *  wire form (`Scalar {value, expr}`). */
async function bindF2(expr: string, cached = 30): Promise<void> {
  await mockClient.applyEditCommand({
    cmd: "updateOperationParams",
    record: "f2",
    op: {
      opType: "Extrude",
      params: {
        profile: { sketchId: "sketch1", regionId: "r0" },
        draftAngleDeg: { value: 0 },
        distance: { value: cached, expr },
      },
    },
  } as unknown as Parameters<typeof mockClient.applyEditCommand>[0]);
}

describe("mock document variables", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  it("starts empty and is forgotten on reset", async () => {
    expect(await mockClient.listVariables()).toEqual([]);
    await mockClient.upsertVariable("w", 1);
    resetMockDocument();
    expect(await mockClient.listVariables()).toEqual([]);
  });

  it("keeps DECLARATION order across a re-value", async () => {
    await mockClient.upsertVariable("width", 10);
    await mockClient.upsertVariable("height", 20);
    const after = await mockClient.upsertVariable("width", 15);
    expect(after.variables.map((v) => v.name)).toEqual(["width", "height"]);
    expect(after.variables[0].value).toBe(15);
  });

  it("refuses a name no expression could ever resolve", async () => {
    for (const bad of ["", "   ", "2wide", "my-var", "w * 2", "a b"]) {
      await expect(mockClient.upsertVariable(bad, 1)).rejects.toThrow(/invalid variable name/);
    }
    expect(await mockClient.listVariables()).toEqual([]);
  });

  it("accepts the names `is_bare_name` accepts, trimming surrounding space", async () => {
    for (const good of ["w", "_w", "width2", "WIDTH"]) {
      await expect(mockClient.upsertVariable(good, 1)).resolves.toBeDefined();
    }
    const table = await mockClient.upsertVariable("  padded  ", 2);
    expect(table.variables.map((v) => v.name)).toContain("padded");
  });

  it("refuses a non-finite value", async () => {
    await expect(mockClient.upsertVariable("w", Number.NaN)).rejects.toThrow(/finite/);
    await expect(mockClient.upsertVariable("w", Number.POSITIVE_INFINITY)).rejects.toThrow(/finite/);
  });

  it("refuses removing a name it does not hold — never a silent no-op", async () => {
    await expect(mockClient.removeVariable("nope")).rejects.toThrow(/unknown variable/);
    await mockClient.upsertVariable("w", 1);
    // Case-sensitive, matching `VariableTable::get`.
    await expect(mockClient.removeVariable("W")).rejects.toThrow(/unknown variable/);
    expect((await mockClient.removeVariable("w")).variables).toEqual([]);
  });

  it("emits document-changed so a listening panel refreshes", async () => {
    const seen: number[] = [];
    const off = mockClient.onDocumentChanged((c) => seen.push(c.revision));
    await mockClient.upsertVariable("w", 1);
    await mockClient.upsertVariable("w", 2);
    await mockClient.removeVariable("w");
    off();
    expect(seen).toHaveLength(3);
    expect(seen[1]).toBeGreaterThan(seen[0]);
  });
});

describe("mock lane — a bound scalar survives the params round trip", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  /*
   * MIRRORS `dto.rs feature_value`, which mints `primaryExpr` from the same
   * `Scalar` the number came from. Without this the mock lane would render a
   * bound extrude as a plain number and the e2e binding spec would be vacuous.
   */
  it("projects primaryExpr onto the feature row, and drops it when cleared", async () => {
    // A complete params object, as every producer sends (the backend replaces the
    // whole op) — the seeded f2 row carries no stored params to merge over.
    const stored = { profile: { sketchId: "sketch1", regionId: "r0" }, draftAngleDeg: { value: 0 } };
    const bind = (expr?: string) => ({
      cmd: "updateOperationParams" as const,
      record: "f2",
      op: {
        opType: "Extrude",
        params: { ...stored, distance: expr === undefined ? { value: 30 } : { value: 30, expr } },
      },
    });

    const bound = await mockClient.applyEditCommand(
      bind("height") as unknown as Parameters<typeof mockClient.applyEditCommand>[0],
    );
    expect(bound.features.find((f) => f.id === "f2")?.primaryExpr).toBe("height");

    const cleared = await mockClient.applyEditCommand(
      bind() as unknown as Parameters<typeof mockClient.applyEditCommand>[0],
    );
    expect(cleared.features.find((f) => f.id === "f2")?.primaryExpr).toBeUndefined();
  });
});

/*
 * W5 — result truth for the variable commands.
 *
 * "Saved + loud failure": a variable edit whose downstream regen fails reports the
 * SAVE as real and the FAILURE as real, in one result. It never reverts the write
 * and never turns a failed rebuild into a rejection — the variable really was
 * saved, and a rejection would say it was not.
 */
describe("mock lane — a variable edit reports BOTH truths (W5)", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  /** Bind the seeded f2 extrude's distance to `=name` (the `=name` gesture's wire form). */
  async function bindF2To(expr: string, cached = 30) {
    const command = {
      cmd: "updateOperationParams" as const,
      record: "f2",
      op: {
        opType: "Extrude",
        params: {
          profile: { sketchId: "sketch1", regionId: "r0" },
          draftAngleDeg: { value: 0 },
          distance: { value: cached, expr },
        },
      },
    };
    await mockClient.applyEditCommand(
      command as unknown as Parameters<typeof mockClient.applyEditCommand>[0],
    );
  }

  it("reports a healthy edit as a success terminal, with the saved table", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2To("height");
    const res = await mockClient.upsertVariable("height", 40);
    expect(res.terminal).toBe("noop"); // nothing rebuilt on this lane — not a failure
    expect(res.errorMessage).toBeUndefined();
    expect(res.variables).toEqual([
      { id: expect.any(String), name: "height", value: 40, resolvedValue: 40, dimension: "scalar" },
    ]);
  });

  it("a value that drives a bound extrude below the kernel floor FAILS — and the variable is still saved", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2To("height");

    const res = await mockClient.upsertVariable("height", 0);

    // Truth 1: the write landed. Not reverted, not rethrown.
    expect(res.variables).toEqual([
      { id: expect.any(String), name: "height", value: 0, resolvedValue: 0, dimension: "scalar" },
    ]);
    expect(await mockClient.listVariables()).toEqual([
      { id: expect.any(String), name: "height", value: 0, resolvedValue: 0, dimension: "scalar" },
    ]);
    // Truth 2: the rebuild it scheduled failed, with the kernel's own reason
    // (`ExtrudeOp.cpp` kMinValue).
    expect(res.terminal).toBe("failed");
    expect(res.errorMessage).toMatch(/Extrude distance too small/);
  });

  it("removing a variable a record still binds STANDS, and reports the resolver's reason", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2To("height");

    const res = await mockClient.removeVariable("height");

    expect(res.variables).toEqual([]);
    expect(await mockClient.listVariables()).toEqual([]);
    expect(res.terminal).toBe("failed");
    // Mirrors `regen::variables::substitute_variables`'s `${field}: ${reason}`,
    // with the reason now the shared evaluator's own `UndefinedVariable` text.
    expect(res.errorMessage).toMatch(/Extrude\.distance: undefined variable `height`/);
  });

  it("an UNBOUND record is never blamed for a variable edit", async () => {
    await mockClient.upsertVariable("height", 25);
    // f2 stays a literal distance — no `expr`, so no substitution and no failure.
    await bindF2To("", 30).catch(() => undefined);
    const res = await mockClient.upsertVariable("height", 0);
    expect(res.terminal).toBe("noop");
    expect(res.errorMessage).toBeUndefined();
  });
});

/*
 * The expression commands, against the mock's real table.
 *
 * The mock resolves through the SHARED TS port of `onecad-core`'s evaluator
 * (`ipc/expr`), so these prove the semantics the backend applies — chained
 * references, dimensions, cycles — rather than a second local approximation
 * that would be free to drift.
 */
describe("mock lane — expression-driven variables", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  it("stores the TEXT and the number it currently evaluates to", async () => {
    await mockClient.upsertVariable("depth", 10);
    const res = await mockClient.upsertVariableExpr("plate", "depth * 2 + 5mm");
    expect(res.variables[1]).toMatchObject({
      name: "plate",
      expr: "depth * 2 + 5mm",
      value: 25,
      resolvedValue: 25,
      dimension: "length",
    });
  });

  /** A plain number is DIMENSIONLESS: it reads as mm in a length field and as
   *  degrees in an angle field. A variable that wants a unit says so. */
  it("infers each variable's dimension from its OWN expression", async () => {
    await mockClient.upsertVariable("plain", 10);
    await mockClient.upsertVariableExpr("tilt", "45deg");
    await mockClient.upsertVariableExpr("bar", "10mm");
    const table = await mockClient.listVariables();
    expect(table.map((v) => v.dimension)).toEqual(["scalar", "angle", "length"]);
  });

  /** A variable may be defined in terms of another one, to a chain depth of
   *  MAX_VARIABLE_CHAIN. (Declaration ORDER independence is proved directly
   *  against the resolver in `expr/table.test.ts` — it cannot be reached from
   *  here, because authoring `c = b * 2` before `b` exists is refused.) */
  it("resolves a chain of references", async () => {
    await mockClient.upsertVariable("w", 4);
    await mockClient.upsertVariableExpr("b", "w + 1");
    await mockClient.upsertVariableExpr("c", "b * 2");
    const table = await mockClient.listVariables();
    expect(table.find((v) => v.name === "c")?.resolvedValue).toBe(10);
    // …and a later edit to the ROOT moves the whole chain.
    await mockClient.upsertVariable("w", 9);
    const moved = await mockClient.listVariables();
    expect(moved.find((v) => v.name === "c")?.resolvedValue).toBe(20);
  });

  it("refuses authoring a reference to a name that does not exist yet", async () => {
    await expect(mockClient.upsertVariableExpr("c", "b * 2")).rejects.toThrow(
      /undefined variable `b`/,
    );
  });

  it("refuses the edit that CLOSES a cycle, before anything is stored", async () => {
    await mockClient.upsertVariable("a", 1);
    await mockClient.upsertVariableExpr("b", "a + 1");
    // `a = b + 1` would make a → b → a.
    await expect(mockClient.upsertVariableExpr("a", "b + 1")).rejects.toThrow(/variable cycle/);
    // Nothing was written: `a` is still the literal it was.
    const table = await mockClient.listVariables();
    expect(table.find((v) => v.name === "a")).toMatchObject({ value: 1, expr: undefined });
  });

  it("refuses a self-reference and an empty expression", async () => {
    await expect(mockClient.upsertVariableExpr("w", "w + 1")).rejects.toThrow(/variable cycle/);
    await expect(mockClient.upsertVariableExpr("w", "   ")).rejects.toThrow(/must not be empty/);
    await expect(mockClient.upsertVariableExpr("w", "gone * 2")).rejects.toThrow(
      /undefined variable/,
    );
    expect(await mockClient.listVariables()).toEqual([]);
  });

  /** The way BACK to a literal: `upsertVariable` clears the expression, exactly
   *  as `Scalar::try_new` does. */
  it("a literal write clears an existing expression", async () => {
    await mockClient.upsertVariable("w", 4);
    await mockClient.upsertVariableExpr("w", "8mm");
    await mockClient.upsertVariable("w", 3);
    expect(await mockClient.listVariables()).toEqual([
      { id: expect.any(String), name: "w", value: 3, resolvedValue: 3, dimension: "scalar" },
    ]);
  });

  it("reports a variable broken by a LATER edit as a row error, not a refusal", async () => {
    await mockClient.upsertVariable("w", 4);
    await mockClient.upsertVariableExpr("plate", "w * 2");
    // Removing `w` is allowed — it is `plate` that becomes unresolvable, and a
    // later breakage is a diagnostic, not a refusal of the edit in front of you.
    await mockClient.removeVariable("w");
    const plate = (await mockClient.listVariables()).find((v) => v.name === "plate");
    expect(plate?.error).toMatch(/undefined variable `w`/);
    // The last number anybody could justify is kept, with the error beside it.
    expect(plate?.resolvedValue).toBe(8);
  });
});

describe("mock lane — evaluateExpression", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  it("evaluates against the table and reports the value's own dimension", async () => {
    await mockClient.upsertVariable("w", 20);
    expect(await mockClient.evaluateExpression("w*2 + 5mm", "length")).toEqual({
      value: 45,
      dimension: "length",
    });
    // A bare number inside an expression is the SITE's canonical unit, so a
    // scalar result carries no dimension of its own.
    expect(await mockClient.evaluateExpression("w*2", "length")).toEqual({
      value: 40,
      dimension: "scalar",
    });
  });

  /** The site boundary is the whole point: an angle in a length field is a loud
   *  refusal, never a silent 45 mm. */
  it("refuses a dimension the site cannot take, as an ERROR not a rejection", async () => {
    const res = await mockClient.evaluateExpression("45deg", "length");
    expect(res.error).toMatch(/expected length/);
    expect(res.value).toBe(0);
    expect(res.dimension).toBe("length");
  });

  it("reports an unknown reference without throwing", async () => {
    expect((await mockClient.evaluateExpression("nope * 2", "length")).error).toMatch(
      /undefined variable `nope`/,
    );
  });

  it("names the real cause when a referenced variable is itself broken", async () => {
    await mockClient.upsertVariable("w", 2);
    await mockClient.upsertVariableExpr("plate", "w * 2");
    await mockClient.removeVariable("w");
    // NOT "undefined variable `plate`" — `plate` plainly exists.
    expect((await mockClient.evaluateExpression("plate + 1mm", "length")).error).toMatch(
      /variable `plate` is itself unresolved/,
    );
  });

  it("records nothing — no revision bump, no document-changed", async () => {
    const seen: number[] = [];
    const off = mockClient.onDocumentChanged((c) => seen.push(c.revision));
    await mockClient.evaluateExpression("2 + 2", "length");
    off();
    expect(seen).toEqual([]);
  });
});

describe("mock lane — renameVariable", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  it("rewrites references in other VARIABLES and in timeline scalars", async () => {
    await mockClient.upsertVariable("w", 5);
    await mockClient.upsertVariableExpr("plate", "w * 2");
    await bindF2("w + 1mm");

    const res = await mockClient.renameVariable("w", "width");

    expect(res.variables.map((v) => v.name)).toEqual(["width", "plate"]);
    expect(res.variables[1].expr).toBe("width * 2");
    // The BINDING followed: the row still reads as an expression, the new one.
    expect(res.features.find((f) => f.id === "f2")?.primaryExpr).toBe("width + 1mm");
  });

  /** Token-wise through the parser, never a substring rewrite — which would
   *  corrupt `w2` while renaming `w`. */
  it("renames only whole identifiers", async () => {
    await mockClient.upsertVariable("w", 1);
    await mockClient.upsertVariable("w2", 2);
    await mockClient.upsertVariableExpr("sum", "w + w2");
    const res = await mockClient.renameVariable("w", "width");
    expect(res.variables.find((v) => v.name === "sum")?.expr).toBe("width + w2");
  });

  it("refuses an unknown name, a duplicate, and an illegal new name", async () => {
    await mockClient.upsertVariable("w", 1);
    await mockClient.upsertVariable("h", 2);
    await expect(mockClient.renameVariable("nope", "x")).rejects.toThrow(/unknown variable/);
    await expect(mockClient.renameVariable("w", "h")).rejects.toThrow(/duplicate variable name/);
    await expect(mockClient.renameVariable("w", "2wide")).rejects.toThrow(/invalid variable name/);
    expect((await mockClient.listVariables()).map((v) => v.name)).toEqual(["w", "h"]);
  });

  it("renaming to the SAME name is a no-op, not a duplicate refusal", async () => {
    await mockClient.upsertVariable("w", 1);
    const res = await mockClient.renameVariable("w", "w");
    expect(res.variables.map((v) => v.name)).toEqual(["w"]);
  });
});
