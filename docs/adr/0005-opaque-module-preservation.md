# 0005 — Unknown module state is preserved verbatim

- Status: Accepted
- Date: 2026-08-08

## Context

Once documents can carry third-party state, a user will eventually open a project
on a machine where one addon is not installed. The tempting behaviors — refuse to
open, or open and silently drop what could not be decoded — both destroy work that
the user never chose to discard.

The codebase already applies this principle locally: unknown `opType` records
survive as `Operation::Opaque`, and unknown top-level keys survive in `Extra` on
both the manifest and the document.

## Decision

A formal file-format invariant:

> If OneCAD cannot interpret module-owned state, it must preserve it during
> load and save whenever possible.

Concretely:

- an unavailable module's payload is carried through untouched — never rewritten,
  never normalized, never pruned;
- a document opens as long as the platform and modeling content are valid; a
  missing addon is a non-blocking notice naming what is absent, not a refusal;
- disabling an addon unregisters its contributions but does **not** touch its
  document data; uninstalling removes the package, not the data;
- when modeling deletes an entity an addon referenced, the platform emits
  `entity.removed` and installed addons clean up. If the addon is absent, stale
  references remain in its opaque blob until it returns. Nothing edits a blob it
  cannot parse.

Migration of a module's own state is owned by that module, under a platform-run
lifecycle: load original bytes → migrate → validate → commit. A failed migration
keeps the original data and reports; it never half-writes.

## Cost

Documents can accumulate state no installed code understands, and OneCAD cannot
garbage-collect it. Accepted — data integrity beats aggressive cleanup, and the
alternative failure mode is silent data loss.
