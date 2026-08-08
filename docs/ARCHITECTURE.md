# OneCAD architecture

This document states the architectural laws of the OneCAD codebase. Folder names
change; these rules do not. Where this document and a folder layout disagree,
this document wins.

The migration this describes is in progress. `TODO.md` records which waves have
landed; a rule stated here is binding for new code even where old code has not
yet been moved behind it.

## 1. The one-sentence version

> The OneCAD Platform owns composition and application infrastructure; modules own
> domain behavior and data; workspaces decide how capabilities are presented;
> addons package extensions; and all interaction across those boundaries happens
> through explicit, namespaced, versionable contracts.

Modeling is not a synonym for OneCAD. Modeling is the first privileged built-in
module running on the Platform.

## 2. Terminology

These four words are not interchangeable.

| Term | Owns | Example |
|---|---|---|
| **Platform** | Domain-neutral infrastructure: documents, entities, transactions, commands, selection, workspaces, modules, events, resources, settings, persistence | `onecad.platform` |
| **Module** | One domain capability, its state, and its contributions | `onecad.modeling` |
| **Workspace** | A presentation/composition configuration — which capabilities are prominent for a kind of work | `onecad.workspace.design` |
| **Addon** | An installable, distributable package, which may contribute modules, workspaces, or contributions directly | `com.example.foo` |

A module is not automatically a workspace. A workspace may combine capabilities
from several modules. An addon may extend the default workspace instead of
creating one.

The Platform must not understand Extrude, Fillet, Sketch, BRep, FEM, drawing
views, or CAM operations. That is a feature, not a gap.

## 3. Layering

```
Application Shell
       ↓
OneCAD Platform
       ↓
Stable contracts / SDK
       ↑
Built-in modules
       ↑
External addons
```

Forbidden edges, in both the TypeScript and Rust trees:

- Platform → modeling implementation
- Addon → modeling internals
- Addon → OCCT worker
- Any module → another module's private state
- SDK → application implementation

The C++ worker is the **geometry service implementation of `onecad.modeling`**,
not a general OneCAD backend. A future module needing geometry consumes the
modeling geometry service; it never speaks OCW1 itself.

## 4. Ownership rules

1. The Platform owns generic infrastructure.
2. A module owns its domain state; **only** the owning module mutates it.
3. Foreign modules interact through public services and commands.
4. Addons never reach into private implementation packages.
5. UI composition happens through registries — never through direct imports into
   the shell.
6. Unknown module state is preserved across load/save.
7. Kernel and history-operation extension is **closed** in addon v1.

## 5. Identity

Every registered thing carries a namespaced id and a declared owner:

```
onecad.platform.document.opened
onecad.modeling.tool.extrude
com.example.foo.command.inspect
```

Rules:

- IDs are reverse-domain and stable forever. Display names are never identifiers.
- A third-party contribution id **must** begin with its addon id. An addon cannot
  register `onecad.*`.
- Duplicate registration is a deterministic failure, never last-one-wins.
- Platform `EntityId` and modeling `ElementId` are different concepts with
  different guarantees and are not collapsed into one abstraction.

## 6. Contribution model

Modules contribute; the shell discovers. Contribution kinds: commands, tools,
panels, inspector sections, viewport layers, workspace contributions, services,
settings, document state, resources.

- **Command** = an action with identity, availability and metadata. One command
  feeds toolbar, context menu, palette, shortcut and automation.
- **Tool** = an interaction lifecycle (a mode/state machine). A tool may invoke
  commands. `Command = action, Tool = interaction`.
- **Panel/inspector/viewport contributions** are React components plus metadata,
  placed into a fixed set of host-owned slots. No raw DOM access, no arbitrary
  layout replacement.
- Every registration is owned, and disposing an owner's scope tears down all of
  its registrations and subscriptions.
- Ordering is explicit (`group`, `priority`) and deterministic. It must never
  depend on module load order.

## 7. Viewport

Viewport ownership is centralized. The host owns the renderer, canvas, camera,
navigation, frame lifecycle, selection integration, resource disposal and render
scheduling. Modules contribute layers.

Rendering is **on demand**: contributions call `invalidate()`; they never run
their own `requestAnimationFrame` loop. Idle means zero frames.

The world is Z-up, right-handed. Mesh vertex buffers are uploaded verbatim.

## 8. Documents and persistence

One project is one `.onecad` file. Inside it:

- Platform-owned: project identity, metadata, module registry, resource index.
- Module-owned: modeling state, addon state, each with its own schema version.

Laws:

- **If OneCAD cannot interpret module-owned state, it must preserve it verbatim
  across load and save.** A missing addon is not a reason to destroy its data.
- Foreign fields are never embedded inside another module's private state.
  An addon annotates a body from its own namespace, keyed by entity id.
- A document records the modules/addons it uses, so a missing one can be
  reported rather than silently ignored.
- Application settings and document state are different storage. Neither
  substitutes for the other.
- Cache data and authoritative data stay distinguishable; a module must not be
  able to put essential project state in a cache namespace.
- Archive guards (entry count, uncompressed size, path traversal, decompression
  limits) apply to every section, including module and addon data. Generalizing
  storage must never become a way around them.

Version numbers that are independent and must not be conflated: application
version, Platform API version, addon RPC version, container format version,
per-module state schema version, addon package version.

## 9. Transactions

All document mutation goes through transactions. Programmatic changes made by a
module or addon use the **same** command/transaction layer as user-triggered
changes — there is no separate mutation path — so automation stays undoable,
redoable, observable and serializable.

## 10. Events

Events announce state changes; they are not an alternative API for performing
them. `onecad.modeling.history.changed` is an event. "Please extrude this body"
is not.

## 11. Typing policy

Dynamic serialization belongs at genuine extension and wire boundaries only.

```
inside a module      strongly typed
platform contracts   strongly typed interfaces / DTOs
RPC boundary         serialized DTOs
unknown module state opaque JSON
```

`KnownOperation` stays a strongly typed enum. Modules existing is not a reason to
weaken it into a string-keyed map. Public DTOs are designed from use cases, not
by mirroring internal structs.

## 12. Extension points that are intentionally closed

Open: commands, tools, panels, inspector, viewport layers, workspaces, services,
settings, document state, resources, integrations, libraries.

Closed in v1: kernel algorithms, history operation types, topological naming,
regeneration internals, worker commands, serialization internals.

Deterministic `NeedsRepair` beats a silent wrong bind, everywhere. Nothing in the
extension architecture may weaken that.
