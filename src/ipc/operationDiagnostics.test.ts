import { describe, expect, it } from "vitest";
import { parseOperationDiagnostics } from "./operationDiagnostics";

describe("parseOperationDiagnostics", () => {
  it("keeps valid bounded evidence and ignores malformed additive detail", () => {
    expect(
      parseOperationDiagnostics([
        {
          severity: "error",
          code: "FILLET_WALKING_FAILED",
          message: "failed",
          stage: "build",
          evidence: { metrics: { requestedRadius: 11 } },
        },
        { severity: "error", code: 7, message: "bad" },
      ]),
    ).toEqual([
      {
        severity: "error",
        code: "FILLET_WALKING_FAILED",
        message: "failed",
        stage: "build",
        evidence: { metrics: { requestedRadius: 11 } },
      },
    ]);
  });

  it("treats malformed optional evidence as absent", () => {
    expect(
      parseOperationDiagnostics([
        { severity: "warning", code: "X", message: "usable", evidence: [] },
      ]),
    ).toEqual([{ severity: "warning", code: "X", message: "usable" }]);
    expect(parseOperationDiagnostics("bad")).toBeUndefined();
  });

  it("enforces carrier and text bounds without rejecting usable entries", () => {
    const valid = { severity: "info", code: "OK", message: "usable" };
    expect(parseOperationDiagnostics(Array.from({ length: 65 }, () => valid))).toHaveLength(64);
    expect(
      parseOperationDiagnostics([
        { ...valid, code: "x".repeat(129) },
        {
          ...valid,
          stage: "x".repeat(65),
          evidence: { value: "x".repeat(65_537) },
        },
      ]),
    ).toEqual([valid]);
  });
});
