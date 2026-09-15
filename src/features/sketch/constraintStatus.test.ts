/*
 * WP0 red test — the DOF 0 contradiction.
 *
 * A sketch with DOF 0 must NEVER render as "Under-constrained", regardless of
 * the solver's internal status label. The current `constraintStatus.ts` default
 * branch returns "Under-constrained" for any non-ok status, even with DOF 0.
 */
import { describe, it, expect } from "vitest";
import {
  sketchStatusText,
  sketchStatusSentence,
  sketchStatusToneClass,
  sketchStatusIsAlert,
  emptySketchCard,
  projectedOnlySketchCard,
  consumingFeatureLabel,
} from "./constraintStatus";

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
  it("names the benign-redundancy state 'Redundant constraint', tone 'over' (S11)", () => {
    const result = sketchStatusText("over", 2);
    expect(result.label).toBe("Redundant constraint · DOF 2");
    expect(result.tone).toBe("over");
    // The old wording read as the same failure as Conflicting; it must be gone.
    expect(result.label).not.toMatch(/Over-constrained/);
  });

  it("renders Conflicting with tone 'error', not 'warn'", () => {
    const result = sketchStatusText("error", 0);
    expect(result.label).toBe("Conflicting · DOF 0");
    expect(result.tone).toBe("error");
  });

  it("explains redundancy as a cleanup, not a failure (S11)", () => {
    expect(sketchStatusSentence("over", 0)).toBe(
      "A constraint repeats one already implied — remove it to keep the sketch clean.",
    );
  });

  it("names the blamed constraints when the solver reported culprits (S11)", () => {
    expect(sketchStatusSentence("error", 0, ["Horizontal · Line 2"])).toBe(
      "1 constraint cannot be met: Horizontal · Line 2",
    );
    expect(
      sketchStatusSentence("error", 0, ["Horizontal · Line 2", "Distance 30 mm · Line 1"]),
    ).toBe("2 constraints cannot be met: Horizontal · Line 2, Distance 30 mm · Line 1");
  });

  it("falls back to the unspecific conflict sentence when no culprit is known", () => {
    expect(sketchStatusSentence("error", 0)).toBe(
      "Conflicting constraints. Remove one to resolve.",
    );
    expect(sketchStatusSentence("error", 0, [])).toBe(
      "Conflicting constraints. Remove one to resolve.",
    );
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

describe("emptySketchCard (design item 12 / audit A11a)", () => {
  it("reads as a neutral 'nothing drawn yet' state, not a false completeness claim", () => {
    const card = emptySketchCard();
    expect(card.label).toBe("Empty sketch");
    expect(card.sentence).toBe("Draw geometry to begin.");
    // "under" — the same tone `sketchStatusToneClass` maps to the neutral
    // color, so this never reads as an alert (`sketchStatusIsAlert`).
    expect(card.tone).toBe("under");
    expect(sketchStatusToneClass(card.tone)).toBe("text-dof-neutral");
    expect(sketchStatusIsAlert(card.tone)).toBe(false);
  });
});

describe("projectedOnlySketchCard (UX review 2026-09-11)", () => {
  it("never claims completeness for a sketch that only holds projected references", () => {
    const card = projectedOnlySketchCard(11);
    expect(card.label).toBe("Projected geometry only");
    expect(card.sentence).toBe("11 projected reference edges · draw geometry to begin.");
    expect(sketchStatusIsAlert(card.tone)).toBe(false);
    expect(projectedOnlySketchCard(1).sentence).toBe("1 projected reference edge · draw geometry to begin.");
  });
});

/*
 * UX review 2026-09-14 — N10: a sketch read "Not evaluated" after a cancelled
 * edit although a completed Extrude was standing on it (A-136). "Not evaluated"
 * is only honest for a sketch nothing in the timeline consumes.
 */
describe("consumingFeatureLabel (UX review 2026-09-14, N10)", () => {
  const timeline = [
    { id: "f1", kind: "sketch", label: "Sketch 1", status: "ok" },
    { id: "f2", kind: "extrude", label: "Extrude", status: "ok" },
    { id: "f3", kind: "fillet", label: "Fillet", status: "ok" },
    { id: "f4", kind: "sketch", label: "Sketch 2", status: "ok" },
    { id: "f5", kind: "extrude", label: "Extrude 2", status: "ok" },
  ];

  it("names the first applied feature standing on the sketch's own row", () => {
    expect(consumingFeatureLabel(timeline, "f1", 5)).toBe("Extrude");
    expect(consumingFeatureLabel(timeline, "f4", 5)).toBe("Extrude 2");
  });

  it("claims nothing for a sketch row with no row of its own to stand on", () => {
    expect(consumingFeatureLabel(timeline, null, 5)).toBeNull();
    expect(consumingFeatureLabel(timeline, "unknown", 5)).toBeNull();
  });

  it("stops at the next sketch rather than borrowing a later sketch's feature", () => {
    const lonely = [
      { id: "f1", kind: "sketch", label: "Sketch 1", status: "ok" },
      { id: "f4", kind: "sketch", label: "Sketch 2", status: "ok" },
      { id: "f5", kind: "extrude", label: "Extrude 2", status: "ok" },
    ];
    expect(consumingFeatureLabel(lonely, "f1", 3)).toBeNull();
  });

  it("ignores rolled-back and failed features", () => {
    expect(consumingFeatureLabel(timeline, "f1", 1)).toBeNull();
    expect(
      consumingFeatureLabel(
        [timeline[0], { ...timeline[1], status: "error" }, timeline[2]],
        "f1",
        3,
      ),
    ).toBe("Fillet");
  });
});
