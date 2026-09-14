# VP-HARDENING acceptance evidence

Raw gate logs are local artifacts (`.gitignore:3` = `*.log`) and are never
committed. What is committed — and what the verifier checks — is a small JSON
summary per run. See `PLAN.md` D12 and `docs/viewport-hardening/04-ACCEPTANCE-AND-TESTS.md`
§13 for the design this implements.

## Layout

```text
docs/qa/viewport-hardening/
  acceptance-matrix.json   # 127 case rows; verified by scripts/verify-viewport-acceptance.mjs
  baseline/
    manifest.json           # committed: sha256 fingerprint of every local baseline *.log
    *.log                    # gitignored local artifacts, fingerprinted above
  runs/<run-id>/
    manifest.json            # committed: run identity + per-lane outcome (see schema below)
    tests.json               # committed: per-case-id pass/fail (see schema below)
    logs/<lane>.log           # gitignored raw stdout+stderr for that lane
```

## Recording a run

```bash
node scripts/record-viewport-run.mjs <run-id> <lane> [--case TEST-X-NN,...] \
  [--cwd <dir>] -- <command...>
```

Runs `<command...>` via `spawnSync` (no shell), streams its stdout/stderr to
the terminal and to `runs/<run-id>/logs/<lane>.log`, then writes/merges
`runs/<run-id>/manifest.json` and (when `--case` is given) `tests.json`.
Re-running the same run-id + lane replaces only that lane's entry. The
recorder's exit code is the command's exit code.

## `manifest.json` schema

```json
{
  "version": 1, "runId": "...", "createdAt": "ISO", "commit": "40-hex",
  "branch": "...", "dirtyDiffSha256": "sha256 of git diff excluding TODO.md/HANDOFF.md/CURRENT_STATE.md",
  "tools": { "node": "...", "bun": "...", "rustc": "...", "cargo": "...", "cmake": "..." },
  "worker": { "path": "worker/build/onecad-worker", "sha256": "..." },
  "lanes": {
    "<lane>": {
      "command": ["..."], "cwd": "repo-relative", "env": { "...": "..." },
      "startedAt": "ISO", "endedAt": "ISO", "durationMs": 0, "exit": 0,
      "logPath": "repo-relative", "logSha256": "...", "tailLines": ["..."]
    }
  }
}
```

## `tests.json` schema

```json
{
  "version": 1, "runId": "...",
  "cases": { "<caseId>": { "lane": "...", "result": "pass|fail", "exit": 0, "logSha256": "...", "summary": "..." } }
}
```

## Rule

Raw `.log` files stay gitignored. `manifest.json` / `tests.json` are
committed. A passing `acceptance-matrix.json` row's `evidence` field must cite
a `runs/<id>/manifest.json` (never a `.log`) that exists, parses, carries a
valid `commit`, and has at least one lane with `exit === 0`.

## Clean-checkout check (PLAN.md Verification step 6)

```bash
git worktree add <scratch> HEAD && node <scratch>/scripts/verify-viewport-acceptance.mjs
```
