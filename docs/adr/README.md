# Architecture Decision Records

One file per decision that is expensive to reverse. Each records the context, the
decision, and what it costs — not a tutorial.

These exist because most of this codebase is written with AI assistance, and a
coding agent infers architecture from folder names unless the law is written
down. `docs/ARCHITECTURE.md` states the laws; these records say why.

| ADR | Decision |
|---|---|
| [0001](0001-module-workspace-addon.md) | Module, Workspace and Addon are three different things |
| [0002](0002-modeling-kernel-closed-in-v1.md) | The modeling kernel is closed to addons in v1 |
| [0003](0003-controlled-ui-slots.md) | UI contributions go into controlled slots, never raw DOM |
| [0004](0004-module-state-in-document-json.md) | Module state lives in `document.json`, not new container paths |
| [0005](0005-opaque-module-preservation.md) | Unknown module state is preserved verbatim |
| [0006](0006-registration-ownership.md) | Every registration has an owner and an owner-scoped teardown |
| [0007](0007-deterministic-contribution-order.md) | Contribution order is explicit, never load-order |
| [0008](0008-branded-ids-unions-retained.md) | Namespaced branded IDs, with the internal unions retained |
| [0009](0009-viewport-context-is-threejs-shaped.md) | The viewport contract is Three.js-shaped in v1 |
| [0010](0010-tool-activation-is-host-owned.md) | Tool activation and availability are host-owned, module-reported |
| [0011](0011-shortcuts-module-ruleset-then-registry.md) | Shortcuts: the module ruleset resolves first, the registry second |
| [0012](0012-tree-rows-are-provider-scoped.md) | Tree rows are provider-scoped, and row actions are commands |
| [0013](0013-sdk-is-a-narrower-surface-than-the-platform.md) | `@onecad/sdk` is a narrower surface than `@/platform` |
| [0014](0014-render-module-openpbr.md) | Render is its own module, targets the Visualization placeholder, adopts OpenPBR |

Status values: `Proposed`, `Accepted`, `Superseded by NNNN`.
