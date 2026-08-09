/*
 * `ExtensionContext` — what an addon is handed, and the whole of it.
 *
 * This is NOT `ModuleScope`. A scope carries `platform`, which is the entire
 * host: every registry, `disposeOwner`, `scopeFor(anyOwner)`, `dispose()`. That
 * is the right shape for a built-in module compiled into the app and the wrong
 * shape for a package downloaded from a release page — not because addons are
 * assumed hostile in v1 (they are trusted), but because whatever they can reach
 * is what we must keep working forever.
 *
 * So the context exposes REGISTRATION and the few services an extension
 * legitimately needs, and nothing that lets it reach around them:
 *
 *   no `platform`          — no way to enumerate or dispose another owner
 *   no `createScope`       — an addon's lifetime is the host's to decide
 *   no registry objects    — register, don't mutate someone else's entries
 *   no `CadClient`         — application IPC is not an extension API
 *   no store, no engine    — those are implementation, and they move
 *
 * `document.state` arrives already bound to the addon's own namespace, so the
 * owner id is not a parameter an addon could get wrong (ADR-0004/0005).
 */
import type {
  CommandDefinition,
  InspectorContribution,
  PanelContribution,
  ToolDefinition,
  TreeProvider,
  ViewportContribution,
  WorkspaceDefinition,
} from "./contributions";
import type { DocumentStateService } from "./documentState";
import type { EventId, OwnerId, ServiceId, ToolId } from "./ids";
import type { ModuleScope } from "./platform";
import type { Disposable } from "./registry";

/** Read-only view of the services other modules publish. */
export interface ExtensionServiceApi {
  /** The implementation, or `undefined` when nothing provides it. */
  get<T>(id: ServiceId): T | undefined;
  /** The implementation, or a throw naming the missing id. */
  require<T>(id: ServiceId): T;
  has(id: ServiceId): boolean;
  /** Publish a service of this addon's own. */
  provide<T>(id: ServiceId, impl: T): Disposable;
}

export interface ExtensionEventApi {
  emit(id: EventId, payload?: unknown): void;
  /** Owned by this addon: unloading it takes the listener with it. */
  subscribe<T = unknown>(id: EventId, fn: (payload: T) => void): Disposable;
}

/** Which tool is active, and how to make one active. No host internals. */
export interface ExtensionToolHostApi {
  activeToolId(): ToolId | null;
  activate(id: ToolId): Promise<void>;
  subscribe(listener: () => void): () => void;
}

export interface ExtensionDocumentApi {
  /** This addon's own slice, already bound to its namespace. */
  readonly state: DocumentStateService;
}

export interface ExtensionContext {
  /** The addon's id. Every contribution id must start with it. */
  readonly owner: OwnerId;

  registerCommand(entry: CommandDefinition): Disposable;
  registerTool(entry: ToolDefinition): Disposable;
  registerPanel(entry: PanelContribution): Disposable;
  registerInspectorSection(entry: InspectorContribution): Disposable;
  registerViewportContribution(entry: ViewportContribution): Disposable;
  registerTreeProvider(entry: TreeProvider): Disposable;
  registerWorkspace(entry: WorkspaceDefinition): Disposable;

  readonly document: ExtensionDocumentApi;
  readonly services: ExtensionServiceApi;
  readonly events: ExtensionEventApi;
  readonly tools: ExtensionToolHostApi;

  /** Tie an arbitrary teardown to this addon's lifetime. */
  own(disposable: Disposable): Disposable;
}

/** What an addon's entry point looks like. */
export interface ExtensionDefinition {
  activate(context: ExtensionContext): void | Promise<void>;
  /** Synchronous, for the reason `ModuleDefinition.deactivate` states. */
  deactivate?(context: ExtensionContext): void;
}

/**
 * Narrow a module scope into the surface an addon gets.
 *
 * HOST-SIDE, and deliberately not exported from `@onecad/sdk`: an addon that
 * could build its own context could hand itself the scope this hides.
 *
 * The narrowing is by CONSTRUCTION — a fresh object with the allowed methods —
 * not by casting a scope to a smaller type. A cast leaves `platform` present at
 * runtime, one `as` away from anything.
 */
export function createExtensionContext(
  scope: ModuleScope,
  documentState: DocumentStateService,
): ExtensionContext {
  const { platform } = scope;

  return {
    owner: scope.owner,

    registerCommand: (entry) => scope.registerCommand(entry),
    registerTool: (entry) => scope.registerTool(entry),
    registerPanel: (entry) => scope.registerPanel(entry),
    registerInspectorSection: (entry) => scope.registerInspectorSection(entry),
    registerViewportContribution: (entry) => scope.registerViewportContribution(entry),
    registerTreeProvider: (entry) => scope.registerTreeProvider(entry),
    registerWorkspace: (entry) => scope.registerWorkspace(entry),

    document: { state: documentState },

    services: {
      get: <T,>(id: ServiceId) => platform.services.get<T>(id),
      require: <T,>(id: ServiceId) => platform.services.require<T>(id),
      has: (id) => platform.services.has(id),
      provide: (id, impl) => scope.registerService(id, impl),
    },

    events: {
      emit: (id, payload) => platform.events.emit(id, payload),
      subscribe: (id, fn) => scope.subscribe(id, fn),
    },

    tools: {
      activeToolId: () => platform.toolHost.activeToolId(),
      activate: (id) => platform.toolHost.activate(id),
      subscribe: (listener) => platform.toolHost.subscribe(listener),
    },

    own: (disposable) => scope.own(disposable),
  };
}

/*
 * There is deliberately NO `registerExtension` here.
 *
 * It existed briefly and was wrong in a way worth recording: it typed the
 * manifest as `ModuleId`, but every addon id is an `AddonId` — a disjoint brand
 * — so the one caller it was written for could not have used it without a cast.
 * Nothing did use it, which is exactly how a broken adapter survives review.
 *
 * Registration is the LOADER's decision (which owner id, which lifetime, what
 * happens when activation fails, whether a failed addon disables itself), and
 * the loader does not exist yet. `createExtensionContext` is the piece that has
 * a real consumer today; the rest waits for the caller that defines its shape.
 */
