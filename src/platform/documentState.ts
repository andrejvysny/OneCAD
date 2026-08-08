/*
 * Module-owned document state, platform side (ADR-0004 / ADR-0005).
 *
 * A module reads and writes ITS OWN slice through this service and never touches
 * another module's. Writes go through the backend's transaction path, so a
 * programmatic change is as undoable as a user edit.
 *
 * The payload is opaque here too: this file has no schema for it and never
 * inspects it. Typing is the owning module's job, at its own boundary.
 */
import type { CadClient } from "@/ipc/client";
import type { DocumentModule } from "@/ipc/types";
import type { ModuleId } from "./ids";

export interface ModuleDocumentState<T = unknown> {
  readonly schemaVersion: number;
  readonly payload: T;
}

export interface DocumentStateService {
  /** This module's slice, or `null` when it has none. */
  read<T = unknown>(): Promise<ModuleDocumentState<T> | null>;
  /** Replace this module's slice. */
  write<T = unknown>(state: ModuleDocumentState<T>): Promise<void>;
  /** Remove this module's slice entirely. */
  clear(): Promise<void>;
}

/**
 * The service handed to ONE module. The module id is bound here rather than
 * passed per call, so a module cannot address another module's slice by
 * accident (docs/ARCHITECTURE.md §4).
 */
export function createDocumentStateService(
  client: CadClient,
  owner: ModuleId,
): DocumentStateService {
  return {
    async read<T>() {
      const state = await client.getModuleState(owner);
      return state === null
        ? null
        : { schemaVersion: state.schemaVersion, payload: state.payload as T };
    },
    write: (state) =>
      client.setModuleState(owner, {
        schemaVersion: state.schemaVersion,
        payload: state.payload,
      }),
    clear: () => client.setModuleState(owner, null),
  };
}

export interface MissingModule extends DocumentModule {
  /** True when this build has no module registered under that id. */
  readonly missing: true;
}

/**
 * Which of a document's modules are not installed.
 *
 * A missing module is a NOTICE, never a refusal to open: its state is preserved
 * and reported, because the alternative is silently discarding a user's data
 * (ADR-0005, spec §43).
 */
export function missingModules(
  documentModules: readonly DocumentModule[],
  registered: readonly string[],
): MissingModule[] {
  const known = new Set(registered);
  return documentModules
    .filter((m) => !known.has(m.moduleId))
    .map((m) => ({ ...m, missing: true as const }));
}
