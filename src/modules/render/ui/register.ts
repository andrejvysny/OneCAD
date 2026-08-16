/*
 * `registerRenderUi(scope)` — every UI contribution `onecad.render` makes.
 *
 * Split from `module.ts` for the reason the other modules split theirs: that
 * file is the module's WIRING (state hydration, service publication, the
 * modeling services it consumes) and this one is its SURFACE. `module.ts` keeps
 * one import and one call.
 *
 * REGISTERED AT BOOTSTRAP, not at editor mount, unlike modeling's and library's
 * UI. Those two register on mount because the editor tree is a deliberate
 * code-split chunk and the start screen must not pay for it; render's UI is a
 * handful of components with no kernel or geometry imports, and its contributions
 * have to exist for the same lifetime as the store they read — the module owns
 * both, so they come and go together. Nothing here renders until a slot host
 * mounts it, so a bootstrap registration costs the start screen nothing but the
 * registry entries.
 *
 * WHAT IS DELIBERATELY NOT HERE: no tool, no command, no workspace. Assignment
 * is a direct manipulation (drag, dropdown, menu item), not a mode, so nothing
 * here belongs on the toolbar; the module's workspace home is still the shell's
 * `Visualization` placeholder.
 */
import { Slots, type InspectorContext, type ModuleScope } from "@/platform";
import { ModelingScopes } from "@/modules/modeling/manifest";

import {
  RenderInspectorPriorities,
  RenderInspectorSections,
  RenderMenuItems,
  RenderPanels,
} from "../ids";
import { installMaterialDropTarget } from "./assignDragDrop";
import { renderDialogStore } from "./dialogStore";
import { MaterialLibraryPanel } from "./MaterialLibraryPanel";
import { MaterialBodySection, MaterialFaceSection } from "./MaterialInspectorSection";
import { RenderDialogHost } from "./RenderDialogHost";

const primary = (ctx: InspectorContext) => ctx.selection[0];
const inModelMode = (ctx: InspectorContext) => ctx.scopes.includes(ModelingScopes.Model);

export function registerRenderUi(scope: ModuleScope): void {
  // The left-sidebar tab. 120 puts it after modeling's ModelTree (100) and
  // Variables (110) — the three share one region through `sidebarTabStore`, and
  // the tab strip's own order is `SidebarTabHeader`'s.
  scope.registerPanel({
    id: RenderPanels.MaterialLibrary,
    slot: Slots.ShellLeft,
    title: "Materials",
    priority: 120,
    component: MaterialLibraryPanel,
  });

  // AFTER library's overlays (140/150): every dialog here is raised by the
  // user's most recent gesture, so it is the most-recently-requested overlay by
  // construction.
  scope.registerPanel({
    id: RenderPanels.DialogHost,
    slot: Slots.ShellOverlay,
    title: "Materials",
    priority: 160,
    component: RenderDialogHost,
  });

  // Gated exactly the way modeling's Appearance sections are — same mode, same
  // kinds, same "a face with no promoted ElementId has nothing to key on"
  // rule, because a Material section that appeared where Appearance did not
  // would read as a bug in the panel rather than as a deliberate difference.
  scope.registerInspectorSection({
    id: RenderInspectorSections.MaterialBody,
    title: "Material",
    priority: RenderInspectorPriorities.Material,
    canRender: (ctx) => inModelMode(ctx) && primary(ctx)?.typeId === "body",
    component: MaterialBodySection,
  });
  scope.registerInspectorSection({
    id: RenderInspectorSections.MaterialFace,
    title: "Material",
    priority: RenderInspectorPriorities.Material,
    canRender: (ctx) =>
      inModelMode(ctx) && primary(ctx)?.typeId === "face" && !!primary(ctx)?.subElement,
    component: MaterialFaceSection,
  });

  // A body row belongs to MODELING's tree provider; this item belongs to
  // render. `platform.menus` is what lets those be two different modules
  // without either importing the other — the same seam the library's "Save as
  // Component…" uses, gating on the same `TreeNode.kind` vocabulary the
  // provider already publishes.
  scope.registerMenuItem({
    id: RenderMenuItems.AssignMaterial,
    slot: Slots.TreeContext,
    title: "Assign material…",
    appliesTo: (target) => target.kind === "body",
    run: (target) =>
      renderDialogStore.getState().openAssign({ bodyId: target.id, bodyLabel: target.label }),
  });

  // The viewport drop target. Window-level and engine-gated — see
  // `assignDragDrop.ts` for why it is not wired into `ViewportRoot`.
  const uninstallDrop = installMaterialDropTarget();
  scope.own({ dispose: uninstallDrop });

  // A module teardown must not leave a modal on screen, or an unanswered
  // question awaited forever by whoever raised it.
  scope.own({ dispose: () => renderDialogStore.getState().reset() });
}
