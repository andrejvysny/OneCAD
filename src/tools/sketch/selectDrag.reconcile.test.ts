/*
 * Select-drag reconcile against the MOCK client's gesture lane (localSolver):
 * begin → solveDrag (latest-wins by seq) → endGesture (ONE commit). Verifies the
 * controller's reconcile contract without the WebGL controller: the seq gate drops
 * stale/null responses, partial-delta positions apply onto the pre-drag base, and
 * endGesture commits exactly once.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { mockClient } from "@/ipc/mockClient";
import { applySolvedPositions } from "@/ipc/sketchWireMap";
import { shouldApplyDrag } from "./selectGesture";
import type { SketchEntity } from "@/ipc/types";

/** A line whose Start we drag; End must stay put (partial delta). */
const base: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [10, 0] }];

afterEach(() => vi.restoreAllMocks());

describe("select drag reconcile (mock gesture lane)", () => {
  it("begin → solve (latest-wins) → end applies the committed delta once", async () => {
    const endSpy = vi.spyOn(mockClient, "endGesture");

    const begin = await mockClient.beginGesture("s1", "e1.Start");
    expect(begin.ready).toBe(true);

    let lastSeq = 0;
    let live = base;

    // First solve → apply onto the base (Start moves, End untouched = partial delta).
    const r1 = await mockClient.solveDrag([2, 0]);
    expect(shouldApplyDrag(lastSeq, r1)).toBe(true);
    lastSeq = r1!.seq;
    live = applySolvedPositions(base, r1!.positions);
    expect(live[0].p0).toEqual([2, 0]);
    expect(live[0].p1).toEqual([10, 0]); // partial: the other endpoint is unchanged

    // Second solve supersedes the first.
    const r2 = await mockClient.solveDrag([5, 1]);
    expect(shouldApplyDrag(lastSeq, r2)).toBe(true);
    lastSeq = r2!.seq;
    live = applySolvedPositions(base, r2!.positions);
    expect(live[0].p0).toEqual([5, 1]);

    // A stale (older-seq) response is dropped by the gate — never clobbers r2.
    expect(shouldApplyDrag(lastSeq, r1)).toBe(false);
    // A null response (client already dropped it) is dropped too.
    expect(shouldApplyDrag(lastSeq, null)).toBe(false);

    // Pointer-up commit: one endGesture, final positions applied.
    const end = await mockClient.endGesture([9, 2]);
    const final = applySolvedPositions(base, end.solvedPositions ?? {});
    expect(final[0].p0).toEqual([9, 2]);
    expect(final[0].p1).toEqual([10, 0]);

    expect(endSpy).toHaveBeenCalledTimes(1);
    // The base array is never mutated (immutable apply).
    expect(base[0].p0).toEqual([0, 0]);
  });

  it("endGesture with no final target commits no movement (Esc-cancel restore path)", async () => {
    await mockClient.beginGesture("s1", "e1.Start");
    await mockClient.solveDrag([7, 7]); // a live preview happened...
    const end = await mockClient.endGesture(); // ...but Esc ends with no target
    // The restore path applies {} onto the base → identical geometry (same ref).
    const restored = applySolvedPositions(base, end.solvedPositions ?? {});
    expect(restored).toBe(base);
  });
});
