/*
 * Service registry — the cross-module contract surface.
 *
 * A service is something another module legitimately needs:
 * `onecad.modeling.geometry-query`, `onecad.platform.selection`. It is NOT a
 * dumping ground for internal objects; a `FilletDialogInternalStateService` is
 * the smell this registry attracts and must not accumulate
 * (docs/ARCHITECTURE.md, spec §170).
 *
 * Lookup is by namespaced id, so a consumer depends on a capability rather than
 * on the module that happens to provide it today.
 */
import type { OwnerId, ServiceId } from "./ids";
import { createRegistry, type Disposable } from "./registry";

interface ServiceEntry {
  readonly id: ServiceId;
  readonly impl: unknown;
}

export class ServiceError extends Error {
  constructor(readonly code: "missing-service", message: string) {
    super(message);
    this.name = "ServiceError";
  }
}

export interface ServiceRegistry {
  register<T>(owner: OwnerId, id: ServiceId, impl: T): Disposable;
  /** The implementation, or `undefined` when nothing provides it. */
  get<T>(id: ServiceId): T | undefined;
  /** The implementation, or a throw naming the missing id. */
  require<T>(id: ServiceId): T;
  has(id: ServiceId): boolean;
  ids(): readonly ServiceId[];
  ownerOf(id: ServiceId): OwnerId | undefined;
  disposeOwner(owner: OwnerId): number;
  subscribe(listener: () => void): () => void;
}

export function createServiceRegistry(): ServiceRegistry {
  const registry = createRegistry<ServiceEntry>("service");

  return {
    register: (owner, id, impl) => registry.register(owner, { id, impl }),
    get: <T,>(id: ServiceId) => registry.get(id)?.impl as T | undefined,
    require<T>(id: ServiceId): T {
      const entry = registry.get(id);
      if (!entry) {
        throw new ServiceError("missing-service", `no module provides service "${id}"`);
      }
      return entry.impl as T;
    },
    has: (id) => registry.has(id),
    ids: () => registry.entries().map((e) => e.id),
    ownerOf: (id) => registry.ownerOf(id),
    disposeOwner: (owner) => registry.disposeOwner(owner),
    subscribe: (listener) => registry.subscribe(listener),
  };
}
