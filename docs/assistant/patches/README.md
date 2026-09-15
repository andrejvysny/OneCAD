# Upstream patches awaiting application

Changes that belong in a dependency, not in OneCAD. They live here because this
session has read-only access to those repositories.

## `agentkit-recover-orphaned-running-tasks.patch`

**Target:** `andrejvysny/agentkit`, applies cleanly on `a450d6ff` (the pin OneCAD
currently builds from `scripts/bootstrap-agentkit.sh`).
**Addresses:** finding F06 of the assistant hardening review.

A recovery pass expires a lease, ends the attempt, and — if the process dies
before re-dispatching — leaves the task `running` with no lease. Every later
expiry sweep finds nothing, because `expireStaleLeases` can only report a lease
that still exists and this one was already deleted. The chat stays busy forever
and cancellation has no local execution to abort. The only record of such a task
was an in-memory `Set`, which the crash takes with it.

The patch adds `TaskStore.selectOrphanedRunningTasks(limit)` and a bounded
periodic sweep in `SingleProcessTaskRunner`. It needs **no schema change** — that
is deliberate, because the SQLite adapter refuses to open a database whose schema
version differs and ships no migrations, so a version bump here would strand
every existing user's conversations.

The periodic pass does not expire leases, and the commit message explains at
length why: on a timer that cannot be done safely, and an existing AgentKit test
caught it during development.

### To apply

```sh
cd <your agentkit checkout>
git checkout a450d6ff -b fix/recovery-orphaned-running-tasks
git am < path/to/agentkit-recover-orphaned-running-tasks.patch
bun run ci
```

`bun run ci` was run here: **1654 passed, 1 skipped, 1 failed**. The single
failure is `packages/client/tests/resume.test.ts` — *"a failed log read breaks the
body, and the client resumes over it"* — which fails identically on the pristine
pin with these changes stashed, so it is pre-existing and unrelated.

### Then, in OneCAD

Bump `AGENTKIT_COMMIT` in `scripts/bootstrap-agentkit.sh` to the resulting
revision. **Do not** edit `.agentkit-src/` directly — it is generated output and a
rebuild discards it.
