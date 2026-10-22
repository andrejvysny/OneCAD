# 0018 — Portable command addons use a restricted WebAssembly component host

- Status: Accepted mechanism for the initial command-addon slice; platform qualification is separate.
- Date: 2026-10-22
- Supersedes: ADR-0017 decision 1's deferral of external addon loading only.

## Context

The native application needs a distributable Rust addon mechanism, beginning with
Bambu Studio export. The selected direction is a Rust SDK and portable sandboxed
extensions, with restricted capabilities by default. Modules, workspaces and
installable addons remain distinct (ADR-0001). Document mutations remain owned by
application services; OCCT stays behind the existing worker protocol.

## Boundary and packaging

`onecad-extension-api` owns toolkit-independent, versioned manifest and command
contracts and the `onecad:extension@0.1.0` WIT world. The Rust SDK compiles guests
for `wasm32-unknown-unknown`; the build tool produces a Component Model binary.
A package is a bounded directory containing `manifest.json`, the component entry,
and optional resources. Packaging builds source addons and includes the generated
packages; it does not rely on tracked machine code or a developer's stale output.

`onecad-extension-host` discovers and installs packages, owns persisted approval
and enabled/failed state, and runs components through Wasmtime. It has no normal
dependency on application services, UI, scene or rendering. Native UI adapts its
validated command plans to the existing bounded document service actor. The app
owns geometry reads, export files and external-application launch services.

Package and contribution IDs are namespaced. Duplicate commands are rejected,
including collisions between nested package namespaces. File traversal uses
bounded directory-relative reads that reject links and special files. The package
fingerprint covers all package files; changing any content invalidates approval.
Installation does not enable code. Permission review binds capabilities and
application identities to the reviewed fingerprint. Uninstall removes the installed
package and contributions, never document-owned data. Bundled packages are seeded
once per profile so refresh does not undo an explicit uninstall.

Local updates validate a same-ID replacement before replacing files, reject
command collisions and fence the installed fingerprint. Replacement revokes
approval, including for identical contents, and requests fresh review. Reported
replacement or persistence failures restore the previous package and approval;
this mechanism does not provide recovery from process death between file renames.

## Execution contract

A fresh component instance receives a command ID and availability context and
returns a typed, immutable effect plan. The entire plan is validated before any
application side effect. Version 1 supports 3MF export and handoff of the previous
export artifact to a declared, approved application. Guests cannot choose arbitrary
filesystem paths, process arguments, or undeclared applications. Executable paths
are user settings; a macOS bundle name is a reviewed discovery hint.

The component has no imports, including WASI. It receives no ambient filesystem,
network, environment, process, clock, document or GPU access. Default guest limits
include fuel, a single 32 MiB memory, stack/table/instance limits and bounded success
and error responses. Untrusted native precompiled artifacts are never deserialized.
These guest limits do not assert a total native process memory bound for the compiler.
Compilation occurs off the UI thread and package/component input sizes are bounded.

A command exports a captured document/runtime/revision/snapshot/worker-epoch/history
identity. The app rejects pending geometry and stale targets, meshes only the chosen
bodies, preserves names and appearance through the existing 3MF writer, and retains
a uniquely named export on launch failure. The handoff admission fence is checked
again immediately before launch. A successful launcher outcome means launch accepted,
not that Bambu Studio imported the model or that a printer received anything.

The native host serializes approval changes and effect execution across document
tabs. UI replies carry monotonic generations; queued native menu events carry
registration tokens, and queued execution requests capture the package fingerprint.
Neither stale replies nor queued clicks may retarget a newer contribution with
the same command ID. Failed guests lose active contribution availability and expose
a diagnostic. Lack of a required document or selection is ordinary unavailability,
not an extension crash.

## Supported surfaces and future contracts

The initial slice contributes commands to Export, the Extensions menu, the
extension manager and the extension command palette. The Bambu addon uses the same public SDK and capability
contracts as another package; the registry has no Bambu-specific dispatch branch.

This version does not expose arbitrary UI cards, workspace registration, feature
evaluators, interactive tool callbacks, viewport layers or rendering passes. Those
require additional typed contribution and service contracts. The current one-shot
WIT world does not support branching on host responses; interactive protocols need
an explicitly versioned expansion. Native-trusted manifests are recognized so their
trust requirement is explicit, but this host refuses native execution even if trust
is supplied. A future native helper runtime requires its own supervised boundary.

Existing document module preservation and opaque-operation handling continue to
apply. This command implementation neither changes the document schema nor makes
unknown parametric operations executable. Future feature evaluators must integrate
with authoritative regeneration and topology identity rather than bypassing them.

Expand the SDK through independently versioned contracts, with these ownership
boundaries:

| Extension point | Contract and host responsibility |
|---|---|
| Buttons, cards and panels | Named UI slots, declarative controls and command bindings; the host owns layout, accessibility, themes and input focus. Stable slots should cover menus, toolbars, inspectors, context menus and home pages without exposing widget internals. |
| Workspaces | Named compositions of tools, panels and shortcuts; workspace switching must preserve document state and resolve conflicting contributions explicitly. |
| Parametric modeling | Versioned feature schemas, deterministic evaluation inputs and topology references; application services own transactions, undo, regeneration and cancellation. Missing evaluators preserve opaque feature data and report unsupported regeneration. |
| Macros | Sequences of authorized application commands with explicit transaction boundaries, progress and cancellation; no direct document mutation or worker connection. |
| Viewport overlays and styles | Bounded immutable draw descriptions and named style overrides; the renderer owns GPU resources, picking IDs, depth policy, budgets and cleanup. Arbitrary shader execution requires a separate decision. |
| Importers and exporters | Host-granted input/output handles, format declarations and streaming limits; application services validate geometry and own document admission. |
| Document metadata | Namespaced, versioned payloads with migration and missing-addon preservation; neither installation nor uninstall silently rewrites documents. |
| Events and background tasks | Read-only snapshots, bounded subscriptions, cancellation and lifecycle cleanup; events enqueue commands rather than reentering document mutation. |

Each addition needs capability review, resource limits, lifecycle tests and a
compatibility policy before becoming public SDK surface. Version negotiation must
reject unsupported required contracts while allowing optional contributions to be
omitted. Package signatures and a future catalog establish provenance; they do not
replace capability approval. Updating a package must invalidate its existing
content-bound approval, and disabling it must cancel subscriptions and retire its
contributions without destroying document-owned data.

## Verification

The sandbox registry suite covers package admission, approvals, denied imports and
capabilities, content changes, duplicate registrations, traps and resource limits.
The cross-layer Bambu test runs the compiled component, exports real worker geometry
for document and selected-body scopes, and hands the resulting file to a controlled
launcher. Application and UI tests cover stale targets, pending geometry, retained
files, literal argument handling, approval state and reply/menu ordering. Native
macOS presentation and actual Bambu Studio import remain separate manual checks.

See the [Rust SDK](../../crates/onecad-extension-sdk/README.md) and
[Bambu addon](../../addons/bambu-studio/README.md) for build and use instructions.
The [native extension UI contract](../design/native-extension-ui.md) maps the
reference designs to supported surfaces and records the visual verification lane.
