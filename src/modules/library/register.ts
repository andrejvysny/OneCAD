/*
 * `registerLibraryModule(platform)` — the Component Library's contribution
 * surface. Mirrors `modules/modeling/register.ts`'s bootstrap/editor-mount
 * split exactly (docs/ARCHITECTURE.md §6):
 *
 *   `contributeLibrary`   — bootstrap-time (tools/commands/services this
 *                            module owns for its whole lifetime). P1 has
 *                            none yet — kept as the hook P2+ (a "place
 *                            selected component" shortcut, say) lands in,
 *                            and so `platform.moduleIds()` reports
 *                            `onecad.library` as present even before the
 *                            editor mounts.
 *   `contributeLibraryUi` — editor-mount-time (panels/tree/viewport). Called
 *                            from `EditorShell.tsx` alongside modeling's own
 *                            UI scope, disposed on unmount.
 */
import type { InspectorContext, ModuleScope, Platform } from "@/platform";
import { Slots } from "@/platform";
import { LIBRARY_MODULE_ID } from "./manifest";
import { LibraryPanels } from "./panelIds";
import { LibraryInspectorPriorities, LibraryInspectorSections } from "./inspectorSectionIds";
import { LibraryPanel } from "@/features/library/LibraryPanel";
import { LibraryStatusSection } from "@/features/library/LibraryStatusSection";
import { ComponentParametersSection } from "@/features/library/ComponentParametersSection";
import { configurePlacementController } from "./placementController";
import {
  ModelingScopes,
  ModelingServices,
  type CommandApiService,
  type GeometryQueryService,
} from "@/modules/modeling/manifest";

/** Bootstrap-time contributions. Empty in P1 — see the module doc comment. */
export function contributeLibrary(_scope: ModuleScope): void {
  // No tools/commands/services of its own yet.
}

/**
 * Registers the library's editor UI. `ShellLeft` already hosts modeling's
 * `ModelTreePanel` (priority 100, full-bleed `left:0` sidebar) — this joins
 * it at priority 110 (mounts AFTER, per `shellContract.ts`'s pinned order)
 * rather than a competing slot: both panels render into the SAME physical
 * space and use `sidebarTabStore` (a shared, tiny UI-only store — no
 * document/document-module state) to decide which one shows its content,
 * the same "one region, several tab-like occupants" pattern a VS Code-style
 * sidebar uses. Neither panel's OWN registration/positioning changed to make
 * this work — `ModelTreePanel` only gained a `SidebarTabHeader` + an early
 * return when it isn't the active tab.
 */
export function contributeLibraryUi(scope: ModuleScope): void {
  scope.registerPanel({
    id: LibraryPanels.LibraryPanel,
    slot: Slots.ShellLeft,
    title: "Library",
    priority: 110,
    component: LibraryPanel,
  });

  // WP-1.6: the tasks-chip status row's first real StatusSection producer.
  scope.registerPanel({
    id: LibraryPanels.StatusSection,
    slot: Slots.StatusSection,
    title: "Library",
    priority: 100,
    component: LibraryStatusSection,
  });

  // WP-2.4: the configurator. `canRender` can only see "a feature is
  // selected in Model mode" (platform selection refs carry no `opType`,
  // ADR-0002) — the component itself decides whether that feature is a
  // PlaceComponent and renders null otherwise, same data-driven pattern
  // modeling's own Dimensions/Dependencies sections use.
  scope.registerInspectorSection({
    id: LibraryInspectorSections.ComponentParameters,
    title: "Component Parameters",
    priority: LibraryInspectorPriorities.ComponentParameters,
    canRender: (ctx: InspectorContext) =>
      ctx.scopes.includes(ModelingScopes.Model) && ctx.selection[0]?.typeId === "feature",
    component: ComponentParametersSection,
  });

  // The placement gesture (WP-1.5) needs modeling's published services
  // (ADR-0002: the kernel touch routes through modeling, never a direct
  // `CadClient` call from library code). Injected here rather than resolved
  // lazily inside the controller. SOFT lookup, not `require`: some harnesses
  // mount library's UI without modeling's bootstrap-time service
  // registration (e.g. `editorMountOrder.golden.test.ts` calls
  // `contributeModelingUi`, not `contributeModeling`) — a missing service
  // there should leave placement quietly unarmed, not fail editor mount.
  const geometryQuery = scope.platform.services.get<GeometryQueryService>(
    ModelingServices.GeometryQuery,
  );
  const commandApi = scope.platform.services.get<CommandApiService>(ModelingServices.CommandApi);
  configurePlacementController(geometryQuery && commandApi ? { geometryQuery, commandApi } : null);
  scope.own({ dispose: () => configurePlacementController(null) });
}

export function registerLibraryModule(platform: Platform): void {
  platform.registerModule({
    id: LIBRARY_MODULE_ID,
    version: "1.0.0",
    activate: (scope) => contributeLibrary(scope),
  });
}
