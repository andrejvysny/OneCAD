/*
 * The Platform: the composition root and the module lifecycle.
 *
 * It owns registries and infrastructure, and knows nothing about any domain. A
 * module registers what it contributes; the shell discovers contributions. The
 * Platform never imports a module (docs/ARCHITECTURE.md §3) — that inversion is
 * what keeps `Platform → Modeling` from reappearing as an import cycle.
 *
 * Lifecycle: register → initialize (dependency order) → ready → dispose.
 * Built-in modules stay initialized for the process lifetime; the machinery for
 * unloading exists anyway, because retrofitting ownership later is far harder
 * than starting with it (ADR-0006).
 */
import type {
  CommandDefinition,
  InspectorContribution,
  PanelContribution,
  ToolDefinition,
  ViewportContribution,
  TreeProvider,
  WorkspaceDefinition,
} from "./contributions";
import { createEventBus, type EventBus } from "./events";
import type { EventId, ModuleId, OwnerId, ServiceId } from "./ids";
import { createRegistry, type Disposable, type Registry } from "./registry";
import { createServiceRegistry, type ServiceRegistry } from "./services";
import { createShortcutService, type ShortcutService } from "./shortcuts";
import { createToolHost, type ToolHost } from "./toolHost";

export interface ModuleManifest {
  readonly id: ModuleId;
  readonly version: string;
  /** Modules that must be ready before this one activates. */
  readonly dependsOn?: readonly ModuleId[];
  /** Services this module intends to provide (documentation + diagnostics). */
  readonly provides?: readonly ServiceId[];
}

export interface ModuleDefinition extends ModuleManifest {
  activate(scope: ModuleScope): void | Promise<void>;
  /**
   * Runs BEFORE the module's registrations are torn down, so it can still reach
   * what it registered. For teardown of owned resources prefer `scope.own()`:
   * this hook is for work a disposer cannot express (flushing, telling a peer).
   *
   * A throwing `deactivate` does not abort disposal — the registrations still
   * go. Refusing to unload because a module's goodbye failed would leave the
   * platform holding contributions nobody owns.
   */
  deactivate?(scope: ModuleScope): void | Promise<void>;
}

/**
 * `deactivating` is observable rather than internal: a module's own `deactivate`
 * runs in it, and a service consulted from there must be able to tell that its
 * owner is going away instead of seeing "ready" one moment and "disposed" the
 * next with no state in between.
 */
export type ModuleState = "registered" | "ready" | "deactivating" | "failed" | "disposed";

export class ModuleError extends Error {
  constructor(
    readonly code:
      | "duplicate-module"
      | "missing-dependency"
      | "dependency-cycle"
      | "activation-failed"
      | "already-initialized",
    message: string,
  ) {
    super(message);
    this.name = "ModuleError";
  }
}

/**
 * What a module is handed at activation. Every registration made through a scope
 * is owned by that module and dies with it, so `scope.dispose()` is the only
 * teardown a module has to implement.
 */
export interface ModuleScope {
  readonly owner: OwnerId;
  readonly platform: Platform;
  registerCommand(entry: CommandDefinition): Disposable;
  registerTool(entry: ToolDefinition): Disposable;
  registerPanel(entry: PanelContribution): Disposable;
  registerInspectorSection(entry: InspectorContribution): Disposable;
  registerViewportContribution(entry: ViewportContribution): Disposable;
  registerTreeProvider(entry: TreeProvider): Disposable;
  registerWorkspace(entry: WorkspaceDefinition): Disposable;
  registerService<T>(id: ServiceId, impl: T): Disposable;
  subscribe<T = unknown>(id: EventId, fn: (payload: T) => void): Disposable;
  /** Tie an arbitrary teardown to this module's lifetime. */
  own(disposable: Disposable): Disposable;
  dispose(): void;
}

export interface Platform {
  readonly commands: Registry<CommandDefinition>;
  readonly tools: Registry<ToolDefinition>;
  readonly panels: Registry<PanelContribution>;
  readonly inspector: Registry<InspectorContribution>;
  readonly viewport: Registry<ViewportContribution>;
  readonly tree: Registry<TreeProvider>;
  readonly workspaces: Registry<WorkspaceDefinition>;
  readonly services: ServiceRegistry;
  readonly events: EventBus;
  /** Who is active, and the activate/deactivate handshake. See `toolHost.ts`. */
  readonly toolHost: ToolHost;
  /** Chord → registration, with a deterministic conflict rule. `shortcuts.ts`. */
  readonly shortcuts: ShortcutService;

  registerModule(def: ModuleDefinition): void;
  /** Activates every registered module in dependency order. Idempotent guard. */
  initialize(): Promise<void>;
  /**
   * The same, for a caller that cannot await — the React root has to have a
   * Platform on its FIRST render. Throws if a module's `activate` is async, so
   * the restriction is visible at startup instead of as a half-built registry.
   */
  initializeSync(): void;
  readonly initialized: boolean;

  moduleIds(): readonly ModuleId[];
  moduleState(id: ModuleId): ModuleState | undefined;
  /** The module's own long-lived scope, created on first use. */
  scopeFor(owner: OwnerId): ModuleScope;
  /**
   * An INDEPENDENT scope under the same owner, for registrations with a shorter
   * life than the module — editor-screen UI that comes and goes with the screen.
   * Disposing it leaves the module's own scope untouched.
   */
  createScope(owner: OwnerId): ModuleScope;

  /** Removes every registration and subscription belonging to `owner`. */
  disposeOwner(owner: OwnerId): void;
  dispose(): void;
}

/**
 * Depth-first topological order over `dependsOn`.
 * A missing dependency and a cycle are both hard failures naming the modules
 * involved — a module silently skipped is worse than a startup that explains.
 */
function initializationOrder(modules: readonly ModuleDefinition[]): ModuleDefinition[] {
  const byId = new Map(modules.map((m) => [m.id, m]));
  const ordered: ModuleDefinition[] = [];
  const done = new Set<ModuleId>();
  const onPath: ModuleId[] = [];

  const visit = (def: ModuleDefinition): void => {
    if (done.has(def.id)) return;
    if (onPath.includes(def.id)) {
      const cycle = [...onPath.slice(onPath.indexOf(def.id)), def.id].join(" → ");
      throw new ModuleError("dependency-cycle", `module dependency cycle: ${cycle}`);
    }
    onPath.push(def.id);
    for (const depId of def.dependsOn ?? []) {
      const dep = byId.get(depId);
      if (!dep) {
        throw new ModuleError(
          "missing-dependency",
          `module "${def.id}" depends on "${depId}", which is not registered`,
        );
      }
      visit(dep);
    }
    onPath.pop();
    done.add(def.id);
    ordered.push(def);
  };

  // Registration order decides only the order of otherwise-independent modules,
  // which is why every contribution carries its own explicit priority.
  for (const def of modules) visit(def);
  return ordered;
}

export function createPlatform(): Platform {
  const commands = createRegistry<CommandDefinition>("command");
  const tools = createRegistry<ToolDefinition>("tool");
  const panels = createRegistry<PanelContribution>("panel");
  const inspector = createRegistry<InspectorContribution>("inspector section");
  const viewport = createRegistry<ViewportContribution>("viewport contribution");
  const tree = createRegistry<TreeProvider>("tree provider");
  const workspaces = createRegistry<WorkspaceDefinition>("workspace");
  const services = createServiceRegistry();
  const events = createEventBus();
  const toolHost = createToolHost(tools);
  const shortcuts = createShortcutService(commands, tools, toolHost);

  const definitions = new Map<ModuleId, ModuleDefinition>();
  const states = new Map<ModuleId, ModuleState>();
  const scopes = new Map<OwnerId, ModuleScope>();
  let initialized = false;

  /** Drop every registration `owner` holds, across all registries. */
  const sweep = (owner: OwnerId): void => {
    commands.disposeOwner(owner);
    tools.disposeOwner(owner);
    panels.disposeOwner(owner);
    inspector.disposeOwner(owner);
    viewport.disposeOwner(owner);
    tree.disposeOwner(owner);
    workspaces.disposeOwner(owner);
    services.disposeOwner(owner);
    events.disposeOwner(owner);
  };

  /**
   * Give a module its goodbye, before anything it registered is taken away.
   *
   * Only for a module that actually reached `ready`: running `deactivate` on one
   * whose `activate` threw would hand it a half-built world it never finished
   * building. A throw here is swallowed — see `ModuleDefinition.deactivate`.
   */
  const runDeactivate = (owner: OwnerId): void => {
    const def = definitions.get(owner as ModuleId);
    if (!def?.deactivate || states.get(owner as ModuleId) !== "ready") return;
    states.set(owner as ModuleId, "deactivating");
    try {
      const result = def.deactivate(platform.scopeFor(owner)) as unknown;
      if (result instanceof Promise) {
        // Disposal is synchronous everywhere it is called from (React cleanup,
        // `disposeOwner`). Awaiting here would tear the registrations down while
        // the module still believed it was live, so an async `deactivate` is
        // reported rather than silently half-run.
        void result.catch(() => {});
        throw new Error("deactivate() returned a promise; disposal is synchronous");
      }
    } catch (cause) {
      // A failed goodbye must not keep the platform holding orphan contributions.
      console.error(`module "${owner}" deactivate() failed`, cause);
    }
  };

  const platform: Platform = {
    commands,
    tools,
    panels,
    inspector,
    viewport,
    tree,
    workspaces,
    services,
    events,
    toolHost,
    shortcuts,

    registerModule(def) {
      if (initialized) {
        throw new ModuleError(
          "already-initialized",
          `module "${def.id}" registered after initialize(); register during bootstrap`,
        );
      }
      if (definitions.has(def.id)) {
        throw new ModuleError("duplicate-module", `module "${def.id}" is already registered`);
      }
      definitions.set(def.id, def);
      states.set(def.id, "registered");
    },

    async initialize() {
      if (initialized) return;
      // Resolved BEFORE anything is marked initialized, so a graph error (cycle,
      // missing dependency) throws on every call rather than latching the
      // platform into a half-initialized state that a retry silently accepts.
      const order = initializationOrder([...definitions.values()]);

      // Set before activating: a module registering another during activation is
      // a bootstrap-ordering bug, and should fail loudly rather than half-work.
      initialized = true;
      for (const def of order) {
        const scope = platform.scopeFor(def.id);
        try {
          await def.activate(scope);
          states.set(def.id, "ready");
        } catch (cause) {
          // Leave nothing half-registered behind a failed activation. The state
          // is stamped AFTER teardown, which reports "disposed" of its own.
          scope.dispose();
          states.set(def.id, "failed");
          throw new ModuleError(
            "activation-failed",
            `module "${def.id}" failed to activate: ${
              cause instanceof Error ? cause.message : String(cause)
            }`,
          );
        }
      }
    },

    initializeSync() {
      if (initialized) return;
      const order = initializationOrder([...definitions.values()]);
      initialized = true;
      for (const def of order) {
        const scope = platform.scopeFor(def.id);
        try {
          const result = def.activate(scope) as unknown;
          if (result instanceof Promise) {
            throw new Error("activate() returned a promise; use initialize() instead");
          }
          states.set(def.id, "ready");
        } catch (cause) {
          scope.dispose();
          states.set(def.id, "failed");
          throw new ModuleError(
            "activation-failed",
            `module "${def.id}" failed to activate: ${
              cause instanceof Error ? cause.message : String(cause)
            }`,
          );
        }
      }
    },

    get initialized() {
      return initialized;
    },

    moduleIds: () => [...definitions.keys()],
    moduleState: (id) => states.get(id),

    scopeFor(owner) {
      const existing = scopes.get(owner);
      if (existing) return existing;
      const scope = platform.createScope(owner);
      scopes.set(owner, scope);
      return scope;
    },

    createScope(owner) {
      const owned: Disposable[] = [];
      const track = (d: Disposable): Disposable => {
        owned.push(d);
        return d;
      };

      const scope: ModuleScope = {
        owner,
        platform,
        registerCommand: (e) => track(commands.register(owner, e)),
        registerTool: (e) => track(tools.register(owner, e)),
        registerPanel: (e) => track(panels.register(owner, e)),
        registerInspectorSection: (e) => track(inspector.register(owner, e)),
        registerViewportContribution: (e) => track(viewport.register(owner, e)),
        registerTreeProvider: (e) => track(tree.register(owner, e)),
        registerWorkspace: (e) => track(workspaces.register(owner, e)),
        registerService: (id, impl) => track(services.register(owner, id, impl)),
        subscribe: (id, fn) => track(events.subscribe(owner, id, fn)),
        own: (d) => track(d),
        dispose() {
          // Reverse order, so a contribution can rely on what it was built on
          // still existing while it tears down.
          for (const d of owned.reverse()) d.dispose();
          owned.length = 0;
          // Deliberately NOT `disposeOwner(owner)`: a scope disposes what IT
          // registered. Sweeping the owner here would let a short-lived child
          // scope tear down the module's own long-lived registrations.
          if (scopes.get(owner) === scope) scopes.delete(owner);
        },
      };
      return scope;
    },

    disposeOwner(owner) {
      runDeactivate(owner);
      sweep(owner);
      if (definitions.has(owner as ModuleId)) states.set(owner as ModuleId, "disposed");
    },

    dispose() {
      // Reverse initialization order: a module tears down while the ones it was
      // built on are still ready, mirroring `scope.dispose()`'s own rule.
      const order = [...definitions.keys()].reverse();
      for (const id of order) runDeactivate(id);
      for (const scope of [...scopes.values()]) scope.dispose();
      // Sweep the registries too. `scope.dispose()` only drops what a SCOPE
      // tracked, so anything registered under an owner by another route would
      // otherwise survive a platform that reports itself disposed.
      for (const id of order) sweep(id);
      for (const id of definitions.keys()) states.set(id, "disposed");
      definitions.clear();
      // Deliberately reusable: with `definitions` cleared this is an empty
      // platform, and a test (or a future reload) can register into it again.
      // Latching it shut would buy nothing — the registries are already empty.
      initialized = false;
    },
  };

  return platform;
}
