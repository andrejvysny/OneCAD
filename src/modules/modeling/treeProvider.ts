/*
 * Modeling's model-tree rows, as a `TreeProvider`.
 *
 * The panel used to hardcode three sections and reach into three stores for
 * each. It now renders whatever providers supply, and modeling is one of them —
 * the seam a second module (FEM results, a drawing list) needs in order to put
 * rows in the same tree without editing the panel.
 *
 * `sections()` READS the stores rather than subscribing: the provider is a
 * projection, and the HOST owns re-rendering (its own store subscriptions are
 * what make these reads fresh). Keeping the subscription on the host side is
 * what lets the shape stay a plain function instead of a hook.
 *
 * The three sections keep their per-kind differences exactly as shipped:
 *   bodies  — dim when isolated away, no double-click action
 *   sketches— double-click re-opens that sketch
 *   datums  — no visibility fact at all, double-click starts a sketch on it
 */
import type { ModuleScope, TreeSection } from "@/platform";
import { ModelingTreeProvider } from "./panelIds";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import {
  renameBody,
  renameSketch,
  setBodyVisible,
  setSketchVisible,
} from "@/features/tree/treeActions";

/** The row kinds this provider mints — a subset of `EntityKind` with no sub-element form. */
type TreeRowKind = "body" | "sketch" | "datum";

const isSelected = (kind: TreeRowKind, id: string): boolean =>
  selectionStore.getState().selected.some((r) => r.kind === kind && r.id === id);

const select = (kind: TreeRowKind, id: string) => () => {
  selectionStore.getState().set([{ kind, id }]);
};

export function modelingTreeSections(): readonly TreeSection[] {
  const { bodies, sketches, datums } = documentStore.getState();
  const isolated = viewportStore.getState().isolatedBodyIds;
  const setMode = toolStore.getState().setMode;

  return [
    {
      id: "onecad.modeling.tree.bodies",
      title: "Bodies",
      nodes: Object.values(bodies).map((b) => ({
        id: b.id,
        label: b.name,
        icon: "cube",
        kind: "body",
        selected: isSelected("body", b.id),
        visible: b.visible,
        // Isolated AWAY → dim the row. The eye still reports the document's own
        // visibility; isolation is a transient viewport mask.
        dimmed: isolated !== null && !isolated.includes(b.id),
        select: select("body", b.id),
        toggleVisible: (v) => void setBodyVisible(b.id, v),
        rename: (name) => void renameBody(b.id, name),
      })),
    },
    {
      id: "onecad.modeling.tree.sketches",
      title: "Sketches",
      nodes: Object.values(sketches).map((s) => ({
        id: s.id,
        label: s.name,
        icon: "pen",
        kind: "sketch",
        selected: isSelected("sketch", s.id),
        visible: s.visible,
        select: select("sketch", s.id),
        activate: () => setMode("sketch", s.id),
        toggleVisible: (v) => void setSketchVisible(s.id, v),
        rename: (name) => void renameSketch(s.id, name),
      })),
    },
    {
      id: "onecad.modeling.tree.datums",
      title: "Datums",
      nodes: Object.values(datums).map((d) => ({
        id: d.id,
        label: d.name,
        icon: "datum",
        kind: "datum",
        // No `visible` key at all: a datum carries no visibility fact in the
        // document, so the host renders no eye (absent ≠ false).
        selected: isSelected("datum", d.id),
        select: select("datum", d.id),
        activate: () => {
          // Select FIRST, then flip: SketchController's
          // `tryEnterOnSelectedDatum` reads the selection when the mode changes.
          selectionStore.getState().set([{ kind: "datum", id: d.id }]);
          setMode("sketch");
        },
        // No `rename`: DATUM W1 ships no RenameDatum command, so offering the
        // affordance would be a dead end.
      })),
    },
  ];
}

/** Registers modeling's rows. Exported so a test can drive the real registration. */
export function contributeModelingTree(scope: ModuleScope): void {
  scope.registerTreeProvider({
    id: ModelingTreeProvider,
    priority: 100,
    sections: modelingTreeSections,
  });
}
