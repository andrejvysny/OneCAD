/*
 * React binding for the Platform.
 *
 * The Platform is passed through context, never imported as a global singleton:
 * a component that reaches for a module-scoped global cannot be rendered in a
 * test (or in a second window) without that global existing. Everything a
 * contribution needs arrives through this provider.
 */
import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";
import type { Platform } from "../platform";
import type { Contribution, Registry } from "../registry";

const PlatformContext = createContext<Platform | null>(null);

export function PlatformProvider({
  platform,
  children,
}: {
  platform: Platform;
  children: ReactNode;
}) {
  return <PlatformContext.Provider value={platform}>{children}</PlatformContext.Provider>;
}

export function usePlatform(): Platform {
  const platform = useContext(PlatformContext);
  if (!platform) {
    throw new Error("usePlatform() outside a <PlatformProvider> — check the bootstrap order");
  }
  return platform;
}

/** The Platform if one is mounted, else null. For code that must degrade. */
export function useOptionalPlatform(): Platform | null {
  return useContext(PlatformContext);
}

/**
 * Subscribe to a registry's ordered contents.
 *
 * `entries()` returns a cached reference that changes only when the registry
 * does, which is exactly `useSyncExternalStore`'s contract.
 */
export function useRegistryEntries<T extends Contribution>(registry: Registry<T>): readonly T[] {
  return useSyncExternalStore(registry.subscribe, registry.entries, registry.entries);
}
