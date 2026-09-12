import { describe, expect, it } from "vitest";
import { completionVerb } from "./completionHint";

describe("completionVerb", () => {
  it("Cut, single body", () => {
    expect(completionVerb("extrude", "Cut", 1)).toBe("Cut created");
  });

  it("Cut, multiple bodies", () => {
    expect(completionVerb("extrude", "Cut", 3)).toBe("Cut created — 3 bodies");
  });

  it("Add (Join), single body", () => {
    expect(completionVerb("extrude", "Add", 1)).toBe("Joined");
  });

  it("Add (Join), multiple bodies", () => {
    expect(completionVerb("revolve", "Add", 2)).toBe("Joined 2 bodies");
  });

  it("Intersect, single body", () => {
    expect(completionVerb("extrude", "Intersect", 1)).toBe("Intersected");
  });

  it("Intersect, multiple bodies", () => {
    expect(completionVerb("extrude", "Intersect", 2)).toBe("Intersected — 2 bodies");
  });

  it("NewBody, single body", () => {
    expect(completionVerb("extrude", "NewBody", 1)).toBe("New body created");
    expect(completionVerb("revolve", "NewBody", 1)).toBe("New body created");
  });

  it("NewBody, multiple bodies", () => {
    expect(completionVerb("extrude", "NewBody", 2)).toBe("2 bodies created");
  });

  it("every mapped hint stays within the 60-char status-bar budget", () => {
    const modes = ["Cut", "Add", "Intersect", "NewBody"] as const;
    for (const mode of modes) {
      for (const count of [1, 2, 12]) {
        expect(completionVerb("extrude", mode, count).length).toBeLessThanOrEqual(60);
      }
    }
  });
});
