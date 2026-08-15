import { describe, it, expect } from "vitest";
import { settleUntil } from "./settle";

/**
 * These tests are the load-bearing proof for `settleUntil`. Without them the
 * helper could degrade into a single flush and nothing would notice, because
 * the two call sites it serves pass on an idle machine either way — that is
 * exactly how the flake it replaces stayed invisible locally.
 */
describe("settleUntil", () => {
  /** Becomes true only after `n` macrotask turns have actually elapsed. */
  function afterTurns(n: number): () => boolean {
    let seen = 0;
    const tick = (): void => {
      seen += 1;
      if (seen < n) setTimeout(tick, 0);
    };
    setTimeout(tick, 0);
    return () => seen >= n;
  }

  it("returns as soon as the assertion holds, without spending a turn", async () => {
    let calls = 0;
    await settleUntil(() => {
      calls += 1;
    });
    expect(calls).toBe(1);
  });

  it("waits across MULTIPLE turns — one flush is not enough", async () => {
    const ready = afterTurns(5);
    await settleUntil(() => expect(ready()).toBe(true));
    expect(ready()).toBe(true);
  });

  it("NEUTRALIZED: with a single turn the same condition fails", async () => {
    // The complement of the test above. If `settleUntil` ever collapses to one
    // flush, the test above starts failing and this one starts passing for the
    // wrong reason — so they are asserted as a pair.
    const ready = afterTurns(5);
    await expect(settleUntil(() => expect(ready()).toBe(true), { turns: 1 })).rejects.toThrow();
  });

  it("surfaces the real assertion error when it runs out of turns", async () => {
    await expect(
      settleUntil(() => expect("actual").toBe("expected"), { turns: 2 }),
    ).rejects.toThrow(/expected/);
  });

  it("uses the caller's turn function", async () => {
    let turns = 0;
    const ready = afterTurns(3);
    await settleUntil(() => expect(ready()).toBe(true), {
      turn: async () => {
        turns += 1;
        await new Promise((resolve) => setTimeout(resolve, 0));
      },
    });
    expect(turns).toBeGreaterThan(0);
  });
});
