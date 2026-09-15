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

/**
 * Registered `group` id → the human category shown under an item
 * (docs/qa/UX_REVIEW_2026-09-14.md N6: a raw contribution id like
 * "modeling.action" reads as an internal detail, not a category).
 */
const GROUP_LABELS: Record<string, string> = {
  "shell.view": "View",
  "shell.app": "App",
  "modeling.action": "Modeling",
  "modeling.tree": "Model tree",
};

/** The human category for a `group` id, or `undefined` for an unrecognised one. */
function groupLabel(group: string): string | undefined {
  if (group in GROUP_LABELS) return GROUP_LABELS[group];
  if (group.startsWith("library.")) return "Library";
  // No dot: already a plain word (e.g. an add-on's own group) — safe to show
  // verbatim. A dotted, unmapped id is an internal namespace, not a category,
  // so it is dropped rather than leaked into the UI.
  return group.includes(".") ? undefined : group;
}

/** Owner id → the label shown under an item. */
export function sourceLabel(owner: string, group: string | undefined): string {
  const builtIn = owner.startsWith("onecad.");
  const domain = owner.split(".").slice(1).join(" ") || owner;
  const base = builtIn ? capitalize(domain) : `Add-on · ${owner}`;
  const label = group ? groupLabel(group) : undefined;
  return label ? `${base} · ${label}` : base;
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
 * predict. Exact title/verb intent ranks ahead of a keyword hit: querying
 * "redo" must surface a disabled `Redo …` before an enabled `Undo …` that only
 * happens to advertise "redo" as a history keyword. Availability breaks ties,
 * so a usable equal-intent answer is never buried under a greyed-out one.
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
  return matched.sort((a, b) => {
    const rank = (item: PaletteItem): number => {
      if (needle.length === 0) return 0;
      const title = item.title.toLowerCase();
      if (title === needle) return 0;
      // Commands are verb-first (Undo Extrude, Redo Fillet). A whole leading
      // verb is intent, unlike an incidental substring in a keyword.
      if (title.startsWith(`${needle} `)) return 1;
      if (title.includes(needle)) return 2;
      if (item.source.toLowerCase().includes(needle)) return 3;
      return 4; // matched by a keyword
    };
    const intent = rank(a) - rank(b);
    if (intent !== 0) return intent;
    return Number(b.enabled) - Number(a.enabled);
  });
}
