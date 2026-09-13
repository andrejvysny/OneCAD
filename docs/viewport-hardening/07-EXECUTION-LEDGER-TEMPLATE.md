# Execution ledger template — do not treat this file as results

Copy this template to the package-relative `execution/STATUS.md` when implementation starts. Create the `execution/` directory if absent. Keep code and evidence checkpoints current before ending or compacting a session. Also append concise progress to the repository's existing `TODO.md`; do not replace its historical contents.

## 1. Session identity

| Field | Value to record at implementation start |
|---|---|
| Program/spec version | VP-HARDENING 1.0 |
| Design baseline | 65b4c60eb226a201f2fa3eb565d7283b1c63b5ac |
| Actual local HEAD | NOT RECORDED |
| Branch | NOT RECORDED |
| Pre-existing changed/untracked files | NOT RECORDED; inspect without modifying |
| Current dirty-diff hash | NOT RECORDED |
| Claude Code version | NOT RECORDED |
| Main model / observed subagent model | NOT RECORDED; select Fable 5.1 and verify actual routing |
| Bun / Node / TypeScript / Three.js | NOT RECORDED from installed/locked dependencies |
| Rust / CMake / compiler / OCCT fingerprint | NOT RECORDED |
| Native machine and display | NOT RECORDED |
| Active heavy build process / owner | NONE RECORDED; verify process table before starting |

These placeholders belong only in the execution template. The design choices are already in the specification; do not ask the user to fill this table as a prerequisite to starting code inspection.

## 2. Package status

| Package | State | Focused evidence | Integrated/native evidence | Owner / next action |
|---|---|---|---|---|
| WP00 inventory | not-started | not-run | not-run | inspect current checkout |
| WP01 line units | not-started | not-run | not-run | depends on inventory |
| WP02 lifecycle | not-started | not-run | not-run | depends on inventory |
| WP03 ownership | not-started | not-run | not-run | follow guide graph |
| WP04 validation | not-started | not-run | not-run | follow guide graph |
| WP05 appearance | not-started | not-run | not-run | follow guide graph |
| WP06 camera/input | not-started | not-run | not-run | follow guide graph |
| WP07 picking | not-started | not-run | not-run | follow guide graph |
| WP08 curve sampling | not-started | not-run | not-run | follow guide graph |
| WP09 normals | not-started | not-run | not-run | follow guide graph |
| WP10 protocol/origins | not-started | not-run | not-run | single cross-layer owner |
| WP11 sketch resources | not-started | not-run | not-run | follow guide graph |
| WP12 shared quality | not-started | not-run | not-run | follow guide graph |
| WP13 sections/edges | not-started | not-run | not-run | follow guide graph |
| WP14 preparation/BVH | not-started | not-run | not-run | follow guide graph |
| WP15 finishing | not-started | not-run | not-run | follow guide graph |
| WP16 qualification | not-started | not-run | not-run | after implementation gates |

Use separate implementation and evidence states. Allowed package states: `not-started`, `reproducing`, `implementing`, `focused-pass`, `integrated-pass`, `native-accepted`, `blocked-native`. Device-specific blocks may be recorded as a reason `blocked-device`. Never call an unexecuted physical case accepted because its reducer counterpart is green.

## 3. File ownership and heavy-lane lease

| Stream | Package | Exact owned paths | Shared seam / integration owner | Start checkpoint | State |
|---|---|---|---|---|---|
| none | — | — | — | — | idle |

Heavy lane: owner, command, PID when available, working directory, output path, start checkpoint. Before reusing an apparently stale lease, inspect whether the process is still running. Do not kill unrelated user builds.

Maximum three active work streams. Only one writer per shared integration file. Review may read a changing worktree, but final review must identify a stable diff checkpoint.

## 4. Work brief issued to an implementer

```markdown
Package and role:
Current source checkpoint:
Owned exact files:
Files allowed to read but not modify:
Relevant SPEC/NUM sections and decision IDs:
Required behavior:
Explicit non-goals:
Required failing reproduction:
Required focused test IDs:
Shared seams already agreed:
Heavy-build lease arrangement:
Return format: changes, red/green evidence, risks, integration notes.
```

A work brief must be self-contained about pixel units, coordinate spaces, and publication identity when those affect the package. Do not rely on a subagent having read the entire main conversation.

## 5. Test execution record

```markdown
Test ID:
Source checkpoint and dirty-diff hash:
Fixture/version/seed:
Lane: U/W/X/G/N/P/B
Command or physical procedure:
Build and kernel identity:
Expected observation:
Actual observation:
Exit code / assertion result:
Raw evidence path:
Status: pass/fail/not-run/blocked
Limitations and skipped combinations:
```

For a numerical proof, state assumptions and the domain actually bounded. For a sample check, state the samples and that it is not a global proof. For timing, record workload, quality state, warm/cold mode, raw samples, and hardware.

## 6. Deviation and risk record

```markdown
Deviation ID / decision affected:
Evidence contradicting the original assumption:
Minimal reproducer or installed API source:
Proposed narrow correction:
User-visible behavior affected:
Wire/persistence/identity impact:
Tests that will validate the correction:
Status and next independent work:
```

Record baseline failures separately from introduced failures. Preserve the original failing logs. Do not assign all failures to the pre-existing baseline without comparing checkpoints.

## 7. End-of-session handoff

State exactly: completed code, passing focused gates, passing integrated gates, native/physical evidence still missing, current source checkpoint, uncommitted changes, active owners/processes, and the next executable action. Include a short ordered list of next packages and their satisfied/unsatisfied dependencies.

Do not use a broad percentage as the delivery claim. “Implemented WP06; TEST-CAM-01–08 pass in U/G; N/P blocked until native rebuild” is useful. “Camera 100% done” is not.
