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
