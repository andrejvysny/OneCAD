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

describe("sketchStore.entityStates — ONE-owner reducer (SCHEMA §7.4)", () => {
  beforeEach(() => sketchStore.getState().reset());

  it("defaults to {} and reset() clears it", () => {
    expect(sketchStore.getState().entityStates).toEqual({});
    sketchStore.getState().setEntityStates({ e1: "conflicting" });
    sketchStore.getState().reset();
    expect(sketchStore.getState().entityStates).toEqual({});
  });

  it("setEntityStates REPLACES the map — a key the new solve dropped is GONE", () => {
    sketchStore.getState().setEntityStates({ e1: "fullyConstrained", e2: "conflicting" });
    sketchStore.getState().setEntityStates({ e2: "underConstrained" });
    // Not merged: e1 fell out of the map, which means UNKNOWN, and a merge would
    // keep asserting a diagnosis the newest solve no longer makes.
    expect(sketchStore.getState().entityStates).toEqual({ e2: "underConstrained" });
  });

  it("short-circuits when unchanged (same object reference)", () => {
    sketchStore.getState().setEntityStates({ e1: "fullyConstrained", e2: "conflicting" });
    const ref = sketchStore.getState().entityStates;
    // A different object, equal content — an echoed gesture map, or any identity solve.
    sketchStore.getState().setEntityStates({ e2: "conflicting", e1: "fullyConstrained" });
    expect(sketchStore.getState().entityStates).toBe(ref);
  });

  it("does NOT short-circuit when a value changed under the same keys", () => {
    sketchStore.getState().setEntityStates({ e1: "underConstrained" });
    const ref = sketchStore.getState().entityStates;
    sketchStore.getState().setEntityStates({ e1: "conflicting" });
    expect(sketchStore.getState().entityStates).not.toBe(ref);
    expect(sketchStore.getState().entityStates).toEqual({ e1: "conflicting" });
  });

  it("does NOT short-circuit when a key was added or removed", () => {
    sketchStore.getState().setEntityStates({ e1: "underConstrained" });
    sketchStore.getState().setEntityStates({ e1: "underConstrained", e2: "underConstrained" });
    expect(sketchStore.getState().entityStates).toEqual({ e1: "underConstrained", e2: "underConstrained" });
    sketchStore.getState().setEntityStates({});
    expect(sketchStore.getState().entityStates).toEqual({});
  });

  it("setSession(null) CLEARS entityStates (exit / dispose / session-swap)", () => {
    sketchStore.getState().setSession(session());
    sketchStore.getState().setEntityStates({ e1: "conflicting" });
    sketchStore.getState().setSession(null);
    expect(sketchStore.getState().entityStates).toEqual({});
  });

  it("a non-null setSession does NOT touch entityStates (the write-back owns it)", () => {
    sketchStore.getState().setEntityStates({ e1: "conflicting" });
    sketchStore.getState().setSession(session());
    expect(sketchStore.getState().entityStates).toEqual({ e1: "conflicting" });
  });
});
