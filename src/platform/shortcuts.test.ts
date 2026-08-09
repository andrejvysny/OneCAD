/*
 * Shortcut resolution.
 *
 * The cases worth pinning are the ambiguous ones: two contributions claiming one
 * chord must resolve the same way on every boot, and when nothing separates them
 * the chord must do NOTHING rather than pick a winner by load order.
 */
import { describe, it, expect, vi } from "vitest";
import { createRegistry } from "./registry";
import { createShortcutService } from "./shortcuts";
import { createToolHost } from "./toolHost";
import { addonId, contributionId, moduleId, type CommandId, type OwnerId, type ToolId } from "./ids";
import type { CommandDefinition, Shortcut, ToolDefinition } from "./contributions";

const BUILT_IN: OwnerId = moduleId("onecad.demo");
const VENDOR_A: OwnerId = addonId("com.example.a");
const VENDOR_B: OwnerId = addonId("com.example.b");

const SCOPE = "onecad.demo.scope.model";
const CHORD: Shortcut = { key: "k", shift: true };

function setup() {
  const commands = createRegistry<CommandDefinition>("command");
  const tools = createRegistry<ToolDefinition>("tool");
  const host = createToolHost(tools);
  const service = createShortcutService(commands, tools, host);
  return { commands, tools, host, service };
}

function command(
  owner: OwnerId,
  name: string,
  extra: Partial<CommandDefinition> = {},
): CommandDefinition {
  return {
    id: contributionId<CommandId>(owner, `${owner}.command.${name}`),
    title: name,
    defaultShortcut: CHORD,
    execute: () => ({ status: "done" as const }),
    ...extra,
  };
}

describe("shortcut service", () => {
  it("resolves a contributed chord to its registration", () => {
    const { commands, service } = setup();
    const entry = command(VENDOR_A, "inspect");
    commands.register(VENDOR_A, entry);

    expect(service.resolve({ key: "K", shift: true }, [])).toMatchObject({
      kind: "command",
      id: entry.id,
      owner: VENDOR_A,
    });
  });

  it("matches the chord EXACTLY — a bare key is not a shifted one", () => {
    const { commands, service } = setup();
    commands.register(VENDOR_A, command(VENDOR_A, "inspect"));

    expect(service.resolve({ key: "k" }, [])).toBeNull();
    expect(service.resolve({ key: "k", shift: true, meta: true }, [])).toBeNull();
  });

  it("honours scopes, and treats an absent scope list as everywhere", () => {
    const { commands, service } = setup();
    commands.register(VENDOR_A, command(VENDOR_A, "scoped", { scopes: [SCOPE] }));

    expect(service.resolve(CHORD, [SCOPE])).not.toBeNull();
    expect(service.resolve(CHORD, ["some.other.scope"])).toBeNull();

    const { commands: c2, service: s2 } = setup();
    c2.register(VENDOR_A, command(VENDOR_A, "unscoped"));
    expect(s2.resolve(CHORD, ["anything"])).not.toBeNull();
  });

  it("a scoped claim beats an unscoped one", () => {
    const { commands, service } = setup();
    const scoped = command(VENDOR_A, "scoped", { scopes: [SCOPE] });
    commands.register(VENDOR_A, scoped);
    commands.register(VENDOR_B, command(VENDOR_B, "unscoped"));

    expect(service.resolve(CHORD, [SCOPE])?.id).toBe(scoped.id);
  });

  it("then explicit priority", () => {
    const { commands, service } = setup();
    const loud = command(VENDOR_A, "loud", { priority: 10 });
    commands.register(VENDOR_A, loud);
    commands.register(VENDOR_B, command(VENDOR_B, "quiet", { priority: 500 }));

    expect(service.resolve(CHORD, [])?.id).toBe(loud.id);
  });

  it("then built-in over addon", () => {
    const { commands, service } = setup();
    const mine = command(BUILT_IN, "mine");
    commands.register(BUILT_IN, mine);
    commands.register(VENDOR_A, command(VENDOR_A, "theirs"));

    expect(service.resolve(CHORD, [])?.id).toBe(mine.id);
  });

  it("an unbreakable tie activates NOTHING and is reported", () => {
    const { commands, service } = setup();
    commands.register(VENDOR_A, command(VENDOR_A, "one"));
    commands.register(VENDOR_B, command(VENDOR_B, "two"));

    // Last-registered-wins here would make behavior depend on install order.
    expect(service.resolve(CHORD, [])).toBeNull();

    const conflict = service.conflictFor(CHORD, []);
    expect(conflict?.candidates).toHaveLength(2);
    expect(service.conflicts([])).toHaveLength(1);
  });

  it("running a tool target goes through the tool host", async () => {
    const { tools, host, service } = setup();
    const activate = vi.fn();
    const id = contributionId<ToolId>(VENDOR_A, "com.example.a.tool.inspect");
    tools.register(VENDOR_A, {
      id,
      title: "Inspect",
      defaultShortcut: CHORD,
      activate,
      deactivate: () => {},
    });

    const target = service.resolve(CHORD, []);
    await service.run(target!);

    expect(activate).toHaveBeenCalledOnce();
    expect(host.activeToolId()).toBe(id);
  });

  it("a keystroke does not bypass canExecute", async () => {
    const { commands, service } = setup();
    const execute = vi.fn(() => ({ status: "done" as const }));
    commands.register(
      VENDOR_A,
      command(VENDOR_A, "gated", {
        canExecute: () => ({ enabled: false, reason: "nothing selected" }),
        execute,
      }),
    );

    await service.run(service.resolve(CHORD, [])!);

    // The palette and the toolbar both respect the gate; so must the keyboard.
    expect(execute).not.toHaveBeenCalled();
  });

  it("a disposed registration stops answering", () => {
    const { commands, service } = setup();
    const handle = commands.register(VENDOR_A, command(VENDOR_A, "inspect"));
    expect(service.resolve(CHORD, [])).not.toBeNull();

    handle.dispose();

    expect(service.resolve(CHORD, [])).toBeNull();
  });
});
