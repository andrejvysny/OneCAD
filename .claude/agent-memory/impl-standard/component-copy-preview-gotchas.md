---
name: component-copy-preview-gotchas
description: PreviewOp matePlacement + interaction PlaceComponent/Copy session gotchas (harness, fmt, op_error)
metadata:
  type: project
---

- Worker harness `expect` is subset-only: it cannot assert a key is ABSENT; pin absence in a Rust real-worker test instead.
- `cargo fmt --all` fails while another agent has an in-flight onecad-ui module (missing file); run `rustfmt --edition 2021 --check <files>` on your own files.
- `ParameterSet.op_error` is private to tests; assert via `params.readiness()` being `Readiness::Invalid`. `advance()` (parameter_flow.rs) clears op_error and the retained `mate_placement`.
- `ModelingOptions::ComponentCopy` is session state only: handled before `apply` in `change_modeling_options` because it leaves the record unchanged (the no-op early return would drop it).
- Worker emits `matePlacement` only when the seat MOVED past epsilon; a mated Copy whose seat is already exact gets no echo and is refused by design.
- macOS has no `timeout`; run long cargo test groups backgrounded.
