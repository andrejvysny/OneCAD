/*
 * Mutation coordinator (C1) — generation fencing + queue flush. A response that
 * lands after a newer setSession (bumped generation) is DROPPED, and
 * flushSketchMutations resolves only once the in-flight upsert settles.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { editConstraintValue, flushSketchMutations } from "./sketchService";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import type { CadClient } from "@/ipc/client";
import type { SketchSession, SketchUpsertResult } from "@/ipc/types";

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function session(id: string): SketchSession {
  return {
    sketchId: id,
    plane: planeFor("XY"),
    entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
    constraints: [{ id: "c1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 }],
    dof: 1,
    status: "UnderConstrained",
  };
}

const okResult = (id: string): SketchUpsertResult => ({
  sketchId: id,
  sketchRevision: 1,
  dof: 1,
  status: "UnderConstrained",
  solvedPositions: {},
});

/** A client whose sketchUpsert is a single manually-resolved deferred promise. */
function deferredClient() {
  let resolve!: (r: SketchUpsertResult) => void;
  const client = {
    sketchUpsert: () => new Promise<SketchUpsertResult>((res) => (resolve = res)),
  } as unknown as CadClient;
  return { client, resolve: () => resolve };
}

describe("generation fencing", () => {
  beforeEach(() => sketchStore.getState().reset());

  it("drops a response that arrives after a newer session bumped the generation", async () => {
    const A = session("sk-A");
    sketchStore.getState().setSession(A);
    const { client, resolve } = deferredClient();

    const p = editConstraintValue(client, "c1", 99);
    await flush(); // the queued turn is now parked at its upsert await

    // A newer session supersedes the in-flight edit (generation bumps).
    const B = session("sk-B");
    sketchStore.getState().setSession(B);

    resolve()(okResult("sk-A")); // the stale upsert finally resolves
    await p;

    // The mutator saw the generation change and dropped its write — B is intact.
    expect(sketchStore.getState().session).toBe(B);
  });
});

describe("flushSketchMutations", () => {
  beforeEach(() => sketchStore.getState().reset());

  it("resolves only after the queued upsert settles", async () => {
    sketchStore.getState().setSession(session("sk-A"));
    const { client, resolve } = deferredClient();

    const editP = editConstraintValue(client, "c1", 5);
    let flushed = false;
    const flushP = flushSketchMutations().then(() => {
      flushed = true;
    });

    await flush();
    expect(flushed).toBe(false); // upsert still pending ⇒ flush blocked

    resolve()(okResult("sk-A"));
    await editP;
    await flushP;
    expect(flushed).toBe(true);
  });
});
