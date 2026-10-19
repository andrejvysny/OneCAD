---
name: interaction-real-tests-need-worker
description: cargo test -p onecad-interaction has 4 real_* tests that panic without ONECAD_WORKER_PATH; use --no-fail-fast
metadata:
  type: project
---

real_point_center (2), real_sketch_modeling (1), transform_modeling::real_transform_preview_commit_cancel_and_history panic at native_plate_support/host.rs:168 when ONECAD_WORKER_PATH unset. Env-only, not regressions. Plain `cargo test` stops at first failing binary, hiding later suites; pass `--no-fail-fast`.
