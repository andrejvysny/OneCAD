/*
 * WP0 red test — the DOF 0 contradiction.
 *
 * A sketch with DOF 0 must NEVER render as "Under-constrained", regardless of
 * the solver's internal status label. The current `constraintStatus.ts` default
 * branch returns "Under-constrained" for any non-ok status, even with DOF 0.
 */
import { describe, it, expect } from "vitest";
import { sketchStatusText, sketchStatusToneClass, sketchStatusIsAlert } from "./constraintStatus";

describe("constraintStatus WP0", () => {
  it("does not render 'Under-constrained' when DOF is 0", () => {
    const result = sketchStatusText("under", 0);
    expect(result.label).not.toMatch(/Under-constrained/i);
    expect(result.tone).toBe("ok");
  });

  it("still renders Under-constrained when DOF > 0", () => {
    const result = sketchStatusText("under", 3);
    expect(result.label).toMatch(/Under-constrained · DOF 3/);
    expect(result.tone).toBe("under");
  });

  it("renders Fully constrained when status is ok", () => {
    const result = sketchStatusText("ok", 0);
    expect(result.label).toBe("Fully constrained · DOF 0");
    expect(result.tone).toBe("ok");
  });
});

describe("constraintStatus 4-state tone (Sketcher UX cleanup)", () => {
  it("renders Over-constrained with tone 'over', not 'warn'", () => {
    const result = sketchStatusText("over", 2);
    expect(result.label).toBe("Over-constrained · DOF 2");
    expect(result.tone).toBe("over");
  });

  it("renders Conflicting with tone 'error', not 'warn'", () => {
    const result = sketchStatusText("error", 0);
    expect(result.label).toBe("Conflicting · DOF 0");
    expect(result.tone).toBe("error");
  });

  it("maps each tone to a distinct, non-alarming-for-under color class", () => {
    expect(sketchStatusToneClass("under")).toBe("text-dof-neutral");
    expect(sketchStatusToneClass("ok")).toBe("text-dof-ok");
    expect(sketchStatusToneClass("over")).toBe("text-warn");
    expect(sketchStatusToneClass("error")).toBe("text-traffic-close");
  });

  it("flags only over/error as needing an alert treatment", () => {
    expect(sketchStatusIsAlert("under")).toBe(false);
    expect(sketchStatusIsAlert("ok")).toBe(false);
    expect(sketchStatusIsAlert("over")).toBe(true);
    expect(sketchStatusIsAlert("error")).toBe(true);
  });
});
