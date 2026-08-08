/*
 * Registry → toolbar projection.
 *
 * The toolbar's arrangement can be rebuilt from the platform tool registry alone:
 * scope selects the set, `priority` orders it, and a group change is a separator.
 * That is what makes "the toolbar is a set of contributions" a fact rather than a
 * claim — the golden contract is asserted against THIS, not against the
 * descriptor table it was built from.
 *
 * The toolbar component switches to this when the shell becomes slot-hosted; it
 * lives here (not in `features/toolbar`) because the mapping from a tool id back
 * to a store-level `Tool` literal is modeling's business.
 */
import type { Platform, ToolDefinition } from "@/platform";
import type { IconName } from "@/icons/paths";
import type { ToolEntry } from "@/features/toolbar/toolbarConfig";
import type { ModelTool, SketchTool, Tool } from "@/stores/toolStore";
import { ModelingModelTools, ModelingSketchTools } from "./ids";
import type { ToolScope } from "./tools";
import { ModelingScopes } from "./manifest";

/** Reverse maps, built once from the forward const maps so they cannot drift. */
const MODEL_BY_ID = new Map<string, ModelTool>(
  Object.entries(ModelingModelTools).map(([tool, id]) => [id, tool as ModelTool]),
);
const SKETCH_BY_ID = new Map<string, SketchTool>(
  Object.entries(ModelingSketchTools).map(([tool, id]) => [id, tool as SketchTool]),
);

export function toolFromId(id: string): { scope: ToolScope; tool: Tool } | undefined {
  const model = MODEL_BY_ID.get(id);
  if (model) return { scope: "model", tool: model };
  const sketch = SKETCH_BY_ID.get(id);
  if (sketch) return { scope: "sketch", tool: sketch };
  return undefined;
}

const scopeToken = (scope: ToolScope): string =>
  scope === "sketch" ? ModelingScopes.Sketch : ModelingScopes.Model;

/** Registered modeling tools for one scope, in registry order. */
export function registeredTools(platform: Platform, scope: ToolScope): readonly ToolDefinition[] {
  const token = scopeToken(scope);
  return platform.tools.entries().filter((t) => t.scopes?.includes(token));
}

/** The toolbar arrangement, rebuilt from the registry. */
export function toolbarFromRegistry(platform: Platform, scope: ToolScope): ToolEntry[] {
  const entries: ToolEntry[] = [];
  let group: string | undefined;
  for (const def of registeredTools(platform, scope)) {
    const resolved = toolFromId(def.id);
    if (!resolved) continue;
    if (group !== undefined && def.group !== group) entries.push({ sep: true });
    group = def.group;
    entries.push({
      id: resolved.tool,
      icon: def.icon as IconName,
      label: def.title,
      shortcut: def.shortcutLabel ?? "",
    });
  }
  return entries;
}
