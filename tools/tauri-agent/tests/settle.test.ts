import { describe, expect, test } from "bun:test";
import type { BridgeLike, ExecuteOpts } from "../src/semantic/webdriver.ts";
import { settle } from "../src/mcp/tools/settle.ts";

class RevBridge implements BridgeLike {
  reads = 0;
  constructor(private readonly revs: number[]) {}
  async execute<T>(_name: string, _fn: unknown, _args: unknown[], _opts: ExecuteOpts): Promise<T> {
    const rev = this.revs[Math.min(this.reads, this.revs.length - 1)] as number;
    this.reads += 1;
    return { rev, lastMutationAt: 0, now: 0 } as T;
  }
}

const nap = async (): Promise<void> => {};
const CFG = { frameMs: 1, quietMs: 1, timeoutMs: 30 };

describe("settle", () => {
  test("settles when two consecutive reads agree", async () => {
    const bridge = new RevBridge([7, 7]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.afterRevision).toBe(7);
    expect(out.warning).toBeUndefined();
    expect(bridge.reads).toBe(2);
  });

  test("keeps polling while the revision keeps moving", async () => {
    const bridge = new RevBridge([1, 2, 3, 3]);
    const out = await settle(bridge, CFG, nap);
    expect(out.settled).toBe(true);
    expect(out.afterRevision).toBe(3);
    expect(bridge.reads).toBe(4);
  });

  test("warns instead of throwing when the UI never goes quiet", async () => {
    let rev = 0;
    const bridge: BridgeLike = {
      execute: async <T,>() => {
        rev += 1;
        return { rev, lastMutationAt: 0, now: 0 } as T;
      },
    };
    const out = await settle(bridge, { frameMs: 1, quietMs: 1, timeoutMs: 5 }, (ms) => Bun.sleep(ms));
    expect(out.settled).toBe(false);
    expect(out.warning).toContain("still changing");
    expect(out.afterRevision).toBeGreaterThan(0);
  });
});
