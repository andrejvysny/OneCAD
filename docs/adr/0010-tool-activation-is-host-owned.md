# ADR-0010 — Tool activation and availability are host-owned, module-reported

Status: accepted (2026-08-09)

## Context

P2 W8 made the toolbar read the tool registry live, but it kept projecting each
`ToolDefinition` back into modeling's `Tool` union to render and activate it
(`registryToolbar.ts`, `toolFromId`). Three consequences followed, and all three
would have been frozen into `@onecad/sdk`:

- A registration whose id was not in modeling's reverse map was **skipped
  silently** — valid in the registry, absent from the screen.
- The toolbar called `activateTool(literal)`, never `ToolDefinition.activate`, so
  the definition's own activation was unreachable from the only UI that shows it.
- Availability came from `getToolApplicability`, keyed on that same closed union,
  while `ToolDefinition.canActivate` had zero producers and zero consumers.

"Which tool is active" is domain-neutral. "What a tool does when it activates" is
not. The two had been collapsed into one modeling-shaped answer.

## Decision

**A `ToolHost` on the Platform owns the active id and the activate/deactivate
handshake** (`src/platform/toolHost.ts`). `FloatingToolbar` renders
`ToolDefinition`s directly — `title`, `icon`, `shortcutLabel`, `group` boundaries
as separators, `priority` as order — and activates through the host.

**Modules stay authoritative for their own tools and REPORT.** Modeling's
`toolStore` is still where AUTO-MODE, the Esc ladder and every self-arming
controller write; `register.ts` subscribes to it and calls `toolHost.report(id)`.
The host mirrors one truth rather than holding a second one, so no entry point
can leave the highlight disagreeing with the store.

**`deactivate()` runs only on a CROSS-owner swap.** Modules are already exclusive
internally, so calling it on every same-owner swap would insert a teardown into a
flow that never had one. Crossing an owner boundary is the case nobody handles
today and the one where two modules would otherwise both believe they hold the
pointer.

**`canActivate` returns `ToolAvailability`, not `boolean`**, and gained an
optional `subscribe(onChange)`. The reason matters: the toolbar has always
explained *why* a tool is grayed out, and a bare boolean would have thrown that
away the moment the host stopped reading modeling's matrix. `subscribe` exists
for the same reason `TreeProvider.subscribe` does (W14) — a predicate read during
render goes stale unless its owner says when the answer moved. Modeling backs all
of its tools with one shared emitter over two stores rather than one subscription
per tool.

**Scope membership: declared token, or none at all.** The toolbar shows tools
declaring the active scope's token, and tools declaring **no** scopes appear
everywhere — the same "empty ⇒ always" rule `CommandDefinition.scopes` already
states. The old projection dropped the unscoped case, which made a
zero-knowledge contribution impossible.

**`ToolDefinition.icon` is an open string and is resolved defensively.** The
platform cannot know a third party's glyph, and `Icon` indexes a closed registry;
`toolbarIcon()` falls back to a placeholder instead of throwing inside render.

## Consequences

- `toolbarFromRegistry` is now **probe-facing only**. `toolbarContract.ts` is
  written in `ToolEntry` and a frozen contract may not be edited to follow a
  refactor, so the probe keeps rebuilding that shape from the registry. A new
  case pins that a foreign registration leaves modeling's arrangement identical.
- Modeling keeps its `Tool` union, its dispatcher and its applicability matrix
  unchanged. The adapter moved, not the behavior — every existing toolbar test
  passes untouched.
- The host's `activate()` sets the id *before* awaiting the definition, so a slow
  `activate` cannot clobber a newer `report()` from the owning module.
- A disposed tool cannot stay active: the registry subscription clears it.
- Still open: nothing gives an addon a way to participate in modeling's AUTO-MODE
  (a sketch tool activating from model mode). An addon tool activates as itself,
  in whatever mode is current. That is a modeling-internal behavior and is not
  part of the tool contract.
