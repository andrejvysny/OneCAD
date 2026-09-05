---
name: core-record-gotchas
description: onecad-core document-record gotchas — inputs are re-derived on deserialize, adding a ref field touches ~65 struct literals, and how to accept ONLY new insta snapshots
metadata:
  type: project
---

Things that cost a test cycle when changing an `onecad-core` document record or
its refs.

**Why:** none is derivable from the code without running the suite, and one of
them (`cargo insta accept`) will silently move a frozen file-format snapshot.

**How to apply:** read before adding a field to `document/record.rs` or
`document/refs.rs`, or before touching `crates/onecad-core/tests/snapshots/`.

- `OperationRecord` re-derives `inputs` at DESERIALIZE time from the op's params.
  A record built by mutating `rec.op` after `OperationRecord::new` therefore does
  NOT equal its own serde round-trip, and its history-prefix hash differs. Re-mint
  with `OperationRecord::new(rec.record_id, step, name, rec.op)` after any params
  mutation that adds a sketch/body/element dependency.
- Regex-substituting a struct literal is as dangerous as a line-offset script:
  a non-greedy `ComponentMate \{.*?extra: Extra::new\(\),` match lands on the
  NESTED `PrimaryRef`'s `extra`, not the outer struct's, and the result still
  compiles nowhere obvious. Anchor by exact line number after reading the site,
  and always `git diff --stat` for unexpected deletions before running the gate.
- Adding one field to `SketchRegionRef` required editing ~65 struct literals
  across `crates/onecad-core/{src,tests}`, `crates/onecad-regen/tests`,
  `src-tauri/src` and `src-tauri/tests` — there is no shared fixture; each test
  file hand-rolls its own profile. A scripted insert keyed on the preceding
  field's line is the practical route, but `region_identity_version` also names a
  field on `FinishSketchDto` (`src/dto.rs`), so exclude the DTO sites by hand.
- `cargo insta accept` / `INSTA_UPDATE` sweeps EVERY pending snapshot, including
  the frozen file-format ones a schema change must not move. To accept only new
  cases: run the test, confirm the existing ones still pass, then rename each
  `*.snap.new` to `*.snap` yourself, dropping the `assertion_line:` metadata line
  insta writes into `.new` files (the committed `.snap` files carry only
  `source:` and `expression:`).
- `cargo fmt` reflows a long `#[serde(...)]` attribute onto multiple lines once it
  carries three or more arguments; write it one-line and let fmt split it.
