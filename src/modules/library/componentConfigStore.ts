/*
 * The configuration a user set on a component BEFORE placing it (LGU-1 canvas
 * D2-A step 3).
 *
 * Configure-then-place is the point of the detail panel: picking M8 there and
 * then watching the placement arm at M6 would make the panel a lie. So the
 * configuration is held here, keyed by `id@version`, and read back when that
 * component is armed (WP-C).
 *
 * DELIBERATELY NOT PERSISTED, and deliberately not document state. It is a
 * draft the user has not committed to anything yet — writing it to the document
 * would put a value nobody placed into a saved file, and writing it to disk
 * would resurrect last month's half-made choice on a cold start. It dies with
 * the session, which is exactly how long the intent lasts.
 */
import { createStore, useStore } from "zustand";
import type { ComponentParamValue } from "@/ipc/types";

type Config = Record<string, ComponentParamValue>;

interface ComponentConfigState {
  /** `"<id>@<version>"` → the free params the user has changed. */
  byComponent: Record<string, Config>;
  set(id: string, version: string, config: Config): void;
  get(id: string, version: string): Config | undefined;
  clear(id: string, version: string): void;
}

const key = (id: string, version: string): string => `${id}@${version}`;

export const componentConfigStore = createStore<ComponentConfigState>()((set, get) => ({
  byComponent: {},
  set(id, version, config) {
    set((s) => ({ byComponent: { ...s.byComponent, [key(id, version)]: config } }));
  },
  get(id, version) {
    return get().byComponent[key(id, version)];
  },
  clear(id, version) {
    set((s) => {
      const next = { ...s.byComponent };
      delete next[key(id, version)];
      return { byComponent: next };
    });
  },
}));

/** The carried configuration for one component, or `undefined`. */
export function useComponentConfig(id: string, version: string): Config | undefined {
  return useStore(componentConfigStore, (s) => s.byComponent[key(id, version)]);
}
