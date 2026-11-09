# 0020 — Assemblies are component definitions and occurrences in one document

- Status: Bounded implementation decision (2026-11-09). Delivery and qualification remain in Plane (ONECAD-340); this is not a gate-completion claim.
- Date: 2026-11-04
- Related: ADR-0021 (scene geometry), ADR-0022 (joint solver hosting).

## Context and decision

A document contains a root definition and embedded component definitions. Each
`ComponentDef` owns its local timeline, sketches, datum planes, bodies, element
index, repair state and named selections. Root fields remain directly available
on `Document` for existing part operations; root-owned assembly state contains
occurrences, visibility, joints and joint bindings. Definitions are embedded;
live external links are not implemented. See
`crates/onecad-core/src/document/{mod.rs,component.rs}` and
`crates/onecad-core/src/edit/assembly.rs`.

An occurrence has its own ID, definition, rigid local pose, grounded flag and
name. Repeated occurrences share a definition and its local topology identities;
a pick or binding additionally carries an `OccurrencePath`, body and `ElementRef`
so a sibling occurrence is not silently substituted. Moving or hiding one
occurrence changes root assembly state. Editing a shared definition changes that
definition for its occurrences; making an independent copy separates subsequent
edits. These edits and Undo/Redo remain under `DocumentSession` and the
application's per-document `DocumentRuntime` single writer.

`PlaceComponent` remains a part-timeline library placement that bakes a body; it
is not converted into an assembly occurrence. Rigid transforms reject reflection.
The general occurrence tree rejects cycles and bounds paths to depth 8 and count
to 2000, but the current document assembly validator and authoring slice accept
**flat embedded occurrences only** and refuse nested assembly state or parented
occurrences. These storage bounds are not capacity or performance promises.
Cross-component sketch references and live external links remain outside this
slice. A definition's feature dependencies remain local to its own history;
the supported flat model does not admit recursive definition dependencies. See
`crates/onecad-assembly/src/occurrence.rs` and
`crates/onecad-core/src/document/component.rs`.

## Persistence and execution

The current document schema is **5**. `crates/onecad-core/src/io/migrate.rs`
registers 1→2 root-definition, 2→3 joint-binding, 3→4 import-source identity,
and 4→5 topology-binding-state migrations. The migration works on decoded
values; an applied migration opens without adopting the original path as its
save target, preserving the source until an explicit new save. Container opens
refuse newer schema versions; incomplete or low-confidence migration policy can
open read-only. Unknown extension data is preserved where supported, while
collisions with reserved migration fields and invalid critical identity or
binding states are refused, not guessed. See
`crates/onecad-core/src/io/{migrate.rs,container.rs}` and
`crates/onecad-app/src/document_runtime.rs`.

A definition edit marks its root joint bindings as needing repair. Persisted
identity evidence and occurrence ownership remain explicit. The bounded
quarantine path handles only the consumer set and worker `NeedsRepair` outcomes
it can prove safe; unknown or competing consumers refuse that path. Re-picking
requires the intended occurrence and fresh scene. It does not make arbitrary
lost or ambiguous references safe. See
`crates/onecad-app/src/document_runtime/component_execution/quarantine.rs`
and `crates/onecad-core/src/document/element_index.rs`.

Component replay snapshots inputs under the runtime lock, performs worker work
without that lock, then checks session, revision, worker epoch, definition
fingerprint and resource admission before committing. Component worker sessions
use a deterministic document/definition namespace on a checked **transient**
worker, with one admitted component execution at a time; they are not a
permanent worker session for every definition. Checked definition meshes are
cached and invalidated under the owning runtime. Source and destination paths
and document IDs have process-local claims; duplicate hosts are refused or
focused, and a failed/cancelled operation releases its staged claims. See
`crates/onecad-app/src/{api/component_execution.rs,document_runtime/component_execution.rs,reservations.rs}`.

## Alternatives and consequences

Multi-file live links, placement as feature records, baking each occurrence into
the root, and reflected instance transforms were not selected for this bounded
slice. Shared definitions avoid repeating authoring history, but publication,
identity repair, memory and draw work still need explicit admission and fences.

The repository contains migration, component, mixed-assembly and repair tests in
`crates/onecad-app/tests/{assembly_schema_migration.rs,component_instances.rs,prusa_mixed_assembly.rs}`
and component/occurrence tests in the core crates. Their existence here is not a
claim that every native workflow or release gate has passed.

## Open questions

- What nested assembly and external-link behavior should be admitted later?
- What evidence is required for imported and ambiguous reference repair beyond the bounded path?
- Which performance and physical-input qualifications remain for target devices?
