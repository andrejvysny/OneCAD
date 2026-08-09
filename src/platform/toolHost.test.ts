/*
 * Tool host.
 *
 * The cases that matter are the ones a naive "activeTool: string" would get
 * wrong: who calls `deactivate`, what happens when the active tool's
 * registration goes away, and whether a module reporting its own state can
 * fight the host for the answer.
 */
import { describe, it, expect, vi } from "vitest";
import { createRegistry } from "./registry";
import { createToolHost } from "./toolHost";
import { addonId, contributionId, type OwnerId, type ToolId } from "./ids";
import type { ToolDefinition } from "./contributions";

const MODULE_A: OwnerId = addonId("com.example.a");
const MODULE_B: OwnerId = addonId("com.example.b");

const toolId = (owner: OwnerId, name: string): ToolId =>
  contributionId<ToolId>(owner, `${owner}.tool.${name}`);

function tool(id: ToolId, calls: string[] = []): ToolDefinition {
  return {
    id,
    title: id,
    activate: () => {
      calls.push(`activate:${id}`);
    },
    deactivate: () => {
      calls.push(`deactivate:${id}`);
    },
  };
}

function setup() {
  const tools = createRegistry<ToolDefinition>("tool");
  const host = createToolHost(tools);
  const calls: string[] = [];
  return { tools, host, calls };
}

describe("tool host", () => {
  it("activating runs the definition and publishes the id", async () => {
    const { tools, host, calls } = setup();
    const id = toolId(MODULE_A, "one");
    tools.register(MODULE_A, tool(id, calls));

    await host.activate(id);

    expect(host.activeToolId()).toBe(id);
    expect(calls).toEqual([`activate:${id}`]);
  });

  it("an unknown id is ignored rather than thrown", async () => {
    const { host } = setup();
    await expect(host.activate(toolId(MODULE_A, "ghost"))).resolves.toBeUndefined();
    expect(host.activeToolId()).toBeNull();
  });

  it("a SAME-owner swap does not deactivate — the module is already exclusive", async () => {
    const { tools, host, calls } = setup();
    const first = toolId(MODULE_A, "one");
    const second = toolId(MODULE_A, "two");
    tools.register(MODULE_A, tool(first, calls));
    tools.register(MODULE_A, tool(second, calls));

    await host.activate(first);
    await host.activate(second);

    // Modeling's `activateTool` swaps its own tools; inserting a teardown here
    // would put a step into a flow that never had one.
    expect(calls).toEqual([`activate:${first}`, `activate:${second}`]);
  });

  it("a CROSS-owner swap deactivates the outgoing tool first", async () => {
    const { tools, host, calls } = setup();
    const mine = toolId(MODULE_A, "one");
    const theirs = toolId(MODULE_B, "one");
    tools.register(MODULE_A, tool(mine, calls));
    tools.register(MODULE_B, tool(theirs, calls));

    await host.activate(mine);
    await host.activate(theirs);

    // Without this, both modules believe they hold the pointer.
    expect(calls).toEqual([`activate:${mine}`, `deactivate:${mine}`, `activate:${theirs}`]);
  });

  it("report() mirrors a module's own activation without re-running it", () => {
    const { tools, host, calls } = setup();
    const id = toolId(MODULE_A, "one");
    tools.register(MODULE_A, tool(id, calls));

    host.report(id);

    expect(host.activeToolId()).toBe(id);
    expect(calls).toEqual([]); // the module already did the work
  });

  it("report() ignores an id that is not registered", () => {
    const { host } = setup();
    host.report(toolId(MODULE_A, "ghost"));
    expect(host.activeToolId()).toBeNull();
  });

  it("disposing the active tool's owner clears the active id", async () => {
    const { tools, host, calls } = setup();
    const id = toolId(MODULE_A, "one");
    tools.register(MODULE_A, tool(id, calls));
    await host.activate(id);

    tools.disposeOwner(MODULE_A);

    // The button it was drawn as is gone; keeping it "active" would leave the
    // toolbar highlighting nothing and the next report fighting a ghost.
    expect(host.activeToolId()).toBeNull();
  });

  it("notifies subscribers on change and only on change", async () => {
    const { tools, host } = setup();
    const id = toolId(MODULE_A, "one");
    tools.register(MODULE_A, tool(id));
    const listener = vi.fn();
    const unsubscribe = host.subscribe(listener);

    await host.activate(id);
    expect(listener).toHaveBeenCalledTimes(1);

    await host.activate(id); // same tool again
    expect(listener).toHaveBeenCalledTimes(1);

    unsubscribe();
    host.report(null);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("an async activate cannot clobber a newer report", async () => {
    const { tools, host } = setup();
    const slow = toolId(MODULE_A, "slow");
    const other = toolId(MODULE_A, "other");
    tools.register(MODULE_A, {
      ...tool(slow),
      activate: async () => {
        await Promise.resolve();
      },
    });
    tools.register(MODULE_A, tool(other));

    const pending = host.activate(slow);
    host.report(other); // the module armed something else while activate awaited
    await pending;

    expect(host.activeToolId()).toBe(other);
  });
});

/*
 * State integrity (post-review hardening).
 *
 * Codex review, P2.5: `report` changed the active id without deactivating a
 * cross-owner outgoing tool, and a failed `activate` left the failed tool
 * highlighted. Both are reachable from shipped paths — a built-in tool chord
 * goes into modeling's own dispatcher and reaches the host only as a report.
 */
describe("tool host — hand-over and failure", () => {
  it("report() deactivates a cross-owner outgoing tool", () => {
    const { tools, host, calls } = setup();
    const theirs = toolId(MODULE_B, "one");
    const mine = toolId(MODULE_A, "one");
    tools.register(MODULE_B, tool(theirs, calls));
    tools.register(MODULE_A, tool(mine, calls));
    host.report(theirs);
    calls.length = 0;

    // The keyboard lane runs modeling's own dispatcher and only REPORTS the
    // result. Without a hand-over here both owners believe they are active.
    host.report(mine);

    expect(calls).toEqual([`deactivate:${theirs}`]);
    expect(host.activeToolId()).toBe(mine);
  });

  it("report() does NOT deactivate on a same-owner change", () => {
    const { tools, host, calls } = setup();
    const first = toolId(MODULE_A, "one");
    const second = toolId(MODULE_A, "two");
    tools.register(MODULE_A, tool(first, calls));
    tools.register(MODULE_A, tool(second, calls));
    host.report(first);
    calls.length = 0;

    host.report(second);

    expect(calls).toEqual([]);
  });

  it("report(null) deactivates the outgoing tool", () => {
    const { tools, host, calls } = setup();
    const id = toolId(MODULE_A, "one");
    tools.register(MODULE_A, tool(id, calls));
    host.report(id);
    calls.length = 0;

    host.report(null);

    expect(calls).toEqual([`deactivate:${id}`]);
    expect(host.activeToolId()).toBeNull();
  });

  it("a tool that throws on activate does not stay active", async () => {
    const { tools, host } = setup();
    const good = toolId(MODULE_A, "good");
    const bad = toolId(MODULE_A, "bad");
    tools.register(MODULE_A, tool(good));
    tools.register(MODULE_A, {
      ...tool(bad),
      activate: () => {
        throw new Error("cannot arm");
      },
    });
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    await host.activate(good);

    await host.activate(bad);

    // Same owner ⇒ the previous tool was never torn down ⇒ it is still the truth.
    expect(host.activeToolId()).toBe(good);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it("a CROSS-owner activation failure leaves nothing active, not a torn-down tool", async () => {
    const { tools, host } = setup();
    const mine = toolId(MODULE_A, "one");
    const theirs = toolId(MODULE_B, "one");
    tools.register(MODULE_A, tool(mine));
    tools.register(MODULE_B, {
      ...tool(theirs),
      activate: () => Promise.reject(new Error("cannot arm")),
    });
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    await host.activate(mine);

    await host.activate(theirs);

    // `mine` was deactivated by the hand-over, so claiming it is active again
    // would highlight a tool that has already torn itself down.
    expect(host.activeToolId()).toBeNull();
    error.mockRestore();
  });

  it("a failed activation does not undo a NEWER report", async () => {
    const { tools, host } = setup();
    const slow = toolId(MODULE_A, "slow");
    const other = toolId(MODULE_A, "other");
    tools.register(MODULE_A, {
      ...tool(slow),
      activate: async () => {
        await Promise.resolve();
        throw new Error("cannot arm");
      },
    });
    tools.register(MODULE_A, tool(other));
    const error = vi.spyOn(console, "error").mockImplementation(() => {});

    const pending = host.activate(slow);
    host.report(other);
    await pending;

    expect(host.activeToolId()).toBe(other);
    error.mockRestore();
  });

  it("activate() never rejects — a click handler has nothing to do with the error", async () => {
    const { tools, host } = setup();
    const bad = toolId(MODULE_A, "bad");
    tools.register(MODULE_A, {
      ...tool(bad),
      activate: () => Promise.reject(new Error("cannot arm")),
    });
    const error = vi.spyOn(console, "error").mockImplementation(() => {});

    await expect(host.activate(bad)).resolves.toBeUndefined();

    error.mockRestore();
  });

  it("passes the caller's context to activate and deactivate", async () => {
    const tools = createRegistry<ToolDefinition>("tool");
    const host = createToolHost(tools);
    const seen: unknown[] = [];
    const mine = toolId(MODULE_A, "one");
    const theirs = toolId(MODULE_B, "one");
    tools.register(MODULE_A, {
      ...tool(mine),
      deactivate: (ctx) => {
        seen.push(["deactivate", ctx]);
      },
    });
    tools.register(MODULE_B, {
      ...tool(theirs),
      activate: (ctx) => {
        seen.push(["activate", ctx]);
      },
    });
    const ctx = { selection: [], scopes: ["onecad.demo.scope.model"] };
    await host.activate(mine);

    await host.activate(theirs, ctx);

    // An addon tool that gates on the scope it was activated in cannot do so if
    // the host always hands it an empty context.
    expect(seen).toEqual([
      ["deactivate", ctx],
      ["activate", ctx],
    ]);
  });
});
