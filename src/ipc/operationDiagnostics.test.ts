import { describe, expect, it } from "vitest";
import { diagnosticHint, parseOperationDiagnostics } from "./operationDiagnostics";
import type { OperationDiagnostic } from "./types";

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

  it("keeps a bounded `reasonCode` (kernel-hardening WP-I; SCHEMA §7.2 diagnostics[].reasonCode)", () => {
    expect(
      parseOperationDiagnostics([
        {
          severity: "error",
          code: "REF_UNRESOLVED",
          message: "refused",
          reasonCode: "GEAR_FACE_NOT_REFERENCEABLE",
        },
      ]),
    ).toEqual([
      {
        severity: "error",
        code: "REF_UNRESOLVED",
        message: "refused",
        reasonCode: "GEAR_FACE_NOT_REFERENCEABLE",
      },
    ]);
    // An oversized reasonCode is dropped, not the whole diagnostic.
    const oversized = parseOperationDiagnostics([
      { severity: "error", code: "OP_FAILED", message: "m", reasonCode: "x".repeat(129) },
    ]);
    expect(oversized?.[0].reasonCode).toBeUndefined();
  });
});

describe("diagnosticHint — WP-I evidence-driven text for gear/generator reasonCodes", () => {
  function diag(over: Partial<OperationDiagnostic>): OperationDiagnostic {
    return { severity: "error", code: "OP_FAILED", message: "backend message", ...over };
  }

  it("names the gear face refusal without echoing the raw backend message", () => {
    expect(
      diagnosticHint(diag({ code: "REF_UNRESOLVED", reasonCode: "GEAR_FACE_NOT_REFERENCEABLE" })),
    ).toBe("This gear face is tooth geometry and cannot be referenced — pick the bore or a cap");
  });

  it("names the param, value and bound for a gear param out of range", () => {
    expect(
      diagnosticHint(
        diag({
          reasonCode: "GEAR_PARAM_OUT_OF_RANGE",
          evidence: { param: "teeth", value: 401, min: 3, max: 400 },
        }),
      ),
    ).toBe("teeth must be between 3 and 400 (got 401)");
  });

  it("names the param, value and bound for a generator param out of range", () => {
    expect(
      diagnosticHint(
        diag({
          reasonCode: "GENERATOR_PARAM_OUT_OF_RANGE",
          evidence: { param: "length", value: 1_000_000, max: 1000 },
        }),
      ),
    ).toBe("length must be at most 1000 (got 1000000)");
  });

  it("falls back to the backend message for an unmapped or missing reasonCode", () => {
    expect(diagnosticHint(diag({}))).toBe("backend message");
    expect(diagnosticHint(diag({ reasonCode: "SOME_FUTURE_CODE" }))).toBe("backend message");
  });
});
