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
import { LibraryCommands, LibraryMenuItems, LibraryPanels, LibraryTools } from "./panelIds";
import { LibraryInspectorPriorities, LibraryInspectorSections } from "./inspectorSectionIds";
import { LibraryModalHost } from "@/features/library/LibraryModalHost";
import { LibraryStatusSection } from "@/features/library/LibraryStatusSection";
import { ComponentParametersSection } from "@/features/library/ComponentParametersSection";
import { libraryModalStore } from "./libraryModalStore";
import { configurePlacementController } from "./placementController";
import {
  configureAuthoringController,
  openSaveAsComponent,
  openSaveAsTemplate,
} from "./authoringController";
import { SaveAsComponentHost } from "@/features/library/SaveAsComponentHost";
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
 * Registers the library's editor UI. The library grid/search/detail no
 * longer sits in `Slots.ShellLeft` beside modeling's `ModelTreePanel` — it
 * lives in the full-size `LibraryModal`, opened by the `OpenLibrary` toolbar
 * tool below. The sidebar's left column is modeling's alone now (Variables
 * takes the tab slot Library used to share — see `modules/modeling/ui.ts`).
 */
export function contributeLibraryUi(scope: ModuleScope): void {
  // The modal itself, mounted once and toggled by `libraryModalStore` — same
  // `Slots.ShellOverlay` pattern `SaveAsComponentHost` below uses.
  scope.registerPanel({
    id: LibraryPanels.LibraryModalHost,
    slot: Slots.ShellOverlay,
    title: "Library",
    // AFTER SaveAsComponentHost (140): opened from the toolbar, so it is
    // never the most-recently-requested overlay at mount time.
    priority: 150,
    component: LibraryModalHost,
  });

  // The toolbar's own "insert" group — deliberately its own group/priority
  // band, lower than every model-editing tool (`model.create` starts at
  // 100), so it sorts first and gets its own separator: placing a component
  // is an insertion, not a modelling op like Select/Sketch/Extrude.
  scope.registerTool({
    id: LibraryTools.OpenLibrary,
    title: "Library",
    icon: "cube",
    group: "library.insert",
    priority: 10,
    scopes: [ModelingScopes.Model],
    activate: () => libraryModalStore.getState().open(),
    deactivate: () => libraryModalStore.getState().close(),
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

  // WP-B2: "Save as Component" — a menu contribution on a BODY row plus the
  // overlay that owns its dialog. The row belongs to modeling's tree provider
  // and the item belongs here; `platform.menus` (WP-B1) is what lets those be
  // two different modules without either importing the other (ADR-0002).
  scope.registerMenuItem({
    id: LibraryMenuItems.SaveAsComponent,
    slot: Slots.TreeContext,
    title: "Save as Component…",
    appliesTo: (target) => target.kind === "body",
    run: (target) => openSaveAsComponent({ bodyId: target.id, bodyName: target.label }),
  });

  // WP-B3: "Save as Template…" is a COMMAND, not a File-menu item — the File
  // menu lives in `features/shell`, and the shell must not import the library
  // to offer a library action. The palette lists every registered command, so
  // a command is the module-owned way in.
  scope.registerCommand({
    id: LibraryCommands.SaveAsTemplate,
    title: "Save as Template…",
    keywords: ["template", "starter", "new project"],
    execute: () => {
      openSaveAsTemplate();
      return { status: "done" };
    },
  });

  scope.registerPanel({
    id: LibraryPanels.SaveAsComponentHost,
    slot: Slots.ShellOverlay,
    title: "Save as component",
    // AFTER the four shell modals (100-130), which is what `shellContract.ts`
    // freezes. Overlay order is stacking order, and a dialog opened from a
    // context menu is the most recent thing the user asked for.
    priority: 140,
    component: SaveAsComponentHost,
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

  // The attachment picker (WP-F1.2) needs the same classification service the
  // placement gesture does — a pick has to become a seatable frame before it can
  // be authored as an attachment. Injected on the same soft-lookup terms: a
  // harness without modeling's bootstrap services simply cannot start a pick.
  configureAuthoringController(geometryQuery ? { geometryQuery } : null);
  scope.own({ dispose: () => configureAuthoringController(null) });
}

export function registerLibraryModule(platform: Platform): void {
  platform.registerModule({
    id: LIBRARY_MODULE_ID,
    version: "1.0.0",
    activate: (scope) => contributeLibrary(scope),
  });
}
