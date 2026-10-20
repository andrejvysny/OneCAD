# 0017 — External addons are deferred; modules, slots and ordering continue as Rust traits

- Status: Accepted direction. Current delivery and qualification status belongs in Plane; this record does not establish gate completion.
- Date: 2026-10-17
- Plane: ONECAD-261 (P0.6), program ONECAD-255
- Scope: the macOS desktop app. The iPad/iOS target was dropped from scope by the user on 2026-10-17.

## Context

ADR-0001 to ADR-0013 built a TypeScript extension platform in `src/platform/`
and `src/sdk/`:

- module, workspace and addon kept as separate concepts (0001);
- a kernel closed to addons (0002);
- closed UI slots (0003);
- module state in `document.json`, preserved verbatim when unknown (0004, 0005);
- owner-scoped registration, with duplicates as hard errors (0006);
- explicit `group`/`priority` ordering (0007);
- branded namespaced ids (0008);
- a host-owned tool host, shortcut lanes and provider-scoped tree rows
  (0010–0012);
- a narrower `@onecad/sdk` (0013).

At the time of this decision, no external addon had been loaded. The former
reference addon was a test fixture, excluded from the web composition root.
The native host does not expose that retired TypeScript SDK.

ADR-0015 records the accepted native Rust/egui direction, with gates open. Every part of that platform written
in TypeScript (React components in slots, Three.js objects in
`viewport.layer`, the `ExtensionContext` object) has no runtime in the native
app. The migration plan lists "external addons, a wasm design later" as out of
scope, and has Phase 4 rebuild "a Rust contribution registry with modules,
slots, explicit `group`/`priority`, owner-prefixed ids, and duplicates as hard
errors".

No Phase 0 spike tested an addon mechanism. Two Phase 0 findings still bear on
it:

- **Toolkit independence.** egui is selected subject to native-input gates.
  Slint's historical license comparison remains evidence only; native extension
  contracts must not expose toolkit types regardless of license.
- **Kernel isolation.** ADR-0016 keeps the OCCT worker in its own process. That
  is the same boundary ADR-0002 relies on: no module other than modeling speaks
  OCW1.

## Decision (accepted direction; implementation open)

1. **No external addon loading in the native app until a separate ADR decides
   the mechanism.** That ADR covers packaging, the ABI or wasm boundary,
   sandboxing and versioning. Until then, every module is built in and
   compiled into the app.
2. **The platform laws stay, as Rust traits and types inside the app.** They
   are not a public SDK:
   - **Modules and workspaces** (0001): a `Module` trait with an id, a lifecycle
     and a registration hook. Workspaces stay presentation configuration only.
   - **Slots** (0003): a closed Rust enum of host-owned slots. A contribution
     supplies toolkit-neutral content data; `onecad-ui` adapts it to widgets.
     App contracts do not depend on egui or concrete interaction types. A viewport contribution hands the host scene data and
     overlay items, through ADR-0015's `SceneQuery` / `RenderSink` / `Anchor`
     seam.
   - **Ownership** (0006): every registration carries an owner id and returns a
     disposer, within an owner scope. Duplicate ids are a hard error naming the
     holder. A contribution id must start with its owner's id.
   - **Order** (0007): iteration is `(priority, insertion index)`. `group` is
     consumer metadata, not a sort key.
   - **Ids** (0008): newtypes such as `ToolId(&'static str)` at the registry
     boundary, with modeling's internal enums kept for exhaustive matches.
   - **Tool host, shortcuts and tree** (0010–0012): the host owns activation and
     availability, built-in rulesets resolve before the registry, and tree rows
     are addressed by `(provider, node)`. Row actions are command ids.
3. **The file-format laws are untouched.** Module state lives in
   `document.json`, unknown module state is preserved verbatim (0004, 0005), and
   the kernel stays closed (0002). They bind the Rust document code that
   survives the rewrite, whatever happens to addons.
4. **The narrowing principle of ADR-0013 is carried forward, not its
   artefacts.** When addons return, their surface is a curated subset that is
   narrow by construction, never the host's internal registry. It exposes no UI-toolkit types, keeping toolkit
   dependencies inside `onecad-ui`. Interaction consumes app public contracts;
   app does not import the interaction implementation.

## Consequences

- The TypeScript SDK, reference addon and associated architecture tests are
  retired. The Rust layering
  check (`cargo xtask check-rust-layers`) takes over the "may not import"
  role.
- Without an external consumer, the Rust registry can change shape freely
  between phases. Behavioral parity includes keymap, modelingInteraction, toolbar, shell,
  inspector and tree/menu contracts (ADR-0015). DOM-specific probes may change;
  user-visible semantics remain required.
- Documents written by a future addon-enabled build still open, because 0005's
  opaque preservation stays in force.

## Cost

Third parties cannot extend OneCAD until a later ADR. Every capability is a
built-in module, reviewed and shipped with the app. The TypeScript SDK work
(0013) is not reused as code, only as design. The alternative, designing an
addon ABI during the native migration would freeze boundaries that Phase 0
did not test. egui selection does not establish an external addon ABI.
