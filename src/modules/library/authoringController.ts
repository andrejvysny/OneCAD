/*
 * authoringController — the open-state of the "Save as Component" dialog
 * (WP-B2).
 *
 * A module-level singleton with `useSyncExternalStore`, the same shape
 * `placementController` and `engineBridge` use, and for the same reason: the
 * thing that OPENS the dialog is a menu contribution's `run()` — a plain
 * callback registered at editor mount, with no React tree above it. A context
 * would not reach it.
 *
 * Deliberately holds a target and nothing else. The dialog owns its own form
 * state; keeping that here would make a re-open show the last attempt's
 * half-typed values.
 */
import { useSyncExternalStore } from "react";

export interface AuthoringTarget {
  bodyId: string;
  bodyName?: string;
}

let target: AuthoringTarget | null = null;
const listeners = new Set<() => void>();

function publish(): void {
  for (const l of [...listeners]) l();
}

/** Opens the dialog for `body`. Re-opening on another body replaces the target. */
export function openSaveAsComponent(next: AuthoringTarget): void {
  target = next;
  publish();
}

export function closeSaveAsComponent(): void {
  if (target === null) return;
  target = null;
  publish();
}

export function saveAsComponentTarget(): AuthoringTarget | null {
  return target;
}

export function useSaveAsComponentTarget(): AuthoringTarget | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    saveAsComponentTarget,
    () => null,
  );
}
