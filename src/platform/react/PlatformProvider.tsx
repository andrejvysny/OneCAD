/*
 * React binding for the Platform.
 *
 * The Platform is passed through context, never imported as a global singleton:
 * a component that reaches for a module-scoped global cannot be rendered in a
 * test (or in a second window) without that global existing. Everything a
 * contribution needs arrives through this provider.
 */
import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";
import type { ToolId } from "../ids";
import type { Platform } from "../platform";
import type { Contribution, Registered, Registry } from "../registry";

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

/**
 * The same, with ownership metadata.
 *
 * Separate from `useRegistryEntries` rather than replacing it: WHO registered a
 * contribution is a question only a few surfaces ask (the workspace menu splits
 * built-ins from add-ons, the extensions manager lists by owner), and handing
 * every consumer the owner would invite reading it where the id alone is the
 * right currency.
 */
export function useRegistryRegistrations<T extends Contribution>(
  registry: Registry<T>,
): readonly Registered<T>[] {
  return useSyncExternalStore(registry.subscribe, registry.registrations, registry.registrations);
}

/** The active tool id, or null. Same subscription contract as above. */
export function useActiveToolId(): ToolId | null {
  const { toolHost } = usePlatform();
  return useSyncExternalStore(toolHost.subscribe, toolHost.activeToolId, toolHost.activeToolId);
}
