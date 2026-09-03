/*
 * The project-edges accumulator, pure (WP-P). The controller test covers the
 * pointer/keyboard ladder; this covers what the picks ADD UP TO.
 */
import { describe, it, expect } from "vitest";
import {
  projectHint,
  projectInit,
  projectRequests,
  projectSummary,
  projectToggle,
  type ProjectPick,
} from "./projectTool";

const pick = (
  topoKey: string,
  kind: ProjectPick["kind"] = "edge",
  bodyId = "body1",
): ProjectPick => ({
  bodyId,
  kind,
  topoKey,
  worldPoint: [1, 2, 3],
});

describe("projectTool — pick accumulation", () => {
  it("accumulates picks in click order", () => {
    let state = projectInit();
    state = projectToggle(state, pick("e:0"));
    state = projectToggle(state, pick("e:1"));
    expect(state.picks.map((p) => p.topoKey)).toEqual(["e:0", "e:1"]);
  });

  it("a SECOND click on a held pick drops it (unpick)", () => {
    let state = projectInit();
    state = projectToggle(state, pick("e:0"));
    state = projectToggle(state, pick("e:1"));
    state = projectToggle(state, pick("e:0"));
    expect(state.picks.map((p) => p.topoKey)).toEqual(["e:1"]);
  });

  it("the same topoKey on ANOTHER body is a different pick", () => {
    let state = projectInit();
    state = projectToggle(state, pick("f:0", "face", "body1"));
    state = projectToggle(state, pick("f:0", "face", "body2"));
    expect(state.picks).toHaveLength(2);
  });

  it("names the pick count in the hint", () => {
    expect(projectHint(projectInit())).toMatch(/click a body edge or face/);
    expect(projectHint(projectToggle(projectInit(), pick("e:0")))).toMatch(/1 picked/);
  });
});

describe("projectTool — requests", () => {
  it("groups by (body, mode) and carries every pick's world anchor", () => {
    let state = projectInit();
    state = projectToggle(state, pick("e:0"));
    state = projectToggle(state, pick("f:2", "face"));
    state = projectToggle(state, pick("e:5"));

    const reqs = projectRequests(state, "sk1");
    expect(reqs).toHaveLength(2);
    // First-pick order: the edge group opened first.
    expect(reqs[0].mode).toBe("edges");
    expect(reqs[0].sketchId).toBe("sk1");
    expect(reqs[0].sources.map((s) => s.topoKey)).toEqual(["e:0", "e:5"]);
    expect(reqs[1].mode).toBe("faceOutline");
    expect(reqs[1].sources.map((s) => s.topoKey)).toEqual(["f:2"]);
    for (const req of reqs) {
      for (const source of req.sources) {
        expect(source.anchor?.worldPoint).toEqual([1, 2, 3]);
      }
    }
  });

  it("splits two bodies into separate single-body requests", () => {
    let state = projectInit();
    state = projectToggle(state, pick("e:0", "edge", "body1"));
    state = projectToggle(state, pick("e:0", "edge", "body2"));
    const reqs = projectRequests(state, "sk1");
    expect(reqs.map((r) => r.sources[0].bodyId)).toEqual(["body1", "body2"]);
  });

  it("omits snapshotId so the transport fills in its own head", () => {
    const reqs = projectRequests(projectToggle(projectInit(), pick("e:0")), "sk1");
    expect(reqs[0].snapshotId).toBeUndefined();
  });
});

describe("projectTool — batch summary", () => {
  const refusal = (message: string) => ({
    bodyId: "body1",
    elementId: "el",
    topoKey: "e:9",
    code: "unsupportedCurve",
    message,
  });

  it("reports a clean batch as info", () => {
    expect(projectSummary(3, [])).toEqual({
      message: "Projected 3 sources into the sketch",
      severity: "info",
    });
  });

  it("reports ONE line for the whole batch, naming the first refusal", () => {
    const out = projectSummary(3, [refusal("MOCK LIMIT: curved"), refusal("also no")]);
    expect(out.message).toBe("Projected 1 of 3; 2 refused: MOCK LIMIT: curved");
    expect(out.severity).toBe("error");
  });
});
