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
- The worktree is shared with concurrent sessions (see the user-level memory). Unrelated files show
  as modified mid-task — e.g. `chunks_exact` → `as_chunks` rewrites across `src-tauri/tests/*.rs`.
  Check whether a diff relates to your change before claiming or reverting it; `cargo fmt` does NOT
  make that rewrite.
- `cargo clippy --workspace --all-targets -- -D warnings` enforces `clippy::type_complexity`: a
  3-tuple return with an `Arc<dyn Trait>` in it needs a `pub type` alias.
- Real-worker targets need BOTH `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker` and
  `ONECAD_REQUIRE_WORKER=1`, run from `src-tauri/`.
