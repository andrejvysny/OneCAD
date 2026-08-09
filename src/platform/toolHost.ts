/*
 * The tool host — who is active, and how a tool becomes active.
 *
 * WHICH tool is active is domain-neutral; WHAT a tool does is not. So the host
 * owns exactly the first half: an id, a subscription, and the activate/deactivate
 * handshake between two definitions. It never learns what a tool draws.
 *
 * ONE WRITER. A module that already has its own tool state (modeling's
 * `toolStore` and its AUTO-MODE dispatcher) stays authoritative for its own
 * tools; it REPORTS every change through `report()`, and the host mirrors it.
 * Activation therefore cannot diverge from the module's own state no matter
 * which entry point started it — toolbar click, keystroke, or a controller
 * arming itself.
 *
 * OWNER-SCOPED DEACTIVATION. `deactivate()` on the outgoing tool runs only when
 * the incoming tool has a DIFFERENT owner. Modules are already exclusive
 * internally (modeling's `activateTool` swaps its own tools), so calling it on
 * every same-owner swap would insert a teardown into a flow that never had one.
 * Crossing an owner boundary is the case nobody handles today, and the one that
 * would otherwise leave two modules both believing they hold the pointer.
 */
import type { ToolDefinition } from "./contributions";
import type { ToolId } from "./ids";
import type { Registry } from "./registry";

export interface ToolHost {
  /** The active tool id, or null when nothing is active. */
  activeToolId(): ToolId | null;
  /**
   * Run the activation handshake for `id`. Unknown ids are ignored — a stale
   * button click after a dispose must not throw into the render tree.
   */
  activate(id: ToolId): Promise<void>;
  /**
   * Mirror an activation that a module performed through its own path. Idempotent;
   * does not call `activate()` on the definition (it already ran).
   */
  report(id: ToolId | null): void;
  /** Fires after `activeToolId()` changes. Returns an unsubscribe. */
  subscribe(listener: () => void): () => void;
}

export function createToolHost(tools: Registry<ToolDefinition>): ToolHost {
  let active: ToolId | null = null;
  const listeners = new Set<() => void>();

  const emit = (): void => {
    for (const l of [...listeners]) l();
  };

  const set = (id: ToolId | null): void => {
    if (active === id) return;
    active = id;
    emit();
  };

  const host: ToolHost = {
    activeToolId: () => active,

    async activate(id) {
      const next = tools.get(id);
      if (!next) return;

      const previous = active === null ? undefined : tools.get(active);
      if (previous && previous.id !== next.id) {
        const from = tools.ownerOf(previous.id);
        const to = tools.ownerOf(next.id);
        // Cross-owner swap only — see the header.
        if (from !== undefined && to !== undefined && from !== to) {
          await previous.deactivate({ selection: [], scopes: [] });
        }
      }

      // Set BEFORE awaiting: `activate` routes into the owning module, which may
      // report its own state synchronously, and a late write here would then
      // clobber the module's newer answer with this call's stale one.
      set(next.id as ToolId);
      await next.activate({ selection: [], scopes: [] });
    },

    report(id) {
      if (id !== null && !tools.has(id)) return;
      set(id);
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };

  // A disposed tool cannot stay active — the button it was rendered as is gone.
  tools.subscribe(() => {
    if (active !== null && !tools.has(active)) set(null);
  });

  return host;
}
