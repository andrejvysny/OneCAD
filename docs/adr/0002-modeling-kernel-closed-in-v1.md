# 0002 — The modeling kernel is closed to addons in v1

- Status: Accepted
- Date: 2026-08-08

## Context

The entire point of the OneCAD-CPP migration is fixing a defect the legacy stack
never fixed (H5-B): parametric edits breaking topological naming. The machinery
that fixes it — `ElementId` minting, descriptor evidence, the resolution ladder
with its score/margin thresholds, `NeedsRepair`, checkpointed regeneration — is
the strongest and most delicate area of the codebase.

An addon API that can inject operations into that machinery would put third-party
code inside the invariant that the product exists to protect.

## Decision

Addons may build on modeling, inspect modeling content and invoke supported
modeling operations. They may **not**:

- add `KnownOperation` variants,
- register OCCT algorithms,
- inject regeneration handlers,
- talk to the C++ worker,
- mutate history representation directly.

All addon access goes through a modeling service facade:

```
Addon → ModelingService → modeling internals
```

never `Addon → history.rs`, `regen.rs`, `WorkerSession`, or the OCW1 protocol.

`KnownOperation` stays a strongly typed Rust enum. The existing `Operation::Opaque`
representation is forward-compatibility for documents written by newer builds; it
is **not** a generic plugin execution mechanism, and must not become one.

## Cost

Some genuinely reasonable addon ideas (a new feature type, a custom blend) are
impossible in v1 and require a built-in module instead. Accepted: a silent
mis-bind caused by third-party geometry code would be far more expensive than a
missing capability, and this boundary can be widened later without redesign.
