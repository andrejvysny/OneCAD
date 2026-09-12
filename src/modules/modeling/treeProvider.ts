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
import type { CommandId, Disposable, ModuleScope, TreeNodeAction, TreeSection } from "@/platform";
import { ModelingTreeProvider } from "./panelIds";
import { ModelingTreeCommands } from "./ids";
import { documentStore } from "@/stores/documentStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { getViewportEngine } from "@/viewport/engineBridge";
import { requestTreeReveal, type TreeNodeLocator } from "@/features/tree/treeReveal";
import {
  deleteDatum,
  deleteSketch,
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

const isHovered = (kind: TreeRowKind, id: string): boolean => {
  const hover = selectionStore.getState().hover;
  if (!hover) return false;
  if (hover.kind === kind) return hover.id === id;
  if (kind === "body") {
    return (
      (hover.kind === "face" || hover.kind === "edge" || hover.kind === "vertex") &&
      hover.bodyId === id
    );
  }
  return kind === "sketch" && hover.kind === "sketchRegion" && hover.sketchId === id;
};

/** Return an exact lease so an old row cannot clear a newer hover target. */
const startHover = (kind: TreeRowKind, id: string) => () => {
  const ref: EntityRef = { kind, id };
  selectionStore.getState().setHover(ref);
  return {
    dispose: () => {
      if (selectionStore.getState().hover === ref) selectionStore.getState().setHover(null);
    },
  };
};

const revealTarget = (): TreeNodeLocator | null => {
  const ref = selectionStore.getState().selected[0];
  if (!ref) return null;
  const document = documentStore.getState();
  if (ref.kind === "body" && document.bodies[ref.id]) {
    return { providerId: ModelingTreeProvider, nodeId: ref.id };
  }
  if (ref.kind === "sketch" && document.sketches[ref.id]) {
    return { providerId: ModelingTreeProvider, nodeId: ref.id };
  }
  if (ref.kind === "datum" && document.datums[ref.id]) {
    return { providerId: ModelingTreeProvider, nodeId: ref.id };
  }
  if (
    (ref.kind === "face" || ref.kind === "edge" || ref.kind === "vertex") &&
    ref.bodyId &&
    document.bodies[ref.bodyId]
  ) {
    return { providerId: ModelingTreeProvider, nodeId: ref.bodyId };
  }
  return ref.kind === "sketchRegion" && document.sketches[ref.sketchId]
    ? { providerId: ModelingTreeProvider, nodeId: ref.sketchId }
    : null;
};

const isCurrentVisibleBody = (id: string): boolean => {
  const document = documentStore.getState();
  return (
    document.status === "ready" &&
    document.geometrySource === "live" &&
    !viewportStore.getState().geometryPending &&
    document.bodies[id]?.visible === true
  );
};

/** Bound preflight prevents ViewportEngine.fitToBodies from fitting all on a miss. */
const canFrameBody = (id: string): boolean => {
  if (!isCurrentVisibleBody(id)) return false;
  const engine = getViewportEngine();
  if (!engine) return false;
  return engine.getBoundsForBodies([id]) !== null;
};

const frameBody = (id: string): boolean => {
  if (!isCurrentVisibleBody(id)) return false;
  const engine = getViewportEngine();
  if (!engine?.getBoundsForBodies([id])) return false;
  engine.fitToBodies([id]);
  return true;
};

const frameAction: TreeNodeAction = {
  id: `${ModelingTreeCommands.frameBody}.action`,
  title: "Frame in viewport",
  commandId: ModelingTreeCommands.frameBody as CommandId,
};

/**
 * Delete, as a declared action rather than a `kind === "datum"` branch in the
 * panel. `confirm` keeps the shipped two-click idiom — destructive, and the tree
 * has no undo affordance in reach — without the host knowing what a datum is.
 */
const deleteAction = (title: string, commandId: string): TreeNodeAction => ({
  id: `${commandId}.action`,
  title,
  commandId: commandId as CommandId,
  danger: true,
  group: "danger",
  confirm: "Confirm delete",
});

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
        ...(b.health === "quarantined"
          ? { meta: "Quarantined", problem: true }
          : {}),
        selected: isSelected("body", b.id),
        hovered: isHovered("body", b.id),
        visible: b.visible,
        // Isolated AWAY → dim the row. The eye still reports the document's own
        // visibility; isolation is a transient viewport mask.
        dimmed: isolated !== null && !isolated.includes(b.id),
        select: select("body", b.id),
        startHover: startHover("body", b.id),
        ...(canFrameBody(b.id) ? { actions: [frameAction] } : {}),
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
        hovered: isHovered("sketch", s.id),
        visible: s.visible,
        select: select("sketch", s.id),
        startHover: startHover("sketch", s.id),
        activate: () => setMode("sketch", s.id),
        toggleVisible: (v) => void setSketchVisible(s.id, v),
        rename: (name) => void renameSketch(s.id, name),
        actions: [deleteAction("Delete sketch", ModelingTreeCommands.deleteSketch)],
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
        hovered: isHovered("datum", d.id),
        select: select("datum", d.id),
        startHover: startHover("datum", d.id),
        activate: () => {
          // Select FIRST, then flip: SketchController's
          // `tryEnterOnSelectedDatum` reads the selection when the mode changes.
          selectionStore.getState().set([{ kind: "datum", id: d.id }]);
          setMode("sketch");
        },
        // No `rename`: DATUM W1 ships no RenameDatum command, so offering the
        // affordance would be a dead end.
        actions: [deleteAction("Delete datum", ModelingTreeCommands.deleteDatum)],
      })),
    },
  ];
}

/**
 * Announce that `sections()` would now project something different.
 *
 * The panel used to hold five modeling store subscriptions of its own "so that
 * the provider's reads are fresh" — a generic host watching one module's stores
 * on that module's behalf, which no second provider could ever benefit from.
 * Every provider now says when its own rows moved, and the host watches nothing.
 */
function subscribeToProjection(onChange: () => void): Disposable {
  const unsubscribes = [
    documentStore.subscribe(onChange),
    selectionStore.subscribe(onChange),
    viewportStore.subscribe(onChange),
  ];
  return {
    dispose: () => {
      for (const u of unsubscribes) u();
    },
  };
}

/** The row actions, as real commands: reachable from a palette, not only a menu. */
function contributeTreeCommands(scope: ModuleScope): void {
  const selectedId = (kind: TreeRowKind): string | undefined =>
    selectionStore.getState().selected.find((r) => r.kind === kind)?.id;

  scope.registerCommand({
    id: ModelingTreeCommands.deleteSketch as CommandId,
    title: "Delete sketch",
    group: "modeling.tree",
    canExecute: () => ({
      enabled: selectedId("sketch") !== undefined,
      reason: "Select a sketch first",
    }),
    execute: () => {
      const id = selectedId("sketch");
      if (!id) return { status: "cancelled" as const };
      void deleteSketch(id);
      return { status: "done" as const };
    },
  });

  scope.registerCommand({
    id: ModelingTreeCommands.revealSelection as CommandId,
    title: "Reveal selected entity in tree",
    group: "modeling.tree",
    canExecute: () => ({
      enabled: revealTarget() !== null,
      reason: "Select a body, sketch, datum, or subelement first",
    }),
    execute: () => {
      const target = revealTarget();
      if (!target) return { status: "cancelled" as const };
      return requestTreeReveal(target) ? { status: "done" as const } : { status: "cancelled" as const };
    },
  });

  scope.registerCommand({
    id: ModelingTreeCommands.frameBody as CommandId,
    title: "Frame body in viewport",
    group: "modeling.tree",
    canExecute: () => {
      const id = selectedId("body");
      return {
        enabled: id !== undefined && canFrameBody(id),
        reason: "Select a body with current visible geometry",
      };
    },
    execute: () => {
      const id = selectedId("body");
      return id && frameBody(id) ? { status: "done" as const } : { status: "cancelled" as const };
    },
  });

  scope.registerCommand({
    id: ModelingTreeCommands.deleteDatum as CommandId,
    title: "Delete datum",
    group: "modeling.tree",
    canExecute: () => ({
      enabled: selectedId("datum") !== undefined,
      reason: "Select a datum first",
    }),
    execute: () => {
      const id = selectedId("datum");
      if (!id) return { status: "cancelled" as const };
      // The backend refuses while a sketch is hosted on the datum; `treeActions`
      // surfaces that rejection as a sticky hint.
      void deleteDatum(id);
      return { status: "done" as const };
    },
  });
}

/** Registers modeling's rows. Exported so a test can drive the real registration. */
export function contributeModelingTree(scope: ModuleScope): void {
  contributeTreeCommands(scope);
  scope.registerTreeProvider({
    id: ModelingTreeProvider,
    priority: 100,
    sections: modelingTreeSections,
    subscribe: subscribeToProjection,
  });
}
