/*
 * The reference addon's OWN domain.
 *
 * Deliberately not a body, a sketch or a datum. An addon that decorated modeling
 * entities would pass every host check by accident — the hosts already know how
 * to render those. A "widget" is a thing OneCAD has never heard of, which is the
 * only way to find the places where a host still assumes it is drawing modeling.
 *
 * State lives in memory here and is mirrored into the document through
 * `context.document.state` on every change, which is also the point: the addon
 * has no store, no IPC, and no schema anyone else can read.
 */
export interface Widget {
  readonly id: string;
  name: string;
  enabled: boolean;
}

export interface WidgetState {
  readonly widgets: readonly Widget[];
  readonly selectedId: string | null;
  /** The addon's tool is armed. A tool is a MODE, so it has to leave a trace. */
  readonly placing: boolean;
}

export const WIDGET_SCHEMA_VERSION = 1;

const SEED: readonly Widget[] = [
  { id: "w1", name: "Widget A", enabled: true },
  { id: "w2", name: "Widget B", enabled: false },
];

/** A tiny observable store. No zustand — the SDK does not ship one, on purpose. */
export function createWidgetStore(seed: readonly Widget[] = SEED) {
  let state: WidgetState = { widgets: seed.map((w) => ({ ...w })), selectedId: null, placing: false };
  const listeners = new Set<() => void>();

  const set = (next: WidgetState): void => {
    state = next;
    for (const l of [...listeners]) l();
  };

  return {
    get: () => state,
    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    select(id: string | null): void {
      set({ ...state, selectedId: id });
    },
    setPlacing(placing: boolean): void {
      set({ ...state, placing });
    },
    toggle(id: string): void {
      set({
        ...state,
        widgets: state.widgets.map((w) => (w.id === id ? { ...w, enabled: !w.enabled } : w)),
      });
    },
    rename(id: string, name: string): void {
      set({
        ...state,
        widgets: state.widgets.map((w) => (w.id === id ? { ...w, name } : w)),
      });
    },
    add(widget: Widget): void {
      set({ ...state, widgets: [...state.widgets, widget] });
    },
    replace(next: WidgetState): void {
      set(next);
    },
  };
}

export type WidgetStore = ReturnType<typeof createWidgetStore>;
