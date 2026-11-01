---
name: component-placement-pipeline-gotchas
description: App-level real-worker test harness + api::apply_operation limits for component placement drafts
metadata:
  type: project
---

- `api::apply_operation` accepts only fresh AddOperations or ONE UpdateOperationParams; an Add+swap-to-Detach batch is refused before the session sees it, so "Copy" = single AddOperation of `library::detached_record`.
- Real-worker app tests without a scheduler: `AppState::new(factory over wm)` + manual `rt.run_regen(ToEnd{from:0})` after each apply/undo; `api::preview_operation`/`preview_context` work without one. See `crates/onecad-app/tests/component_placement_pipeline.rs`.
- `cargo test -p onecad-app --lib library --test X` applies the `library` filter to every target, so integration tests report 0 run; run each `--test` separately.
- `onecad_library::tables::Iso4762Table` (not `tables::iso4762`, private). M6 mate on a bore ends head-under/shank-up (bbox z -6..20 on a 5 mm plate).
- Draft/hover mate targets use `library::stage_mate_target(.., &GeometryReadFenceDto)` (staged ids: not persisted, not dirty, adopted by `apply_operation` of a record referencing them). `promote_mate_target` persists+dirties: commit-time/tests only.
- PreviewOp `matePlacement` = mate resolved (moved or not) via `OpOutcome::mate_resolved_placement`; ExecutePlan's `mate_placement` stays moved-only.
