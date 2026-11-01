---
name: worker-fixture-harness-limits
description: C++ worker_harness fixture matcher gaps and canonical fixture wiring for protocol/fixtures
metadata:
  type: project
---

The C++ `worker_harness` does not implement `$capture:`/`$ref:` (README documents them, but only `$present`/`$any`/`$hex64` work); use `$present` for measured numbers. `snapshotId` after an AcceptPrepared can be higher than expected once DiscardPrepared rounds ran earlier in the file; use `$present`. Each new protocol/fixtures/*.ndjson also needs its own `add_test(canonical_*)` in worker/tests/CMakeLists.txt (Rust parse lane globs the dir). `.clang-format` lives at repo root, not worker/. Do not run `git stash` in the shared tree: other sessions have uncommitted work.
