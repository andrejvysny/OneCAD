import { useSyncExternalStore } from "react";

let current: HTMLElement | null = null;
const listeners = new Set<() => void>();

export function setToolChipDockHost(host: HTMLElement | null): void {
  if (current === host) return;
  current = host;
  for (const listener of listeners) listener();
}

export function useToolChipDockHost(): HTMLElement | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
    () => null,
  );
}
