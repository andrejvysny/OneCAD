/*
 * The reference addon — an ARCHITECTURAL FIXTURE, not a product feature.
 *
 * It exists to answer one question with running code: can something that imports
 * `@onecad/sdk` and nothing else contribute every open extension point? Every
 * host that still assumes it is drawing modeling shows up here as a crash, an
 * empty region, or a row that never updates.
 *
 * Its import list is the assertion (`architecture.test.ts` enforces it):
 *   allowed   @onecad/sdk · react · three
 *   forbidden @/platform · @/modules · @/features · @/stores · @/tools ·
 *             @/viewport · @/ipc · @/app
 *
 * Everything below therefore goes through `ExtensionContext`. There is no
 * `platform`, no registry object, no `CadClient` and no store in reach.
 */
import {
  addonId,
  contributionId,
  Slots,
  type AddonId,
  type CommandId,
  type Disposable,
  type ExtensionContext,
  type ExtensionDefinition,
  type InspectorContributionId,
  type PanelId,
  type ToolId,
  type TreeProviderId,
  type ViewportContext,
  type ViewportContributionId,
  type WorkspaceId,
} from "@onecad/sdk";
import {
  createWidgetStore,
  WIDGET_SCHEMA_VERSION,
  type Widget,
  type WidgetState,
} from "./domain";
import { widgetInspectorSection, widgetPanel } from "./ui";

export const REFERENCE_ADDON_ID: AddonId = addonId("com.onecadtest.reference");

/** Its own type id — never a modeling `body`/`sketch`/`datum`. */
export const WIDGET_TYPE = "com.onecadtest.reference.item";

/**
 * Every id is minted through `contributionId`, which REFUSES one outside the
 * addon's namespace — the check that makes "an addon cannot register `onecad.*`"
 * a runtime fact instead of a convention (ADR-0006).
 */
export const ReferenceIds = {
  command: contributionId<CommandId>(
    REFERENCE_ADDON_ID,
    "com.onecadtest.reference.command.addWidget",
  ),
  tool: contributionId<ToolId>(REFERENCE_ADDON_ID, "com.onecadtest.reference.tool.place"),
  panel: contributionId<PanelId>(REFERENCE_ADDON_ID, "com.onecadtest.reference.panel.widgets"),
  inspector: contributionId<InspectorContributionId>(
    REFERENCE_ADDON_ID,
    "com.onecadtest.reference.inspector.widget",
  ),
  tree: contributionId<TreeProviderId>(
    REFERENCE_ADDON_ID,
    "com.onecadtest.reference.tree.widgets",
  ),
  viewport: contributionId<ViewportContributionId>(
    REFERENCE_ADDON_ID,
    "com.onecadtest.reference.viewport.markers",
  ),
  workspace: contributionId<WorkspaceId>(
    REFERENCE_ADDON_ID,
    "com.onecadtest.reference.workspace.widgets",
  ),
} as const;

/**
 * `seed` exists for the fixture's own tests — an addon that always starts with
 * widgets can never exercise its own "nothing to place yet" state.
 */
export function createReferenceAddon(seed?: readonly Widget[]): ExtensionDefinition {
  const store = createWidgetStore(seed);

  /** Mirror the addon's state into ITS OWN document namespace. */
  const persist = (context: ExtensionContext): void => {
    void context.document.state.write<WidgetState>({
      schemaVersion: WIDGET_SCHEMA_VERSION,
      payload: store.get(),
    });
  };

  return {
    async activate(context) {
      // Load first: a document may already carry this addon's slice, written by
      // a previous session — or preserved verbatim while the addon was missing.
      const saved = await context.document.state.read<WidgetState>();
      if (saved?.schemaVersion === WIDGET_SCHEMA_VERSION) store.replace(saved.payload);

      context.registerCommand({
        id: ReferenceIds.command,
        title: "Add widget",
        group: "reference",
        defaultShortcut: { key: "w", shift: true },
        execute: () => {
          const n = store.get().widgets.length + 1;
          store.add({ id: `w${n}`, name: `Widget ${n}`, enabled: true });
          persist(context);
          return { status: "done" as const };
        },
      });

      context.registerTool({
        id: ReferenceIds.tool,
        title: "Place widget",
        icon: "select",
        shortcutLabel: "⇧W",
        // No `scopes`: the toolbar's "empty ⇒ everywhere" rule is what lets an
        // addon appear without knowing another module's scope tokens.
        canActivate: () => ({
          enabled: store.get().widgets.length > 0,
          reason: "Add a widget first",
        }),
        subscribe: (onChange) => ({ dispose: store.subscribe(onChange) }),
        // A tool is a MODE, so activation has to leave a trace something can
        // observe — otherwise the fixture would "pass" while doing nothing.
        activate: () => store.setPlacing(true),
        deactivate: () => store.setPlacing(false),
      });

      context.registerPanel({
        id: ReferenceIds.panel,
        slot: Slots.ShellRight,
        title: "Widgets",
        priority: 500,
        component: widgetPanel(store),
      });

      context.registerInspectorSection({
        id: ReferenceIds.inspector,
        priority: 500,
        canRender: (ctx) => ctx.selection.some((ref) => ref.typeId === WIDGET_TYPE),
        component: widgetInspectorSection(store),
      });

      context.registerTreeProvider({
        id: ReferenceIds.tree,
        priority: 500,
        sections: () => {
          const { widgets, selectedId } = store.get();
          return [
            {
              id: `${REFERENCE_ADDON_ID}.tree.section`,
              title: "Widgets",
              nodes: widgets.map((w) => ({
                id: w.id,
                label: w.name,
                icon: "cube",
                kind: WIDGET_TYPE,
                selected: w.id === selectedId,
                visible: w.enabled,
                select: () => store.select(w.id),
                toggleVisible: () => {
                  store.toggle(w.id);
                  persist(context);
                },
                rename: (next) => {
                  store.rename(w.id, next);
                  persist(context);
                },
                actions: [
                  {
                    id: `${REFERENCE_ADDON_ID}.action.add`,
                    title: "Add another",
                    commandId: ReferenceIds.command,
                    group: "reference",
                  },
                ],
              })),
            },
          ];
        },
        // The host watches nothing on the addon's behalf (ADR-0012).
        subscribe: (onChange) => ({ dispose: store.subscribe(onChange) }),
      });

      context.registerViewportContribution({
        id: ReferenceIds.viewport,
        priority: 500,
        attach: (viewport: ViewportContext): Disposable => {
          // A real layer would add objects to `viewport.root`. This one only
          // proves the lifecycle: it repaints when its own state moves and when
          // the theme changes, and gives everything back on dispose.
          const stop = store.subscribe(() => viewport.invalidate());
          const theme = viewport.onThemeChange(() => viewport.invalidate());
          return {
            dispose: () => {
              stop();
              theme.dispose();
            },
          };
        },
      });

      context.registerWorkspace({
        id: ReferenceIds.workspace,
        title: "Widgets",
        panels: [{ panelId: ReferenceIds.panel, slot: Slots.ShellRight }],
      });
    },

    deactivate: () => store.setPlacing(false),
  };
}
