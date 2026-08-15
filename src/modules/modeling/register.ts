/*
 * `registerModelingModule(platform)` — modeling's contribution surface.
 *
 * The shell does not know Extrude, Fillet or the sketch toolbar; it knows slots
 * and registries. This is the other half of that inversion: everything modeling
 * puts on screen or into a keystroke is declared here (docs/ARCHITECTURE.md §6).
 *
 * Behavior is DELEGATED, never reimplemented. A registered tool's `activate`
 * calls the same `activateTool` dispatcher the toolbar and the keyboard already
 * use, and a command calls the same `runAction`, so a registry-driven invocation
 * and a click cannot diverge (spec §172).
 */
import { activateTool } from "@/tools/activateTool";
import { runAction } from "@/shortcuts/useShortcuts";
import type { ShortcutAction } from "@/shortcuts/keymap";
import { createClient } from "@/ipc/client";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { activeTool, toolStore, type ModelTool, type SketchTool } from "@/stores/toolStore";
import { getToolApplicability } from "@/tools/modelTools/toolApplicability";
import type {
  CommandDefinition,
  Disposable,
  ModuleScope,
  Platform,
  Shortcut,
  ToolDefinition,
} from "@/platform";
import {
  MODELING_MODULE_ID,
  ModelingScopes,
  ModelingServices,
  type CommandApiService,
  type GeometryQueryService,
} from "./manifest";
import {
  modelToolId,
  sketchToolId,
  ModelingCommands,
  ModelingModelTools,
  ModelingSketchTools,
  type ModelingCommandKey,
} from "./ids";
import {
  MODELING_TOOL_DESCRIPTORS,
  type ModelingToolDescriptor,
  type ToolScope,
} from "./tools";
import { bindingForTool, MODELING_BINDINGS } from "./bindings";
import type { CommandId, ToolId } from "@/platform";

const scopeToken = (scope: ToolScope): string =>
  scope === "sketch" ? ModelingScopes.Sketch : ModelingScopes.Model;

function toolIdOf(d: ModelingToolDescriptor): ToolId {
  return d.scope === "sketch" ? sketchToolId(d.tool) : modelToolId(d.tool);
}

function shortcutOf(d: ModelingToolDescriptor): Shortcut | undefined {
  const binding = bindingForTool(d.tool, d.scope);
  if (!binding) return undefined;
  return binding.shift === undefined
    ? { key: binding.key }
    : { key: binding.key, shift: binding.shift };
}

/**
 * One store subscription behind every tool's `subscribe`.
 *
 * Availability is a function of the selection and of which sketches exist. Wiring
 * each of the ~30 tools to both stores directly would install 60 subscriptions
 * for one fact; this installs two, lazily, and drops them when the last listener
 * goes — so a headless test that never renders the toolbar touches no store.
 */
const availability = (() => {
  const listeners = new Set<() => void>();
  let stop: (() => void) | undefined;

  const fire = () => {
    for (const l of [...listeners]) l();
  };

  return {
    listen(onChange: () => void): Disposable {
      listeners.add(onChange);
      if (!stop) {
        const a = selectionStore.subscribe(fire);
        const b = documentStore.subscribe(fire);
        stop = () => {
          a();
          b();
        };
      }
      return {
        dispose: () => {
          listeners.delete(onChange);
          if (listeners.size === 0 && stop) {
            stop();
            stop = undefined;
          }
        },
      };
    },
  };
})();

function toolDefinition(d: ModelingToolDescriptor): ToolDefinition {
  return {
    id: toolIdOf(d),
    title: d.label,
    icon: d.icon,
    group: d.group,
    priority: d.priority,
    flyout: d.family,
    toolbarHidden: d.toolbarHidden,
    generatorsMenu: d.generatorsMenu,
    scopes: [scopeToken(d.scope)],
    defaultShortcut: shortcutOf(d),
    shortcutLabel: d.shortcut,
    // The applicability matrix, unchanged and unmoved — it is still the same
    // rule `ModelToolController`'s arm guards use, so a button cannot show
    // enabled and then fail on click. What changed is WHO asks: the toolbar
    // used to call `getToolApplicability` on a modeling `Tool` literal, which
    // no third-party tool could ever have. Reads modeling's own stores rather
    // than `ctx`: the platform's `SelectionRef` is a different currency, and
    // round-tripping through it would only add a lossy translation.
    //
    // MODEL SCOPE ONLY, and that is load-bearing rather than an optimisation.
    // Sketch tools are pure pointer gestures with no selection precondition, and
    // the matrix is keyed on the `Tool` union where `mirror` means the model
    // MirrorBody op — the very collision scope-qualified ids exist for
    // (`ids.ts`). Handing it a sketch literal disables the sketch Mirror tool
    // whenever no body is selected.
    ...(d.scope === "model"
      ? {
          canActivate: () =>
            getToolApplicability(d.tool, selectionStore.getState().selected, {
              sketches: documentStore.getState().sketches,
              bodies: documentStore.getState().bodies,
            }),
          // The answer above moves with the selection (and with which sketches
          // exist), neither of which a generic toolbar watches. One shared
          // emitter for all of modeling's tools, not one subscription per tool.
          subscribe: (onChange: () => void) => availability.listen(onChange),
        }
      : {}),
    // AUTO-MODE lives in `activateTool`: picking a sketch tool from model mode
    // enters sketch mode, and vice versa. Routing through it is what keeps a
    // registry activation identical to a toolbar click.
    activate: () => activateTool(d.tool),
    // Tools are exclusive and swapped by activating the next one; there is no
    // separate teardown today, and inventing one here would be a behavior change.
    deactivate: () => {},
  };
}

/** The non-tool actions, in the order their bindings are declared. */
const COMMAND_ACTIONS: Readonly<Record<ModelingCommandKey, { title: string; action: ShortcutAction }>> =
  {
    enterSketch: { title: "New sketch", action: { type: "enterSketch" } },
    finishSketch: { title: "Finish sketch", action: { type: "finishSketch" } },
    cancel: { title: "Cancel", action: { type: "cancel" } },
    deleteSketchSelection: {
      title: "Delete sketch selection",
      action: { type: "deleteSketchSelection" },
    },
    toggleConstruction: { title: "Construction geometry", action: { type: "toggleConstruction" } },
    isolate: { title: "Isolate selection", action: { type: "isolate" } },
  };

function commandDefinition(key: ModelingCommandKey, priority: number): CommandDefinition {
  const { title, action } = COMMAND_ACTIONS[key];
  const binding = MODELING_BINDINGS.find((b) => b.action.type === action.type);
  return {
    id: ModelingCommands[key] as CommandId,
    title,
    group: "modeling.action",
    priority,
    defaultShortcut:
      binding === undefined
        ? undefined
        : binding.shift === undefined
          ? { key: binding.key }
          : { key: binding.key, shift: binding.shift },
    scopes:
      binding === undefined || binding.scope === "global"
        ? undefined
        : [scopeToken(binding.scope)],
    execute: () => {
      runAction(action);
      return { status: "done" as const };
    },
  };
}

/** `toolStore`'s current tool as a registry id. */
function activeToolId(): ToolId {
  const state = toolStore.getState();
  const tool = activeTool(state);
  return (
    state.mode === "sketch"
      ? ModelingSketchTools[tool as SketchTool]
      : ModelingModelTools[tool as ModelTool]
  ) as ToolId;
}

/**
 * Keep the platform's active-tool id in step with `toolStore`.
 *
 * `toolStore` stays authoritative for modeling's own tools — AUTO-MODE, the Esc
 * ladder and every controller that arms itself write there, not here. This
 * REPORTS those writes so the host mirrors one truth instead of holding a second
 * one, which is what lets the toolbar highlight by `ToolId` without modeling and
 * the platform ever disagreeing about what is active.
 */
function bridgeActiveTool(scope: ModuleScope): void {
  const push = () => scope.platform.toolHost.report(activeToolId());
  push(); // the store already has a tool at activation
  const unsubscribe = toolStore.subscribe(push);
  scope.own({ dispose: unsubscribe });
}

/**
 * `ModelingServices.GeometryQuery`'s implementation — a thin `CadClient`
 * delegate (Component Library WP-0.1's first real service registration; see
 * `manifest.ts::GeometryQueryService`).
 */
const geometryQueryService: GeometryQueryService = {
  classifyElement: (bodyId, elementId, topoKey) =>
    createClient().classifyElement(bodyId, elementId, topoKey),
};

/**
 * `ModelingServices.CommandApi`'s implementation — a thin `CadClient`
 * delegate (Component Library WP-1.3's first real service registration; see
 * `manifest.ts::CommandApiService`).
 */
const commandApiService: CommandApiService = {
  // Forward EVERY argument. This seam once dropped `params` (WP-A3 auto-size):
  // the ghost previewed the sized screw through `source.params` while the
  // commit placed the generator default — exactly the preview/commit divergence
  // the tauriClient comment warns about.
  placeComponent: (componentId, componentVersion, translate, rotate, params, mate) =>
    createClient().placeComponent(componentId, componentVersion, translate, rotate, params, mate),
  detachComponent: (recordId) => createClient().detachComponent(recordId),
  beginPreview: (draft) => createClient().beginPreview(draft),
  updatePreview: (sessionId, params, epoch) => createClient().updatePreview(sessionId, params, epoch),
  endPreview: (sessionId, commit) => createClient().endPreview(sessionId, commit),
  onPreviewResult: (cb) => createClient().onPreviewResult(cb),
};

/** Registers every modeling contribution into `scope`. Exported for tests. */
export function contributeModeling(scope: ModuleScope): void {
  for (const d of MODELING_TOOL_DESCRIPTORS) scope.registerTool(toolDefinition(d));

  let priority = 100;
  for (const key of Object.keys(COMMAND_ACTIONS) as ModelingCommandKey[]) {
    scope.registerCommand(commandDefinition(key, priority));
    priority += 10;
  }

  scope.registerService(ModelingServices.GeometryQuery, geometryQueryService);
  scope.registerService(ModelingServices.CommandApi, commandApiService);

  bridgeActiveTool(scope);
}

export function registerModelingModule(platform: Platform): void {
  platform.registerModule({
    id: MODELING_MODULE_ID,
    version: "1.0.0",
    activate: (scope) => contributeModeling(scope),
  });
}
