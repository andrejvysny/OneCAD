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
 *
 * That rule applies to `report()` TOO, and that is not obvious: a built-in tool
 * chord goes straight into modeling's own dispatcher and only reaches the host
 * as a report, so a report-only path would silently strand an active addon tool
 * (Codex review, P2.5).
 */
import type { ToolContext, ToolDefinition } from "./contributions";
import type { ToolId } from "./ids";
import type { Registry } from "./registry";

export interface ToolHost {
  /** The active tool id, or null when nothing is active. */
  activeToolId(): ToolId | null;
  /**
   * Run the activation handshake for `id`. Unknown ids are ignored — a stale
   * button click after a dispose must not throw into the render tree.
   *
   * `ctx` is what the tool is activated WITH. Callers that know the current
   * selection and scopes (the toolbar, the keyboard lane) pass them; the default
   * is empty, which is the honest answer for a caller that knows neither.
   */
  activate(id: ToolId, ctx?: ToolContext): Promise<void>;
  /**
   * Mirror an activation that a module performed through its own path. Does not
   * call `activate()` on the incoming definition (the module already did the
   * work), but DOES deactivate an outgoing tool from a different owner.
   */
  report(id: ToolId | null, ctx?: ToolContext): void;
  /** Fires after `activeToolId()` changes. Returns an unsubscribe. */
  subscribe(listener: () => void): () => void;
}

const EMPTY_CTX: ToolContext = { selection: [], scopes: [] };

export function createToolHost(tools: Registry<ToolDefinition>): ToolHost {
  let active: ToolId | null = null;
  const listeners = new Set<() => void>();
  /**
   * Bumped by every state change. An in-flight `activate` compares it on the way
   * out, so a slow tool cannot publish itself over a newer answer — nor undo a
   * newer one when it fails.
   */
  let generation = 0;

  const emit = (): void => {
    for (const l of [...listeners]) l();
  };

  const set = (id: ToolId | null): void => {
    generation += 1;
    if (active === id) return;
    active = id;
    emit();
  };

  /**
   * Tear the outgoing tool down when the incoming one belongs to someone else.
   * Returns whether it actually did.
   */
  const handOver = (nextId: ToolId | null, ctx: ToolContext): boolean => {
    if (active === null || active === nextId) return false;
    const previous = tools.get(active);
    if (!previous) return false;
    const from = tools.ownerOf(previous.id);
    const to = nextId === null ? undefined : tools.ownerOf(nextId);
    // A cross-owner hand-over, or a hand-over to nothing at all.
    if (from === undefined || from === to) return false;
    try {
      // Deliberately not awaited: `report` is synchronous (it mirrors a store
      // write that already happened), and a module's teardown that needs to be
      // awaited is a design the host cannot honour on this path.
      void previous.deactivate(ctx);
    } catch (cause) {
      // One module's failed teardown must not block another's activation.
      console.error(`tool "${previous.id}" deactivate() failed`, cause);
    }
    return true;
  };

  const host: ToolHost = {
    activeToolId: () => active,

    async activate(id, ctx = EMPTY_CTX) {
      const next = tools.get(id);
      if (!next) return;

      const previous = active;
      const handedOver = handOver(next.id as ToolId, ctx);

      // Set BEFORE awaiting: `activate` routes into the owning module, which may
      // report its own state synchronously, and a late write here would then
      // clobber the module's newer answer with this call's stale one.
      set(next.id as ToolId);
      const mine = generation;
      try {
        await next.activate(ctx);
      } catch (cause) {
        // A tool that could not arm must not stay highlighted. Roll back only if
        // nothing has happened since — a newer report wins over this failure
        // exactly as it would have won over the success.
        //
        // WHERE to roll back to: the previous tool is still live only if we did
        // NOT hand over. Once it has been deactivated, "nothing is active" is
        // the truthful answer, and claiming otherwise would highlight a tool
        // that has already torn itself down.
        if (generation === mine) set(handedOver ? null : previous);
        // Never rejects: every caller is a click handler or a keystroke that has
        // nothing useful to do with the error, and an unhandled rejection is a
        // worse outcome than a logged one.
        console.error(`tool "${next.id}" activate() failed`, cause);
      }
    },

    report(id, ctx = EMPTY_CTX) {
      if (id !== null && !tools.has(id)) return;
      handOver(id, ctx);
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
