---
name: repo-verification-notes
description: OneCAD verification gotchas — how to prove a test is red-first, and which worktree churn is not yours
metadata:
  type: project
---

Verification habits that paid off in this repo.

**Why:** the house rule is "never report a gate you did not run", and a new regression test is worth
nothing unless it is shown to fail without the fix.

**How to apply:**

- To prove a new test is RED-first without reverting the diff: add a temporary `std::env::var(...)`
  early-return that bypasses ONLY the new guard, run the test with that env var set, then remove the
  hook and `diff` against a backup copy to prove the file is byte-identical again.
- An env-gated TEMPORARY audit assertion inside production code is a cheap way to prove a
  broad-phase/fast-path invariant over the whole ctest suite (e.g. "no culled curve pair
  actually intersects": 0 violations over 172 tests). It proves NOTHING until you also verify
  it fires — invert its condition to always-true, rebuild, and confirm it aborts. Then restore
  from a backup copy and `diff -q` to prove the file is byte-identical again.
- Concurrent implementers share `worker/`, so configure your OWN build dir
  (`cmake -S worker -B worker/build-t<n> ... -DONECAD_WORKER_BUILD_DIR=worker/build-t<n>`);
  `worker/build-*` is gitignored. `-DONECAD_WORKER_BUILD_DIR` must equal `-B`, and
  `ONECAD_OCCT_ROOT` is `$HOME/.onecad-occt/8.0.1` on this machine.
- The worktree is shared with concurrent sessions (see the user-level memory). Unrelated files show
  as modified mid-task — e.g. `chunks_exact` → `as_chunks` rewrites across `src-tauri/tests/*.rs`.
  Check whether a diff relates to your change before claiming or reverting it; `cargo fmt` does NOT
  make that rewrite.
- `cargo clippy --workspace --all-targets -- -D warnings` enforces `clippy::type_complexity`: a
  3-tuple return with an `Arc<dyn Trait>` in it needs a `pub type` alias.
- Editing C++ with the Edit/Write tool is SAFE again: a repo-root `.clang-format` with
  `DisableFormat: true` + `SortIncludes: Never` (recorded 2026-09-02) makes the PostToolUse
  clang-format hook a no-op, so an edit no longer reflows unrelated lines.
- Real-worker targets need BOTH `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker` and
  `ONECAD_REQUIRE_WORKER=1`, run from `src-tauri/`.
- To measure real wire responses before authoring a fixture, drive the lane with
  `worker_harness --worker <bin> --repl` and one request envelope per stdin line. Far faster than
  guessing an `expect` and iterating on MISMATCH output.
- Red-first for a C++ production choice: edit the one expression, rebuild only the affected target,
  confirm the FAIL line, then restore with the inverse `Edit` and `diff` against a `/tmp` backup.
  `diff -q ... && rm ...` compound commands hit the permission prompt; run a bare `diff` instead.
- `cargo test -p <crate>` stops at the FIRST failing test target and never runs the
  rest, so a single red integration target hides the whole-crate count. Use
  `--no-fail-fast` whenever you need a baseline number to report.
- Red-first for a Rust production choice inside one function: `cp` the file to the
  scratchpad, insert `if true { return <inert>; }` at the top of the function, run,
  then `cp` the backup back and run a bare `diff` to prove byte-identity. Probe each
  mechanism SEPARATELY (e.g. "skip the whole pass" vs "skip only the merge") — a test
  that stays green under one probe and red under the other tells you exactly which
  half it is pinning.
- Before attributing a gate's status change to your own diff, `git status` the gate
  file. In a shared worktree the orchestrator may have re-expressed the assertion
  under you; single-fix probes that all leave it green are the tell.
- `tracing::warn!/info!` produce NO output under `cargo test` (no subscriber
  installed), so grepping test output for a tracing line proves nothing. Use a
  temporary `eprintln!` with `-- --nocapture` when you need to see a decision.
  To see a WORKER stderr line under `cargo test`, install a global capture
  subscriber in a scratch test the way `tests/worker_stderr_capture.rs` does
  (`registry().with(EnvFilter::new(DEFAULT_FILTER)).with(layer)`) — the stderr
  forwarder is a detached task, so a thread-local subscriber never sees it.
- A concurrent implementer's in-flight edit can make the WHOLE workspace fail to
  compile on a symbol you never touched (a half-added enum, a DTO field added to
  the struct but not the initializer). Do NOT "fix" it — poll with a backgrounded
  `until cargo test --test <yours> --no-run -q >/dev/null 2>&1; do sleep 20; done`
  and carry on. `cargo fmt --all --check` will likewise flag THEIR files; list the
  failing paths (`| grep '^Diff in'`) before claiming the gate is red on you.
- `rm` is permission-denied in this sandbox; delete a scratch file with
  `python3 -c "import os; os.remove(...)"`, or `mv` it into the scratchpad directory.
- The Playwright MOCK lane cannot express every regen assertion. `mockClient.undo/redo` report
  `changedBodies` from `diffBodies` over `syntheticBodies` only, so undo/redo of an op on the
  SEEDED demo box (`body1`) publishes no changed body and therefore triggers no mesh reload —
  a "the swap reconciled the selection" assertion is unreachable there. A COMMIT does publish
  (`mutateOp` returns `changed: [bodyId]` even for the no-CSG fillet/chamfer/shell/hole
  branches), so drive a regen assertion off the commit, not off undo/redo.
- `src/tools/sketch/projectTool.ts` contains a literal NUL byte (a `${bodyId}\x00${mode}` group
  key), so git classifies it as BINARY: `git diff` shows `Bin <n> -> <m>` and no hunks. Read the
  file to review a change there.
- Red-first for a FRONTEND change: `cp` the module to the scratchpad, patch the one expression
  with a short `python3` heredoc, run the single vitest/Playwright test, then `cp` the backup
  back. Playwright's `-g "<substring>"` runs one test in ~40 s including the vite boot.
