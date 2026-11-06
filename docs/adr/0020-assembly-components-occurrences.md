# 0020 — Assemblies are component definitions and occurrences in one document

- Status: Proposed (2026-11-04). Delivery and qualification status belongs in
  Plane (ONECAD-340); this record does not establish implementation or gate
  completion.
- Date: 2026-11-04
- Related: ADR-0021 (instanced geometry), ADR-0022 (joint solver hosting).

## Context

`Document` (`crates/onecad-core/src/document/mod.rs`) holds one strict-linear
`Timeline` (`history/timeline.rs`), one `BodyRegistry`, sketches and datum
planes, mutated only through `DocumentSession`. `DOCUMENT_SCHEMA_VERSION` is 1.
The OCW1 worker opens exactly one session per document (`OpenSession`,
protocol/SCHEMA.md: "One session per document (V1)").

`PlaceComponent` (`document/record.rs`, with `ComponentMate`/`MateFrame`) is a
part-level library insert: it bakes a library body into the current timeline
and mates it in place. It has no instance identity and cannot express a
repeated, movable, jointed part.

Assemblies need repeated placement of one definition, per-placement rigid
transforms, joints, and element identity that survives both. The crate scaffold
`crates/onecad-assembly` (`math.rs` RigidTransform/Quat/Frame, `occurrence.rs`
OccurrenceTree with `MAX_DEPTH = 8`, `MAX_OCCURRENCES = 2000`) exists, untested.

## Decision

1. **One document holds the assembly.** Each `ComponentDef` owns its own
   `Timeline`, `BodyRegistry` and sketches. The root component is the assembly.
   Definitions are embedded copies; live external links are deferred.
2. **Migration.** A schema-1 document migrates to a root `ComponentDef` holding
   the existing timeline with zero occurrences, with no semantic change.
   `DOCUMENT_SCHEMA_VERSION` becomes 2. Newer versions are refused. Source
   bytes are preserved, and a failed migration leaves the original intact.
3. **Occurrence** = `{id, def, parent, local rigid transform, grounded, name}`.
   Placement is assembly state owned by the root, **not** a part-timeline
   record. Undo/redo goes through the same `DocumentSession` single writer.
4. **Identity** = `OccurrencePath` + existing `ElementRef`. Repeated
   occurrences share element ids within their definition.
5. **Rigid transforms only.** Reflection is refused; a mirror is a new
   definition.
6. **`PlaceComponent` stays** as the part-level library insert (baked body),
   distinct from assembly insert. It is not migrated into occurrences.
7. **Worker sessions.** One session per `(document, componentDef)`, reusing
   the `OpenSession` machinery with fencing per component. Each definition
   regenerates once in its local frame; Rust composes transforms.
8. **Cross-component sketch references (ONECAD-399)** are deferred past the M3
   core. v1 uses a frozen projected copy.
9. **Cycle policy.** The occurrence tree is acyclic; a definition may not
   contain an occurrence of itself, directly or transitively.

### Rejected alternatives

- **One document per part, assembly as a linking file.** Rejected for v1:
  multi-file consistency, save/reopen atomicity and undo across files cost
  more than the embed. Kept as the deferred external-link path.
- **Occurrences as timeline records.** Rejected: dragging would rewrite
  history and regen parts that did not change. Placement is state, not
  feature.
- **Baking each placement into the root body (extend `PlaceComponent`).**
  Rejected: no shared definition, no edit-once propagation, no instancing
  (ADR-0021), no joints.
- **Supporting reflection in transforms.** Rejected: flips winding and
  handedness through geometry, normals and solver frames for a case a mirrored
  definition handles.

## Consequences

- `Document` gains a component table; existing timeline/body code moves under
  `ComponentDef` behind the migration. Every consumer of `Document.timeline`
  needs a root-component accessor. Blast radius is not yet measured.
- Edits to a definition regenerate that definition once, not per occurrence;
  the worker holds N sessions and the supervisor must fence each.
- Schema-2 documents are not readable by older builds (refused, not guessed).
- Depth 8 and 2000 occurrences are provisional caps (see ADR-0021).
- No part of this is implemented beyond the crate scaffold; it is untested.

## Open questions

- Occurrence transforms outside the timeline vs captured positions: is
  undo-by-session enough, or do saved states need captured placements?
- Solver spike scope (ADR-0022) and whether it blocks the schema bump.
- Deferring ONECAD-399: is a frozen projected copy acceptable to users, and
  how is staleness shown?
- Mirrored occurrences refusal: error text and a "mirror to new definition"
  affordance.
- Workbench and tree visual designs are supplied by the user; none are
  assumed here.
