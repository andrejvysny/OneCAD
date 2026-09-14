import { describe, expect, test } from "bun:test";
import type { SnapNode, Snapshot } from "../src/semantic/snapshot.ts";
import type { BridgeLike } from "../src/semantic/webdriver.ts";
import { POOL_REFRESHED_WARNING, poolMiss, resolveWithRefresh } from "../src/mcp/tools/targetPool.ts";

const NODE = {
  ref: "@s2e1",
  fp: "abc",
  role: "button",
  name: "Extrude",
  rect: { x: 10, y: 10, width: 40, height: 20 },
  depth: 0,
  state: {},
  css: "div > button",
  dragRegion: false,
  disabled: false,
};

/** A bridge whose only script is the snapshot; `resolve.forInput` is never reached here. */
function bridgeWith(nodes: unknown[]): BridgeLike & { snapshots: number } {
  const b = {
    snapshots: 0,
    async execute<T>(name: string): Promise<T> {
      if (name !== "ui_snapshot") throw new Error(`unexpected script ${name}`);
      b.snapshots += 1;
      return {
        rootFound: true,
        generation: 2,
        revision: 5,
        title: "t",
        viewport: { width: 100, height: 100 },
        nodes,
        total: nodes.length,
        truncated: false,
      } as T;
    },
  };
  return b;
}

describe("resolveWithRefresh", () => {
  test("a role/name miss refreshes the pool once and then resolves", async () => {
    const session = { refs: new Map<string, SnapNode>(), lastSnapshot: undefined as Snapshot | undefined };
    const bridge = bridgeWith([NODE]);
    const { resolved, refreshed } = await resolveWithRefresh(session, bridge, { role: "button", name: "Extrude" });
    expect(refreshed).toBe(true);
    expect(resolved.node?.ref).toBe("@s2e1");
    expect(bridge.snapshots).toBe(1);
    expect(session.refs.size).toBe(1);
    expect(session.lastSnapshot?.generation).toBe(2);
  });

  test("a second miss after the refresh is ELEMENT_NOT_FOUND, not a loop", async () => {
    const session = { refs: new Map<string, SnapNode>(), lastSnapshot: undefined as Snapshot | undefined };
    const bridge = bridgeWith([]);
    const err = (await resolveWithRefresh(session, bridge, { testId: "nope" }).catch((e: unknown) => e)) as { code: string };
    expect(err.code).toBe("ELEMENT_NOT_FOUND");
    expect(bridge.snapshots).toBe(1);
  });

  test("a stale ref is never retried by refreshing", async () => {
    const session = { refs: new Map<string, SnapNode>(), lastSnapshot: undefined as Snapshot | undefined };
    const bridge = bridgeWith([NODE]);
    const err = (await resolveWithRefresh(session, bridge, { ref: "@s1e9" }).catch((e: unknown) => e)) as { code: string };
    expect(err.code).toBe("ELEMENT_NOT_FOUND");
    expect(bridge.snapshots).toBe(0);
  });

  test("poolMiss classifies targets", () => {
    expect(poolMiss({ role: "button", name: "x" })).toBe(true);
    expect(poolMiss({ testId: "x" })).toBe(true);
    expect(poolMiss({ text: "x" })).toBe(true);
    expect(poolMiss({ ref: "@s1e1" })).toBe(false);
    expect(poolMiss({ css: "div" })).toBe(false);
    expect(poolMiss({ point: { x: 1, y: 1, space: "webview" } })).toBe(false);
    expect(POOL_REFRESHED_WARNING).toContain("refreshed");
  });
});
