/*
 * Module lifecycle: dependency ordering, cycle rejection, and the guarantee that
 * a module scope tears down everything the module registered (ADR-0006).
 */
import { describe, it, expect, vi } from "vitest";
import { createPlatform, ModuleError, type ModuleDefinition, type ModuleScope } from "./platform";
import { moduleId, contributionId, unsafeId } from "./ids";
import type { CommandId, EventId, PanelId, ServiceId, ToolId } from "./ids";
import { Slots } from "./slots";

const A = moduleId("onecad.a");
const B = moduleId("onecad.b");
const C = moduleId("onecad.c");

function noopModule(id: typeof A, extra: Partial<ModuleDefinition> = {}): ModuleDefinition {
  return { id, version: "1.0.0", activate: () => {}, ...extra };
}

const Noop = () => null;

describe("module registration", () => {
  it("rejects a duplicate module id", () => {
    const platform = createPlatform();
    platform.registerModule(noopModule(A));
    expect(() => platform.registerModule(noopModule(A))).toThrowError(ModuleError);
  });

  it("rejects registration after initialize()", async () => {
    const platform = createPlatform();
    await platform.initialize();
    try {
      platform.registerModule(noopModule(A));
      expect.unreachable("late registration must fail loudly");
    } catch (e) {
      expect((e as ModuleError).code).toBe("already-initialized");
    }
  });

  it("tracks module state through the lifecycle", async () => {
    const platform = createPlatform();
    platform.registerModule(noopModule(A));
    expect(platform.moduleState(A)).toBe("registered");
    await platform.initialize();
    expect(platform.moduleState(A)).toBe("ready");
    platform.dispose();
    expect(platform.moduleState(A)).toBe("disposed");
  });
});

describe("initialization order", () => {
  it("activates dependencies first, regardless of registration order", async () => {
    const order: string[] = [];
    const platform = createPlatform();
    // C depends on B depends on A, registered in reverse.
    platform.registerModule({
      id: C,
      version: "1.0.0",
      dependsOn: [B],
      activate: () => void order.push("c"),
    });
    platform.registerModule({
      id: B,
      version: "1.0.0",
      dependsOn: [A],
      activate: () => void order.push("b"),
    });
    platform.registerModule({ id: A, version: "1.0.0", activate: () => void order.push("a") });

    await platform.initialize();
    expect(order).toEqual(["a", "b", "c"]);
  });

  it("rejects a missing dependency, naming both modules", async () => {
    const platform = createPlatform();
    platform.registerModule(noopModule(A, { dependsOn: [B] }));
    await expect(platform.initialize()).rejects.toThrowError(/onecad\.a.*onecad\.b/);
  });

  it("rejects a dependency cycle", async () => {
    const platform = createPlatform();
    platform.registerModule(noopModule(A, { dependsOn: [B] }));
    platform.registerModule(noopModule(B, { dependsOn: [A] }));

    await expect(platform.initialize()).rejects.toThrowError(ModuleError);
    await expect(platform.initialize()).rejects.toThrowError(/cycle/);
  });

  it("awaits an async activate before starting a dependent module", async () => {
    const order: string[] = [];
    const platform = createPlatform();
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: async () => {
        await Promise.resolve();
        order.push("a");
      },
    });
    platform.registerModule({
      id: B,
      version: "1.0.0",
      dependsOn: [A],
      activate: () => void order.push("b"),
    });

    await platform.initialize();
    expect(order).toEqual(["a", "b"]);
  });

  it("a failed activation leaves nothing of that module registered", async () => {
    const platform = createPlatform();
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: (scope) => {
        scope.registerPanel({
          id: contributionId<PanelId>(A, "onecad.a.panel"),
          slot: Slots.ShellLeft,
          component: Noop,
        });
        throw new Error("boom");
      },
    });

    await expect(platform.initialize()).rejects.toThrowError(/failed to activate/);
    expect(platform.moduleState(A)).toBe("failed");
    expect(platform.panels.size).toBe(0);
  });
});

describe("module scope teardown", () => {
  function registerEverything(scope: ModuleScope, seen: string[]): void {
    scope.registerCommand({
      id: contributionId<CommandId>(A, "onecad.a.command"),
      title: "Command",
      execute: () => ({ status: "done" }),
    });
    scope.registerTool({
      id: contributionId<ToolId>(A, "onecad.a.tool"),
      title: "Tool",
      activate: () => {},
      deactivate: () => {},
    });
    scope.registerPanel({
      id: contributionId<PanelId>(A, "onecad.a.panel"),
      slot: Slots.ShellLeft,
      component: Noop,
    });
    scope.registerInspectorSection({
      id: contributionId(A, "onecad.a.section"),
      canRender: () => true,
      component: Noop,
    });
    scope.registerViewportContribution({
      id: contributionId(A, "onecad.a.layer"),
      attach: () => ({ dispose: () => {} }),
    });
    scope.registerWorkspace({
      id: contributionId(A, "onecad.a.workspace"),
      title: "W",
      panels: [],
    });
    scope.registerService(contributionId<ServiceId>(A, "onecad.a.service"), { ping: () => "pong" });
    scope.subscribe(unsafeId<EventId>("onecad.a.event"), () => seen.push("event"));
  }

  it("dispose() removes every registration and subscription in one call", async () => {
    const platform = createPlatform();
    const seen: string[] = [];
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: (scope) => registerEverything(scope, seen),
    });
    await platform.initialize();

    expect(platform.commands.size).toBe(1);
    expect(platform.services.has(unsafeId<ServiceId>("onecad.a.service"))).toBe(true);
    platform.events.emit(unsafeId<EventId>("onecad.a.event"));
    expect(seen).toEqual(["event"]);

    platform.scopeFor(A).dispose();

    expect(platform.commands.size).toBe(0);
    expect(platform.tools.size).toBe(0);
    expect(platform.panels.size).toBe(0);
    expect(platform.inspector.size).toBe(0);
    expect(platform.viewport.size).toBe(0);
    expect(platform.workspaces.size).toBe(0);
    expect(platform.services.has(unsafeId<ServiceId>("onecad.a.service"))).toBe(false);

    platform.events.emit(unsafeId<EventId>("onecad.a.event"));
    expect(seen, "a disposed module must not still be listening").toEqual(["event"]);
  });

  it("owned disposables run in reverse registration order", async () => {
    const order: string[] = [];
    const platform = createPlatform();
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: (scope) => {
        scope.own({ dispose: () => order.push("first") });
        scope.own({ dispose: () => order.push("second") });
      },
    });
    await platform.initialize();
    platform.scopeFor(A).dispose();
    expect(order).toEqual(["second", "first"]);
  });

  it("disposing one module leaves another module's registrations alone", async () => {
    const platform = createPlatform();
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: (scope) =>
        void scope.registerCommand({
          id: contributionId<CommandId>(A, "onecad.a.command"),
          title: "A",
          execute: () => ({ status: "done" }),
        }),
    });
    platform.registerModule({
      id: B,
      version: "1.0.0",
      activate: (scope) =>
        void scope.registerCommand({
          id: contributionId<CommandId>(B, "onecad.b.command"),
          title: "B",
          execute: () => ({ status: "done" }),
        }),
    });
    await platform.initialize();

    platform.scopeFor(A).dispose();
    expect(platform.commands.entries().map((c) => c.id)).toEqual(["onecad.b.command"]);
  });
});

describe("services", () => {
  it("require() throws a message naming the missing service", async () => {
    const platform = createPlatform();
    expect(() => platform.services.require(unsafeId<ServiceId>("onecad.a.nope"))).toThrowError(
      /onecad\.a\.nope/,
    );
  });

  it("a service is retrievable by id and typed at the call site", async () => {
    const platform = createPlatform();
    platform.registerModule({
      id: A,
      version: "1.0.0",
      activate: (scope) =>
        void scope.registerService(contributionId<ServiceId>(A, "onecad.a.svc"), {
          answer: () => 42,
        }),
    });
    await platform.initialize();

    const svc = platform.services.require<{ answer: () => number }>(
      unsafeId<ServiceId>("onecad.a.svc"),
    );
    expect(svc.answer()).toBe(42);
  });
});

describe("events", () => {
  it("delivers to every listener and survives a listener unsubscribing mid-emit", () => {
    const platform = createPlatform();
    const seen: string[] = [];
    const first = platform.events.subscribe(A, unsafeId<EventId>("onecad.a.e"), () => {
      seen.push("first");
      first.dispose();
    });
    platform.events.subscribe(A, unsafeId<EventId>("onecad.a.e"), () => seen.push("second"));

    platform.events.emit(unsafeId<EventId>("onecad.a.e"));
    expect(seen).toEqual(["first", "second"]);

    platform.events.emit(unsafeId<EventId>("onecad.a.e"));
    expect(seen).toEqual(["first", "second", "second"]);
  });

  it("carries its payload", () => {
    const platform = createPlatform();
    const fn = vi.fn();
    platform.events.subscribe<{ n: number }>(A, unsafeId<EventId>("onecad.a.e"), fn);
    platform.events.emit(unsafeId<EventId>("onecad.a.e"), { n: 7 });
    expect(fn).toHaveBeenCalledWith({ n: 7 });
  });
});
