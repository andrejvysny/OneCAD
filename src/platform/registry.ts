/*
 * The contribution registry (ADR-0006, ADR-0007).
 *
 * One generic store behind every typed registry. Three properties matter and are
 * each pinned by test:
 *
 *  OWNERSHIP    every entry records who registered it, and disposing an owner
 *               removes ALL of its entries in one call. A module must not have to
 *               remember twenty individual disposers.
 *
 *  DUPLICATES   registering an existing id is a hard failure naming the current
 *               holder. Last-one-wins would let one contributor silently take
 *               over another's command, failing far from its cause.
 *
 *  ORDER        iteration is `(priority, insertion index)`. Insertion index is a
 *               last-resort tiebreak for entries that declared identical
 *               priority — order must not otherwise depend on load timing.
 *               `group` is metadata for consumers (it is what a toolbar reads to
 *               place separators), NOT a sort key: group names sort
 *               alphabetically, which is never the intended visual order.
 */
import { isOwnedBy, type OwnerId } from "./ids";

/** Everything a contribution must declare to be placed deterministically. */
export interface Contribution {
  readonly id: string;
  /** Consumer-visible cluster (e.g. a toolbar separator group). Not a sort key. */
  readonly group?: string;
  /** Lower sorts first. Defaults to `DEFAULT_PRIORITY`. */
  readonly priority?: number;
}

export const DEFAULT_PRIORITY = 100;

export interface Registered<T extends Contribution> {
  readonly entry: T;
  readonly owner: OwnerId;
  /** Registration sequence, used only to break exact priority ties. */
  readonly index: number;
}

export interface Disposable {
  dispose(): void;
}

export class RegistrationError extends Error {
  constructor(
    readonly code: "duplicate-id" | "foreign-namespace",
    message: string,
  ) {
    super(message);
    this.name = "RegistrationError";
  }
}

export interface Registry<T extends Contribution> {
  /** Registers `entry` for `owner`. Throws on a duplicate or foreign id. */
  register(owner: OwnerId, entry: T): Disposable;
  get(id: string): T | undefined;
  has(id: string): boolean;
  /** All entries in deterministic order. */
  entries(): readonly T[];
  /** All entries in deterministic order, with their ownership metadata. */
  registrations(): readonly Registered<T>[];
  ownerOf(id: string): OwnerId | undefined;
  /** Removes every registration made by `owner`. Returns how many went. */
  disposeOwner(owner: OwnerId): number;
  /** Fires after any change. Returns an unsubscribe. */
  subscribe(listener: () => void): () => void;
  readonly size: number;
}

function compare(a: Registered<Contribution>, b: Registered<Contribution>): number {
  const pa = a.entry.priority ?? DEFAULT_PRIORITY;
  const pb = b.entry.priority ?? DEFAULT_PRIORITY;
  if (pa !== pb) return pa - pb;
  return a.index - b.index;
}

/**
 * @param label appears in error messages ("command", "panel", …).
 * @param allowUnownedIds escape hatch for registries whose ids are not namespaced
 *        under the owner (currently none; kept explicit rather than implicit).
 */
export function createRegistry<T extends Contribution>(
  label: string,
  options: { allowUnownedIds?: boolean } = {},
): Registry<T> {
  const byId = new Map<string, Registered<T>>();
  const listeners = new Set<() => void>();
  let nextIndex = 0;
  // Sorted views, rebuilt lazily: reads happen per render, writes at startup.
  // Both are cached because `useSyncExternalStore` requires a STABLE snapshot
  // reference between changes — returning a fresh array per call loops forever.
  let sortedCache: readonly Registered<T>[] | null = null;
  let entriesCache: readonly T[] | null = null;

  const changed = (): void => {
    sortedCache = null;
    entriesCache = null;
    for (const l of listeners) l();
  };

  const sorted = (): readonly Registered<T>[] => {
    if (sortedCache === null) sortedCache = [...byId.values()].sort(compare);
    return sortedCache;
  };

  const entries = (): readonly T[] => {
    if (entriesCache === null) entriesCache = sorted().map((r) => r.entry);
    return entriesCache;
  };

  return {
    register(owner, entry) {
      if (!options.allowUnownedIds && !isOwnedBy(owner, entry.id)) {
        throw new RegistrationError(
          "foreign-namespace",
          `${label} "${entry.id}" is outside its owner's namespace ("${owner}.*")`,
        );
      }
      const existing = byId.get(entry.id);
      if (existing) {
        throw new RegistrationError(
          "duplicate-id",
          `${label} "${entry.id}" is already registered by "${existing.owner}"`,
        );
      }
      byId.set(entry.id, { entry, owner, index: nextIndex++ });
      changed();
      return {
        dispose: () => {
          // Only remove OUR registration: a later re-register under the same id
          // belongs to whoever made it, and a stale handle must not evict it.
          if (byId.get(entry.id)?.entry === entry) {
            byId.delete(entry.id);
            changed();
          }
        },
      };
    },

    get: (id) => byId.get(id)?.entry,
    has: (id) => byId.has(id),
    entries,
    registrations: () => sorted(),
    ownerOf: (id) => byId.get(id)?.owner,

    disposeOwner(owner) {
      let removed = 0;
      for (const [id, reg] of [...byId]) {
        if (reg.owner === owner) {
          byId.delete(id);
          removed++;
        }
      }
      if (removed > 0) changed();
      return removed;
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    get size() {
      return byId.size;
    },
  };
}
