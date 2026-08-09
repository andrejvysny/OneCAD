/*
 * Viewport LAYERS — the bottom-left Layers menu (prototype 2a).
 *
 * A layer is a transient view filter, not a document fact. Per-entity
 * visibility (the eye in the explorer) dispatches `SetVisibility` and is
 * persisted; these toggles hide a whole class of scene content for as long as
 * this session lasts and write nothing. Keeping the two apart is the point —
 * "I turned off sketches to see the part" must not survive into the file.
 *
 * The groups are DECLARED here rather than discovered from the registry
 * because a layer is only meaningful if the viewport can actually draw it:
 * `bodies`, `sketches` and `datums` map onto real scene roots
 * (`ViewportEngine.setLayerVisible`), and `grid` is `viewportStore`'s existing
 * toggle surfaced in the same menu instead of only on the corner cluster.
 *
 * A second group appears only when a module contributes one. There is no
 * simulation or add-on group today, and inventing empty ones would advertise
 * capabilities the build does not have.
 */
import { createStore, useStore } from "zustand";

/** The scene roots `ViewportEngine.setLayerVisible` understands. */
export type SceneLayer = "bodies" | "sketches" | "datums";
/** Every switch the menu shows, including the one `viewportStore` owns. */
export type LayerKey = SceneLayer | "grid";

export interface LayerDescriptor {
  readonly key: LayerKey;
  readonly label: string;
}

export interface LayerGroup {
  readonly id: string;
  readonly title: string;
  readonly layers: readonly LayerDescriptor[];
}

export const LAYER_GROUPS: readonly LayerGroup[] = [
  {
    id: "document",
    title: "Document",
    layers: [
      { key: "bodies", label: "Bodies" },
      { key: "sketches", label: "Sketches" },
      { key: "datums", label: "Datums" },
    ],
  },
  {
    id: "reference",
    title: "Reference",
    layers: [{ key: "grid", label: "Grid" }],
  },
];

export interface LayersState {
  open: boolean;
  /** Scene layers only. `grid` lives in `viewportStore` and is not duplicated. */
  visible: Record<SceneLayer, boolean>;

  setOpen(open: boolean): void;
  toggleOpen(): void;
  setVisible(layer: SceneLayer, visible: boolean): void;
}

export const layersStore = createStore<LayersState>((set) => ({
  open: false,
  visible: { bodies: true, sketches: true, datums: true },

  setOpen: (open) => set({ open }),
  toggleOpen: () => set((s) => ({ open: !s.open })),
  setVisible: (layer, visible) =>
    set((s) => ({ visible: { ...s.visible, [layer]: visible } })),
}));

export function useLayersStore<T>(selector: (s: LayersState) => T): T {
  return useStore(layersStore, selector);
}
