/*
 * The PROJECTION_STALE derivation (WP-P B4) — the banner's only input.
 */
import { describe, it, expect } from "vitest";
import { PROJECTION_STALE_CODE } from "@/ipc/types";
import type { FeatureMeta } from "@/stores/documentStore";
import { projectionStaleVerdict } from "./projectionStale";

const feature = (over: Partial<FeatureMeta> = {}): FeatureMeta => ({
  id: "rec1",
  kind: "sketch",
  label: "Sketch",
  valueText: "",
  status: "ok",
  ...over,
});

const staleDiag = (message: string) => ({
  severity: "warning" as const,
  code: PROJECTION_STALE_CODE,
  message,
  reasonCode: PROJECTION_STALE_CODE,
});

describe("projectionStaleVerdict", () => {
  it("is null with no diagnostics at all", () => {
    expect(projectionStaleVerdict([feature()])).toBeNull();
  });

  it("is null for a non-projection diagnostic", () => {
    const f = feature({
      diagnostics: [{ severity: "error", code: "EXPR_UNRESOLVED", message: "boom" }],
    });
    expect(projectionStaleVerdict([f])).toBeNull();
  });

  it("reads the count off the backend's own sentence", () => {
    const f = feature({
      diagnostics: [staleDiag("3 projected entities in this sketch no longer match their source")],
    });
    expect(projectionStaleVerdict([f])).toEqual({
      count: 3,
      entityIds: [],
      message: "3 projected entities in this sketch no longer match their source",
    });
  });

  it("prefers the structured evidence: entityIds drive the count and sketchId attributes it", () => {
    const f = feature({
      diagnostics: [
        { ...staleDiag("stale"), evidence: { sketchId: "sk-A", entityIds: ["e1", "e2"] } },
      ],
    });
    expect(projectionStaleVerdict([f], "sk-A")).toEqual({
      count: 2,
      entityIds: ["e1", "e2"],
      message: "stale",
    });
    // Another open sketch does not inherit sketch A's verdict.
    expect(projectionStaleVerdict([f], "sk-B")).toBeNull();
    // Without an active sketch the verdict is reported regardless.
    expect(projectionStaleVerdict([f])?.count).toBe(2);
  });

  it("reports count 0 rather than inventing one when the message has no leading number", () => {
    const f = feature({ diagnostics: [staleDiag("projections are stale")] });
    expect(projectionStaleVerdict([f])?.count).toBe(0);
  });

  it("ignores the diagnostic on a NON-sketch row (it can only ride a sketch step)", () => {
    const f = feature({ kind: "extrude", diagnostics: [staleDiag("2 projected entities")] });
    expect(projectionStaleVerdict([f])).toBeNull();
  });
});
