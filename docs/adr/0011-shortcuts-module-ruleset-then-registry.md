# ADR-0011 — Shortcuts: the module ruleset resolves first, the registry second

Status: accepted (2026-08-09)

## Context

`CommandDefinition.defaultShortcut` and `ToolDefinition.defaultShortcut` were
**write-only**. Three producers filled them in (`modules/modeling/register.ts`,
`modules/shell/module.ts`) and nothing ever read them back: `useShortcuts`
installs one window listener that asks `resolveBinding`, which reads two static
tables merged at module load. A contribution could declare a chord and that chord
would never fire — a keyboard no addon can reach.

The obvious fix — move all resolution into a platform `ShortcutService` — is
wrong, for a reason specific to this codebase. Modeling's resolution rules are
not generic: mode precedence, exact-chord matching, and a cross-mode fallback
restricted to `tool` actions with a `NO_CROSS_MODE` opt-out. They know what
"sketch mode" is. They are also pinned chord-for-chord by a frozen contract
(`src/test/contracts/keymapContract.ts`) whose probe compares `resolveBinding`
against an independently written oracle over every (key, shift, mode) triple.
Re-deriving that inside the platform would re-derive the behavior the contract
exists to protect, and would teach the platform a modeling concept.

## Decision

**Two lanes, in a fixed order.** The keydown handler asks:

1. the hardcoded modifier chords (undo/redo/save/open/close) — unchanged;
2. `resolveBinding` — modeling's + shell's static tables, unchanged;
3. `platform.shortcuts` — every registration's `defaultShortcut`.

A contribution therefore **cannot shadow a built-in chord**, and the golden
keymap oracle stays green byte for byte. The modeling ruleset is a *module's*
ruleset that happens to run first, not platform behavior.

**Conflicts resolve deterministically, never by load order:** scope-specific
beats unscoped → lower explicit `priority` → built-in (`onecad.*`) beats addon.
A chord still tied after all three **activates nothing** and is reported through
`conflictFor`/`conflicts` and a `logWarn`. Two addons claiming ⇧F must behave the
same on every boot, and a keystroke silently running the wrong addon's command is
the worst place to allow an arbitrary winner.

**Scope tokens are supplied by the caller.** `useShortcuts` maps the editor mode
to modeling's scope tokens; the platform only intersects opaque strings.

**A keystroke does not bypass `canExecute`.** The palette and the toolbar respect
the gate; so does the keyboard.

## Consequences

- The ⌘-chords stay hardcoded and are **not addon-reachable in v1**. Stated here
  rather than left implicit: an addon declaring `{key: "s", meta: true}` gets
  nothing, because the handler returns before the registry lane.
- Every built-in chord is claimed twice — once in the static table, once as a
  `defaultShortcut` mirror. That duplication predates this ADR (the mirrors were
  already there) and is what makes the tables removable later without a contract
  change: when the frozen keymap is retired, lane 2 goes and lane 3 stays.
- `useShortcuts` now needs a `Platform`. Its probe was rewrapped
  (`renderWithPlatform`); the contract it asserts against did not move.
- Remapping is still not offered. The service resolves *declared* chords only;
  a user keymap is a later, separate store in front of the same resolver.
