# ADR-0012 — Tree rows are provider-scoped, and row actions are commands

Status: accepted (2026-08-09)

## Context

W10 turned the model tree into a `TreeProvider` registry, and W14 gave the
contract an optional `subscribe`. Three things were still true of the runtime,
and all three would have been frozen into the SDK:

- **`TreeNode.id` was resolved globally.** The panel did
  `sections.flatMap(s => s.nodes).find(n => n.id === id)` across every provider.
  Ids are provider-local by contract, so two providers may legitimately mint the
  same string — and the context menu would then open against whichever provider
  sorted first, offering one module's Rename/Hide over another module's row.
- **The host watched modeling's stores on modeling's behalf.** Five
  subscriptions in `ModelTreePanel` existed "so that the provider's reads are
  fresh". No second provider could benefit from them, and the modeling provider
  deliberately omitted `subscribe` because of them.
- **The menu branched on `node.kind`.** `kind === "datum"` and
  `kind === "sketch"` gated delete and reattach, so a foreign provider's rows
  could have rows but never row actions.

## Decision

**Rows are addressed by `(providerId, nodeId)`.** `TreeNode.id` stays
provider-local — the alternative, demanding globally unique ids, pushes a naming
burden onto every contributor to fix a host bug. Panel state (menu target,
inline rename, confirm step) is keyed by the composite.

**Every provider announces its own changes.** Modeling implements `subscribe`
over its stores; the host subscribes to providers and to nothing else.

**Row actions are `TreeNodeAction`, backed by a `CommandId`** — not a callback.
The same action is then reachable from a palette, a keystroke and automation
instead of existing only inside one popover (ARCHITECTURE §6). Opening a row's
menu selects that row first, so the command reads its target through the
ordinary selection path rather than through a private argument only this menu
could supply. `confirm` makes the two-click destructive idiom declarative, so
"delete" items cannot each drift into their own confirmation.

**Reattach stays a modeling-specific branch, and is flagged as one.** It needs a
fact the node does not carry (is this sketch face-hosted?) *and* a second
anchored popover on the same row. Inventing a generic popover protocol for one
call site would be the wrong trade; it is the single remaining `kind` check and
the single remaining store read in the panel.

**`TreeNode.icon` is resolved defensively**, like `ToolDefinition.icon`
(ADR-0010): an unknown glyph renders a placeholder instead of throwing inside a
row.

**Settings left the tree.** `SettingsModal` was mounted from `ModelTreePanel`,
which made "which application-wide preferences exist" a model-tree
responsibility. It is application chrome and now lives in the shell's
`StatusBar`.

## Consequences

- `settingsStore` is untouched: a global zustand store with ~20 non-test
  importers. Moving the MOUNT is cheap; moving OWNERSHIP is its own package, and
  conflating the two would have made this wave a settings refactor.
- The delete probes moved from `tree-menu-*` test ids to
  `tree-action-<commandId>.action`, in vitest and in two e2e specs. The behavior
  they assert — two-click confirm, backend rejection surfaced — is unchanged.
- Modeling's two delete commands register with the EDITOR-mount scope, next to
  the tree provider that offers them, not at bootstrap: the tree is editor
  chrome, and advertising commands whose UI is not loaded would be a lie.
- A tree host that reads no store is now movable to the shell. Nothing depends
  on that yet, so nothing moved.
