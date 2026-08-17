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
    expect(res.variables).toEqual([{ id: expect.any(String), name: "height", value: 40 }]);
  });

  it("a value that drives a bound extrude below the kernel floor FAILS — and the variable is still saved", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2To("height");

    const res = await mockClient.upsertVariable("height", 0);

    // Truth 1: the write landed. Not reverted, not rethrown.
    expect(res.variables).toEqual([{ id: expect.any(String), name: "height", value: 0 }]);
    expect(await mockClient.listVariables()).toEqual([
      { id: expect.any(String), name: "height", value: 0 },
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
    // Mirrors `regen::variables::substitute_variables`'s `${field}: ${reason}`.
    expect(res.errorMessage).toMatch(/Extrude\.distance: variable `height` is not defined/);
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
