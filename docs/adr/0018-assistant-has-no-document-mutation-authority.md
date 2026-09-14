# 0018 — The assistant has no document mutation authority in v1

**Status:** Accepted (2026-09-14)
**Depends on:** ADR-0002 (the modeling kernel is closed to addons in v1).

## Context

The assistant program's eventual goal is an agent that builds and edits real parametric
geometry. Reaching that safely needs an isolated draft workspace, a Rust-minted
authorization grant bound to an approved work order, idempotent operation ids, durable
commit receipts, and a native adoption path that produces one ordinary undo group. None of
that exists yet.

The tempting shortcut is to ship the chat surface with a CAD tool wired to the existing
`CadClient` and add the safety machinery afterwards. That inverts the risk: the dangerous
capability lands first and the guards chase it.

## Decision

**In this work package the assistant cannot change a document at all**, and that is
expressed structurally rather than by convention.

1. No CAD tool is registered in the agent's tool catalog. Not a disabled one — an absent
   one. A tool that is not in the catalog cannot be called by a confused model, a prompt
   injection, or a bug.
2. AgentKit routes every write through a `ProposalApplier`. This build supplies a
   `NoopProposalApplier` whose `apply()` throws. AgentKit's `recoverOnBoot` requires a
   concrete `ProposalService`, so the object must exist; making it a no-op means **the one
   place in the architecture where a CAD write could land is occupied by something that
   cannot perform one.**
3. The OCAK1 verb table (`docs/assistant/wire-protocol.md` §5) has no verb by which the
   sidecar can mutate a document, read the filesystem, spawn a process, or open a socket.
   Adding one is a work package with its own authorization design, not a new table row.
4. The assistant module registers no command that mutates document state, and it does not
   import `@/ipc/client` for anything kernel-touching. Per ADR-0002 that path is closed to
   it regardless.

## Why say it this way

A guarantee that lives in a comment decays. A guarantee that lives in an absent tool and a
throwing applier is one a reviewer can verify by reading two files, and one that a later
change cannot erode by accident — replacing the no-op applier is a visible, reviewable act.

## Consequences

The exit condition for this work package is a durable local chat sidebar with read-only
scripted tools. That matches the implementation guide's own gate for this increment and
does not exceed it.

When mutation arrives it replaces exactly one object — the applier — behind the candidate,
grant, receipt and adoption machinery. Until then, `apply()` throwing is correct behaviour
and its test asserts it.
