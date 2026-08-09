# ADR-0013 — `@onecad/sdk` is a narrower surface than `@/platform`

Status: accepted (2026-08-09)

## Context

`@/platform` was designed as the internal host API and is good at that job. It
exports `createPlatform`, the registry objects, `PlatformProvider`, `SlotHost`,
`scopeFor`, `disposeOwner`, and `createDocumentStateService(client, owner)` —
which takes a `CadClient`. All of that is right for a built-in module compiled
into the app and versioned with it.

None of it is right for a package downloaded from a release page. Not because
addons are assumed hostile — in v1 they are trusted — but because **whatever an
addon can reach is what we must keep working forever**. Re-exporting the host's
composition root as the extension API would freeze the host's internal shape.

`ModuleScope` is the specific problem: it carries `platform`, i.e. every
registry, every other owner's scope, and `dispose()`.

## Decision

**Two surfaces, one implementation.** `@onecad/sdk` (aliased to `src/sdk/`, not
published — a real package buys nothing until something outside this repo builds
against it) re-exports a curated subset of the platform: branded ids and their
checked constructors, `Disposable`, the contribution contracts, `Slots`,
`SelectionRef`, the document-state *types*, and `ExtensionContext`.

**`ExtensionContext` replaces `ModuleScope` for addons.** Same registration
methods, minus `platform`, minus `createScope`, plus `document.state` already
bound to the addon's namespace so the owner id is not a parameter an addon can
get wrong. The narrowing is **by construction** — a fresh object with the allowed
methods — not by casting a scope to a smaller type, because a cast leaves
`platform` present at runtime one `as` away from anything.

**`createExtensionContext` and `registerExtension` are host-side and absent from
the SDK.** An addon that could build its own context could hand itself the scope
the context exists to hide.

**The SDK's runtime surface is snapshot-tested.** Seven runtime values today
(`Slots`, `addonId`, `contributionId`, `isSlotId`, `isOwnerIdShape`,
`DEFAULT_PRIORITY`, `IdError`); everything else is types. Widening it should be a
visible diff in review, not a side effect of adding an export in the platform.

**`react`, `react-dom` and `three` are host-provided.** A viewport contribution
hands the host `Object3D`s and a panel is mounted into the host's React tree, so
a second copy of either inside an addon bundle is a correctness bug, not a size
problem (ADR-0009). Recorded now, before packaging exists, because the packaging
tooling has to externalize them from day one.

## Consequences

- The reference addon (`src/addons/reference/`) is a FIXTURE, and it is
  deliberately **not registered in `bootstrap.ts`**: shipping fake "Widgets" into
  the product UI to prove an architectural point would be a worse trade than
  proving it in tests that drive the real toolbar, tree, inspector, shortcut lane
  and viewport host. When the addon loader lands it becomes the loader's first
  package, which is where registration belongs.
- Its domain is its own (`com.onecadtest.reference.item`), never a body or a
  sketch. An addon that decorated modeling entities would pass every host check
  by accident.
- `architecture.test.ts` enforces both directions: the addon may not import
  `@/platform`, `@/modules`, `@/features`, `@/stores`, `@/tools`, `@/viewport`,
  `@/ipc`, `@/app`, `@/ui`, `@/icons`, `@/debug`; the SDK may not import
  application implementation. Both rules carry the existing positive-control
  pattern, because an empty result is also what a broken scanner returns.
- If an addon ever *needs* an exception to those rules, the exception is a
  missing SDK export, and adding it is a deliberate contract change.
