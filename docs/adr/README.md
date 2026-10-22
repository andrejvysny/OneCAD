# Architecture Decision Records

These records retain code-coupled architectural rationale and historical
provenance. [Current architecture](../ARCHITECTURE.md) describes the native Rust
implementation. Project decisions, status and research are maintained in the
[canonical Plane documentation](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/0bfe867d-215a-4cfb-a501-9f628976bc9d).

ADRs 0001–0014 describe the former TypeScript implementation. Preserve applicable
data and ownership invariants; references to React, DOM, Three.js or a public
TypeScript SDK do not describe an active native runtime. ADRs 0015–0017 reconcile
the native direction. Accepted direction does not imply qualification or delivery.

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
| [0009](0009-viewport-context-is-threejs-shaped.md) | Historical Three.js viewport contract; superseded by 0015 |
| [0010](0010-tool-activation-is-host-owned.md) | Tool activation and availability are host-owned, module-reported |
| [0011](0011-shortcuts-module-ruleset-then-registry.md) | Shortcuts: the module ruleset resolves first, the registry second |
| [0012](0012-tree-rows-are-provider-scoped.md) | Tree rows are provider-scoped, and row actions are commands |
| [0013](0013-sdk-is-a-narrower-surface-than-the-platform.md) | Historical TypeScript SDK; narrowing principle retained by 0015/0017 |
| [0014](0014-render-module-openpbr.md) | Render is its own module, targets the Visualization placeholder, adopts OpenPBR |
| [0015](0015-native-rust-ui.md) | Accepted native Rust/egui/wgpu direction; qualification remains separate |
| [0016](0016-kernel-hosting-macos.md) | Accepted production sidecar direction; optional in-process experiment retained |
| [0017](0017-external-addons-deferred.md) | Deferral superseded by 0018; native boundaries remain toolkit-independent |
| [0018](0018-portable-command-addons.md) | Restricted portable command addons, Rust SDK and host-owned effects |

Status values: `Proposed`, `Accepted`, `Superseded by NNNN`.
