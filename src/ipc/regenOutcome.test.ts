import { describe, expect, it } from "vitest";

import { classifyRegen, failureReason, keepsRecord } from "./regenOutcome";
import type { ApplyOperationResult, RegenTerminal } from "./types";

function result(over: Partial<ApplyOperationResult> = {}): ApplyOperationResult {
  return { revision: 1, changedBodies: [], removedBodies: [], features: [], ...over };
}

const body = { bodyId: "body1", meshKey: "body1" };

describe("classifyRegen", () => {
  it("maps every terminal to its own verdict", () => {
    const expected: Record<RegenTerminal, string> = {
      published: "published",
      noop: "noop",
      needsRepair: "needsRepair",
      failed: "failed",
      timeout: "timeout",
    };
    for (const [terminal, kind] of Object.entries(expected)) {
      expect(classifyRegen(result({ terminal: terminal as RegenTerminal })).kind).toBe(kind);
    }
  });

  it("treats a resolved errorMessage as failure whatever the terminal claims", () => {
    const outcome = classifyRegen(
      result({ terminal: "published", changedBodies: [body], errorMessage: "kernel refused" }),
    );
    expect(outcome).toEqual({ kind: "failed", reason: "kernel refused", diagnostics: undefined });
  });

  // The two cases body counting got backwards.
  it("does not call an empty published result a failure", () => {
    expect(classifyRegen(result({ terminal: "published" })).kind).toBe("published");
  });

  it("does not call a delete-only result a failure", () => {
    const outcome = classifyRegen(result({ terminal: "published", removedBodies: ["body1"] }));
    expect(outcome.kind).toBe("published");
  });

  it("does not call needsRepair a success", () => {
    const outcome = classifyRegen(result({ terminal: "needsRepair" }));
    expect(outcome.kind).toBe("needsRepair");
    // …and it is not a failure either: the record must survive for repair to act on.
    expect(keepsRecord(outcome)).toBe(true);
    expect(failureReason(outcome)).toBeNull();
  });

  it("always supplies a reason, so no caller has to invent one", () => {
    expect(failureReason(classifyRegen(null))).toBe("the operation returned no result");
    expect(failureReason(classifyRegen(result({ terminal: "failed" })))).toBe(
      "the operation failed without a reason",
    );
    expect(failureReason(classifyRegen(result({ terminal: "timeout" })))).toBe(
      "the operation did not complete in time",
    );
  });

  it("carries diagnostics through a failure", () => {
    const diagnostics = [
      { severity: "error" as const, code: "FILLET_WALKING_FAILED", message: "boom" },
    ];
    const outcome = classifyRegen(result({ errorMessage: "boom", diagnostics }));
    expect(outcome).toMatchObject({ kind: "failed", diagnostics });
  });

  describe("without a terminal, the legacy inference still applies verbatim", () => {
    it("empty is a failure", () => {
      expect(classifyRegen(result()).kind).toBe("failed");
      expect(failureReason(classifyRegen(result()))).toBe("no body changed");
    });

    it("non-empty is a publish", () => {
      expect(classifyRegen(result({ changedBodies: [body] })).kind).toBe("published");
    });
  });
});

describe("keepsRecord", () => {
  it("is true exactly for the outcomes that leave a record behind", () => {
    const kept: RegenTerminal[] = ["published", "noop", "needsRepair"];
    const dropped: RegenTerminal[] = ["failed", "timeout"];
    for (const terminal of kept) {
      expect(keepsRecord(classifyRegen(result({ terminal })))).toBe(true);
    }
    for (const terminal of dropped) {
      expect(keepsRecord(classifyRegen(result({ terminal })))).toBe(false);
    }
  });
});
