/*
 * Registry → toolbar projection, and the scope token the toolbar filters on.
 *
 * `toolbarFromRegistry` is now a PROBE-FACING projection: `FloatingToolbar`
 * renders `ToolDefinition`s directly (a projection into modeling's `Tool` union
 * is exactly what made a third-party tool disappear from the toolbar), while the
 * frozen contract in `src/test/contracts/toolbarContract.ts` is written in
 * `ToolEntry`. A contract may not be edited to follow a refactor, so the probe
 * keeps rebuilding that shape FROM the registry — never from the descriptor table
 * the registry was populated from, or it would only prove the table equals itself.
 *
 * It lives here (not in `features/toolbar`) because the mapping from a tool id
 * back to a store-level `Tool` literal is modeling's business.
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

/**
 * The scope token an editor mode shows tools for.
 *
 * The toolbar filters registrations on this, so a contribution that wants to
 * appear alongside modeling's tools declares it — and one that declares no scope
 * at all appears in every mode (`CommandDefinition.scopes`'s "empty ⇒ always").
 */
export const toolbarScopeToken = (scope: ToolScope): string =>
  scope === "sketch" ? ModelingScopes.Sketch : ModelingScopes.Model;

const scopeToken = toolbarScopeToken;

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
    if (def.toolbarHidden) continue;
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
