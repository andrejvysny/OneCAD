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
    expect(after.map((v) => v.name)).toEqual(["width", "height"]);
    expect(after[0].value).toBe(15);
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
    expect(table.map((v) => v.name)).toContain("padded");
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
    expect(await mockClient.removeVariable("w")).toEqual([]);
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
