/*
 * What the ⌘K palette offers, projected off the registries.
 *
 * There is no palette REGISTRY and deliberately so: a command, a tool and a
 * workspace already have identity, availability and metadata (ADR-0003), and a
 * fourth registration for "…and it is also in the palette" would be a second
 * place to forget. Anything registered is findable; nothing has to opt in.
 *
 * A DISABLED item still appears, dimmed, carrying the reason its owner gave.
 * That is the whole reason `CommandAvailability` has a `reason` field: a
 * command that silently vanishes when it cannot run teaches the user that the
 * app is inconsistent, while "Fillet — requires one or more edges" teaches
 * them how to use it.
 */
import type {
  CommandContext,
  CommandDefinition,
  Platform,
  Shortcut,
  ToolContext,
  ToolDefinition,
  WorkspaceDefinition,
} from "@/platform";

export type PaletteKind = "command" | "tool" | "workspace";

export interface PaletteItem {
  readonly id: string;
  readonly kind: PaletteKind;
  readonly title: string;
  /** Where it came from — "Workspace", "Add-on · Robot Tools", "View". */
  readonly source: string;
  /** Written form of the chord, e.g. "⇧F". Empty when it has none. */
  readonly shortcut: string;
  readonly enabled: boolean;
  /** Why it is disabled, when its owner said. */
  readonly reason?: string;
  readonly keywords: readonly string[];
  run(): void;
}

/** Owner id → the label shown under an item. */
export function sourceLabel(owner: string, group: string | undefined): string {
  const builtIn = owner.startsWith("onecad.");
  const domain = owner.split(".").slice(1).join(" ") || owner;
  const base = builtIn ? capitalize(domain) : `Add-on · ${owner}`;
  return group ? `${base} · ${group}` : base;
}

function capitalize(s: string): string {
  return s.length === 0 ? s : s[0].toUpperCase() + s.slice(1);
}

/**
 * The written form of a chord, when the definition did not supply one.
 * Mac glyphs, because that is what every other surface in the app shows.
 */
export function formatShortcut(shortcut: Shortcut | undefined): string {
  if (!shortcut) return "";
  const parts: string[] = [];
  if (shortcut.ctrl) parts.push("⌃");
  if (shortcut.alt) parts.push("⌥");
  if (shortcut.shift) parts.push("⇧");
  if (shortcut.meta) parts.push("⌘");
  parts.push(shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key);
  return parts.join("");
}

export interface PaletteDeps {
  readonly platform: Platform;
  readonly commandContext: CommandContext;
  readonly toolContext: ToolContext;
  /** Called with the workspace the user picked. */
  activateWorkspace(workspace: WorkspaceDefinition): void;
}

export function buildPaletteItems(deps: PaletteDeps): PaletteItem[] {
  const { platform, commandContext, toolContext } = deps;
  const items: PaletteItem[] = [];

  for (const reg of platform.commands.registrations()) {
    const def: CommandDefinition = reg.entry;
    const verdict = def.canExecute?.(commandContext) ?? { enabled: true };
    items.push({
      id: def.id,
      kind: "command",
      title: def.title,
      source: sourceLabel(reg.owner, def.group),
      shortcut: formatShortcut(def.defaultShortcut),
      enabled: verdict.enabled,
      reason: verdict.reason,
      keywords: [def.description ?? "", ...(def.keywords ?? [])].filter(Boolean),
      run: () => void def.execute(commandContext),
    });
  }

  for (const reg of platform.tools.registrations()) {
    const def: ToolDefinition = reg.entry;
    const verdict = def.canActivate?.(toolContext) ?? { enabled: true };
    items.push({
      id: def.id,
      kind: "tool",
      title: def.title,
      source: sourceLabel(reg.owner, def.group),
      shortcut: def.shortcutLabel ?? formatShortcut(def.defaultShortcut),
      enabled: verdict.enabled,
      reason: verdict.reason,
      keywords: [],
      run: () => void platform.toolHost.activate(def.id, toolContext),
    });
  }

  for (const reg of platform.workspaces.registrations()) {
    const def = reg.entry;
    items.push({
      id: def.id,
      kind: "workspace",
      title: `Switch to ${def.title}`,
      source: "Workspace",
      shortcut: "",
      enabled: true,
      keywords: [def.title],
      run: () => deps.activateWorkspace(def),
    });
  }

  return items;
}

/**
 * Substring match over title, source and keywords.
 *
 * Deliberately not fuzzy. A CAD command list is small and its names are
 * domain terms, so fuzzy matching mostly buys surprising hits ("fillet"
 * matching "Fixed support") in exchange for a ranking function nobody can
 * predict. Enabled items sort first so a usable answer is never buried under
 * greyed-out ones.
 */
export function filterPaletteItems(
  items: readonly PaletteItem[],
  query: string,
): readonly PaletteItem[] {
  const needle = query.trim().toLowerCase();
  const matched =
    needle.length === 0
      ? [...items]
      : items.filter((i) =>
          [i.title, i.source, ...i.keywords].some((t) => t.toLowerCase().includes(needle)),
        );
  return matched.sort((a, b) => Number(b.enabled) - Number(a.enabled));
}
