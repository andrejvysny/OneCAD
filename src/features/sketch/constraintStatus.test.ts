/*
 * WP0 red test — the DOF 0 contradiction.
 *
 * A sketch with DOF 0 must NEVER render as "Under-constrained", regardless of
 * the solver's internal status label. The current `constraintStatus.ts` default
 * branch returns "Under-constrained" for any non-ok status, even with DOF 0.
 */
import { describe, it, expect } from "vitest";
import { sketchStatusText } from "./constraintStatus";

describe("constraintStatus WP0", () => {
  it("does not render 'Under-constrained' when DOF is 0", () => {
    const result = sketchStatusText("under", 0);
    expect(result.label).not.toMatch(/Under-constrained/i);
    expect(result.tone).toBe("ok");
  });

  it("still renders Under-constrained when DOF > 0", () => {
    const result = sketchStatusText("under", 3);
    expect(result.label).toMatch(/Under-constrained · DOF 3/);
    expect(result.tone).toBe("warn");
  });

  it("renders Fully constrained when status is ok", () => {
    const result = sketchStatusText("ok", 0);
    expect(result.label).toBe("Fully constrained · DOF 0");
    expect(result.tone).toBe("ok");
  });
});
