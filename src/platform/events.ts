/*
 * Event bus.
 *
 * Events ANNOUNCE state changes. They are not an alternative way to perform them
 * — `onecad.modeling.history.changed` is an event, "please extrude this body" is
 * not (docs/ARCHITECTURE.md §10). Operations go through commands and services so
 * they stay transactional and undoable.
 *
 * Subscriptions are owned, so unloading a module cannot leave a listener behind
 * holding its disposed state (ADR-0006).
 */
import type { EventId, OwnerId } from "./ids";
import type { Disposable } from "./registry";

type Listener = (payload: unknown) => void;

export interface EventBus {
  emit(id: EventId, payload?: unknown): void;
  subscribe<T = unknown>(owner: OwnerId, id: EventId, fn: (payload: T) => void): Disposable;
  /** Removes every subscription made by `owner`. Returns how many went. */
  disposeOwner(owner: OwnerId): number;
  listenerCount(id: EventId): number;
}

export function createEventBus(): EventBus {
  interface Entry {
    readonly id: EventId;
    readonly owner: OwnerId;
    readonly fn: Listener;
  }
  const byId = new Map<EventId, Set<Entry>>();

  const remove = (entry: Entry): void => {
    const set = byId.get(entry.id);
    if (!set) return;
    set.delete(entry);
    if (set.size === 0) byId.delete(entry.id);
  };

  return {
    emit(id, payload) {
      const set = byId.get(id);
      if (!set) return;
      // Snapshot: a listener may subscribe or dispose while we are notifying.
      for (const entry of [...set]) entry.fn(payload);
    },

    subscribe(owner, id, fn) {
      const entry: Entry = { id, owner, fn: fn as Listener };
      const set = byId.get(id) ?? new Set<Entry>();
      set.add(entry);
      byId.set(id, set);
      return { dispose: () => remove(entry) };
    },

    disposeOwner(owner) {
      let removed = 0;
      for (const set of [...byId.values()]) {
        for (const entry of [...set]) {
          if (entry.owner === owner) {
            remove(entry);
            removed++;
          }
        }
      }
      return removed;
    },

    listenerCount: (id) => byId.get(id)?.size ?? 0,
  };
}
