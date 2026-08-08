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
import type {
  CommandDefinition,
  ModuleScope,
  Platform,
  Shortcut,
  ToolDefinition,
} from "@/platform";
import { MODELING_MODULE_ID, ModelingScopes } from "./manifest";
import { modelToolId, sketchToolId, ModelingCommands, type ModelingCommandKey } from "./ids";
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

function toolDefinition(d: ModelingToolDescriptor): ToolDefinition {
  return {
    id: toolIdOf(d),
    title: d.label,
    icon: d.icon,
    group: d.group,
    priority: d.priority,
    scopes: [scopeToken(d.scope)],
    defaultShortcut: shortcutOf(d),
    shortcutLabel: d.shortcut,
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

/** Registers every modeling contribution into `scope`. Exported for tests. */
export function contributeModeling(scope: ModuleScope): void {
  for (const d of MODELING_TOOL_DESCRIPTORS) scope.registerTool(toolDefinition(d));

  let priority = 100;
  for (const key of Object.keys(COMMAND_ACTIONS) as ModelingCommandKey[]) {
    scope.registerCommand(commandDefinition(key, priority));
    priority += 10;
  }
}

export function registerModelingModule(platform: Platform): void {
  platform.registerModule({
    id: MODELING_MODULE_ID,
    version: "1.0.0",
    activate: (scope) => contributeModeling(scope),
  });
}
