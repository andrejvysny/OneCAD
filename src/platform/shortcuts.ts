/*
 * Shortcut resolution over the registries.
 *
 * `CommandDefinition.defaultShortcut` and `ToolDefinition.defaultShortcut` were
 * write-only: three producers filled them in and nothing ever read them back, so
 * a registered command could declare a chord and that chord would never fire. A
 * keyboard that cannot see the registry is a keyboard no addon can reach.
 *
 * WHAT THIS DOES NOT OWN. Modeling's mode precedence, exact-chord matching and
 * the cross-mode `tool`-only fallback stay in `shortcuts/keymap.ts`: they are a
 * MODULE's ruleset (they know what "sketch mode" is), they are pinned chord for
 * chord by a frozen contract, and re-deriving them here would be re-deriving
 * behavior the contract exists to protect. The keyboard asks that ruleset first
 * and this second, so built-ins keep resolving exactly as they always have.
 *
 * CONFLICTS ARE NEVER RESOLVED BY LOAD ORDER. Two addons claiming ⇧F must not
 * depend on which one installed first. The ladder is: scope-specific beats
 * unscoped, then explicit `priority`, then built-in (`onecad.*`) beats addon. A
 * chord still tied after all three activates NOTHING and is reported — a silent
 * arbitrary winner is the failure mode this whole architecture exists to avoid.
 */
import type { CommandContext, CommandDefinition, Shortcut, ToolDefinition } from "./contributions";
import type { CommandId, OwnerId, ToolId } from "./ids";
import type { Registry } from "./registry";
import { DEFAULT_PRIORITY } from "./registry";
import type { ToolHost } from "./toolHost";

/** A keystroke, normalized. `key` is compared case-insensitively. */
export interface Chord {
  readonly key: string;
  readonly shift?: boolean;
  readonly alt?: boolean;
  readonly ctrl?: boolean;
  readonly meta?: boolean;
}

export type ShortcutTargetKind = "command" | "tool";

export interface ShortcutTarget {
  readonly kind: ShortcutTargetKind;
  readonly id: CommandId | ToolId;
  readonly owner: OwnerId;
  readonly title: string;
}

/** Two or more registrations claim one chord and nothing separates them. */
export interface ShortcutConflict {
  readonly chord: Chord;
  readonly candidates: readonly ShortcutTarget[];
}

export interface ShortcutService {
  /**
   * The single registration that owns `chord` in `scopes`, or null when nothing
   * claims it OR the claim is ambiguous (`conflictFor` then explains).
   */
  resolve(chord: Chord, scopes: readonly string[]): ShortcutTarget | null;
  /** The ambiguity that suppressed `chord`, if that is why resolve returned null. */
  conflictFor(chord: Chord, scopes: readonly string[]): ShortcutConflict | null;
  /** Every chord claimed by more than one registration, for diagnostics. */
  conflicts(scopes: readonly string[]): readonly ShortcutConflict[];
  /**
   * Execute a resolved target: run the command, or activate the tool.
   *
   * `ctx` is the SAME context resolution ran against. Dropping it here was a real
   * defect: a scope-gated contributed command resolved correctly and then refused
   * to execute, because `canExecute` was asked about an empty context (Codex
   * review, P2.5).
   */
  run(target: ShortcutTarget, ctx?: CommandContext): Promise<void>;
}

const EMPTY_CONTEXT: CommandContext = { selection: [], scopes: [] };

const norm = (key: string): string => (key.length === 1 ? key.toLowerCase() : key);

/** Exact chord equality — an unmodified key must not answer a modified one. */
function sameChord(a: Chord, b: Chord): boolean {
  return (
    norm(a.key) === norm(b.key) &&
    Boolean(a.shift) === Boolean(b.shift) &&
    Boolean(a.alt) === Boolean(b.alt) &&
    Boolean(a.ctrl) === Boolean(b.ctrl) &&
    Boolean(a.meta) === Boolean(b.meta)
  );
}

/** Scopes absent/empty ⇒ active everywhere (`CommandDefinition.scopes`). */
function inScope(declared: readonly string[] | undefined, active: readonly string[]): boolean {
  return declared === undefined || declared.length === 0 || declared.some((s) => active.includes(s));
}

interface Candidate extends ShortcutTarget {
  readonly scoped: boolean;
  readonly priority: number;
  readonly builtIn: boolean;
}

/** Built-ins own the `onecad.` namespace; everything else is a third party. */
const isBuiltIn = (owner: OwnerId): boolean => owner.startsWith("onecad.");

export function createShortcutService(
  commands: Registry<CommandDefinition>,
  tools: Registry<ToolDefinition>,
  toolHost: ToolHost,
): ShortcutService {
  function candidatesFor(chord: Chord, scopes: readonly string[]): Candidate[] {
    const out: Candidate[] = [];

    const collect = (
      kind: ShortcutTargetKind,
      entry: { id: string; title: string; defaultShortcut?: Shortcut; scopes?: readonly string[]; priority?: number },
      owner: OwnerId | undefined,
    ): void => {
      if (!entry.defaultShortcut || owner === undefined) return;
      if (!sameChord(entry.defaultShortcut, chord)) return;
      if (!inScope(entry.scopes, scopes)) return;
      out.push({
        kind,
        id: entry.id as CommandId | ToolId,
        owner,
        title: entry.title,
        scoped: entry.scopes !== undefined && entry.scopes.length > 0,
        priority: entry.priority ?? DEFAULT_PRIORITY,
        builtIn: isBuiltIn(owner),
      });
    };

    for (const c of commands.entries()) collect("command", c, commands.ownerOf(c.id));
    for (const t of tools.entries()) collect("tool", t, tools.ownerOf(t.id));
    return out;
  }

  /** The survivors after the ladder. One ⇒ a winner; more ⇒ a conflict. */
  function survivors(chord: Chord, scopes: readonly string[]): Candidate[] {
    let pool = candidatesFor(chord, scopes);
    if (pool.length <= 1) return pool;

    const scoped = pool.filter((c) => c.scoped);
    if (scoped.length > 0 && scoped.length < pool.length) pool = scoped;
    if (pool.length === 1) return pool;

    const best = Math.min(...pool.map((c) => c.priority));
    pool = pool.filter((c) => c.priority === best);
    if (pool.length === 1) return pool;

    const builtIns = pool.filter((c) => c.builtIn);
    if (builtIns.length > 0 && builtIns.length < pool.length) pool = builtIns;
    return pool;
  }

  const strip = (c: Candidate): ShortcutTarget => ({
    kind: c.kind,
    id: c.id,
    owner: c.owner,
    title: c.title,
  });

  // Plain functions, not object methods: a caller that destructures the service
  // (`const { resolve } = platform.shortcuts`) must not get a broken `this`.
  const conflictFor = (chord: Chord, scopes: readonly string[]): ShortcutConflict | null => {
    const pool = survivors(chord, scopes);
    return pool.length > 1 ? { chord, candidates: pool.map(strip) } : null;
  };

  return {
    resolve(chord, scopes) {
      const pool = survivors(chord, scopes);
      return pool.length === 1 && pool[0] ? strip(pool[0]) : null;
    },

    conflictFor,

    conflicts(scopes) {
      const seen = new Set<string>();
      const out: ShortcutConflict[] = [];
      const consider = (shortcut: Shortcut | undefined): void => {
        if (!shortcut) return;
        const key = `${norm(shortcut.key)}|${Boolean(shortcut.shift)}|${Boolean(shortcut.alt)}|${Boolean(shortcut.ctrl)}|${Boolean(shortcut.meta)}`;
        if (seen.has(key)) return;
        seen.add(key);
        const conflict = conflictFor(shortcut, scopes);
        if (conflict) out.push(conflict);
      };
      for (const c of commands.entries()) consider(c.defaultShortcut);
      for (const t of tools.entries()) consider(t.defaultShortcut);
      return out;
    },

    async run(target, ctx = EMPTY_CONTEXT) {
      if (target.kind === "tool") {
        await toolHost.activate(target.id as ToolId, ctx);
        return;
      }
      const command = commands.get(target.id);
      // Availability is the command's own answer; a keystroke must not bypass a
      // gate the palette and the toolbar both respect.
      if (!command) return;
      if (command.canExecute && !command.canExecute(ctx).enabled) return;
      await command.execute(ctx);
    },
  };
}
