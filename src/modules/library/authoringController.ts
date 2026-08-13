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
import type { GeometryQueryService } from "@/modules/modeling/manifest";

export interface AuthoringTarget {
  bodyId: string;
  bodyName?: string;
}

/**
 * Modeling's published services, injected at editor mount by
 * `contributeLibraryUi` — the same `configure*` pattern `placementController`
 * uses, and for the same reason (ADR-0002: a kernel touch from library code
 * routes through modeling's services, never a direct `CadClient` call).
 *
 * The attachment picker needs `classifyElement` to turn a viewport pick into a
 * seatable frame. `null` on unmount so a stale service cannot outlive the scope
 * that resolved it; the picker then simply cannot arm.
 */
export interface AuthoringServices {
  geometryQuery: GeometryQueryService;
}

let services: AuthoringServices | null = null;

export function configureAuthoringController(next: AuthoringServices | null): void {
  services = next;
}

export function authoringServices(): AuthoringServices | null {
  return services;
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

/** Whether the "Save as Template" dialog is open (spec §8; WP-B3). */
let templateOpen = false;

export function openSaveAsTemplate(): void {
  templateOpen = true;
  publish();
}

export function closeSaveAsTemplate(): void {
  if (!templateOpen) return;
  templateOpen = false;
  publish();
}

export function isSaveAsTemplateOpen(): boolean {
  return templateOpen;
}

export function useSaveAsTemplateOpen(): boolean {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    isSaveAsTemplateOpen,
    () => false,
  );
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
