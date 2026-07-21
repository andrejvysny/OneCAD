import { describe, it, expect, beforeEach } from "vitest";
import { sketchStore } from "./sketchStore";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchSession } from "@/ipc/types";

function session(conflicting?: string[]): SketchSession {
  return {
    sketchId: "sk",
    plane: planeFor("XY"),
    entities: [],
    constraints: [],
    dof: 0,
    status: "UnderConstrained",
    conflicting,
  };
}

describe("sketchStore.conflictingIds — ONE-owner reducer (SCHEMA §7.4)", () => {
  beforeEach(() => sketchStore.getState().reset());

  it("defaults to [] and reset() clears it", () => {
    expect(sketchStore.getState().conflictingIds).toEqual([]);
    sketchStore.getState().setConflicting(["a"]);
    sketchStore.getState().reset();
    expect(sketchStore.getState().conflictingIds).toEqual([]);
  });

  it("setConflicting REPLACES the set", () => {
    sketchStore.getState().setConflicting(["a", "b"]);
    expect(sketchStore.getState().conflictingIds).toEqual(["a", "b"]);
    sketchStore.getState().setConflicting(["c"]);
    expect(sketchStore.getState().conflictingIds).toEqual(["c"]);
  });

  it("setConflicting short-circuits when unchanged (same array reference)", () => {
    sketchStore.getState().setConflicting(["a", "b"]);
    const ref = sketchStore.getState().conflictingIds;
    sketchStore.getState().setConflicting(["a", "b"]); // equal ids
    expect(sketchStore.getState().conflictingIds).toBe(ref); // no re-alloc ⇒ no churn
  });

  it("setSession(null) CLEARS conflictingIds (exit / dispose / session-swap)", () => {
    sketchStore.getState().setSession(session());
    sketchStore.getState().setConflicting(["a"]);
    sketchStore.getState().setSession(null);
    expect(sketchStore.getState().conflictingIds).toEqual([]);
  });

  it("a non-null setSession does NOT touch conflictingIds (the write-back owns it)", () => {
    sketchStore.getState().setConflicting(["a"]);
    sketchStore.getState().setSession(session()); // a live solve write-back setSession
    expect(sketchStore.getState().conflictingIds).toEqual(["a"]);
  });
});
