---
name: repo-protocol-fixtures
description: What the C++ worker_harness actually implements vs what protocol/fixtures/README.md documents, and how a new canonical fixture gets run
metadata:
  type: project
---

Authoring a `protocol/fixtures/*.ndjson` fixture: the README oversells the C++ harness.

**Why:** `protocol/fixtures/README.md` describes a matcher richer than
`worker/tools/harness/main.cpp` implements, and a fixture written from the README alone fails in
ways that look like a worker bug.

**How to apply:**

- `worker_harness` implements only `send` and `expect`, with `$any` / `$present` / `$hex64` and —
  since T4.4 — a per-expect `"tolerance": {"abs":…, "rel":…}` key (stripped before matching).
  NOT implemented: `$hex256`, `$capture`/`$ref`, `binSha256`, `@file`, `drain`/`expectAny`, and the
  STANDALONE `{"tolerance": …}` directive LINE.
- Never add a standalone `{"tolerance": …}` line: the Rust lane
  (`onecad-protocol` `messages.rs` `ndjson_fixtures_parse_into_message_types`) panics on any
  directive that is not `send` or `expect`. Put the key inside the `expect` object.
- Array matching is `actual.size() >= expected.size()` with element-wise subset — so a 2-element
  `["$any","$any"]` matcher pins "at least two", not "exactly two".
- A new fixture is auto-run by TWO lanes without any registration: `interop_smoke`
  (`worker/tests/interop/check_interop.sh` globs the directory) and the Rust parse test. Only the
  named per-fixture `ctest` entry in `worker/tests/CMakeLists.txt` is hand-written.
- Solver-lane fixtures bind the C++ lane only — `onecad-worker-stub` implements none of §7.4 and
  is not expected to replay them.
- Rounds that need a nonzero solved double belong in a worker test, not the fixture: the house
  split is "the contract in the fixture, the numbers in `worker/tests/test_*.cpp`".
- The harness binary is `worker/build/tools/harness/worker_harness`, NOT `worker/build/worker_harness`.
  Run one by hand: `worker_harness --worker worker/build/onecad-worker --fixture <path>`; on a
  mismatch it prints the matcher AND the raw response, which is the cheapest way to discover a
  verb's actual result shape (expect a wrong value on purpose to dump it).
- `CloseSession` answers `{"sessionClosed":true}`, not `{"sessionOpen":false}`.
