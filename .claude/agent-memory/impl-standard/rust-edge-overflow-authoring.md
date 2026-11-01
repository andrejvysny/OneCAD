---
name: rust-edge-overflow-authoring
description: Rust-side Fillet/Chamfer overflow opt-in wiring points and test gotchas
metadata:
  type: project
---

- Stamp site: `operand_binding::with_overflow_auto` (create via `bind_edge_op`; edit via `parameter_set::candidate` override loop only, NOT the Authored write-back). Tests compare via `parameter_support::with_auto_overflow`.
- Adding a field to FilletParams/ChamferParams touches ~70 struct literals (no Default/spread); script it: insert after `tangent_closure_version:` line, dedupe if a literal is matched twice.
- Preview DTOs (`PreviewResultDto`, `EditPreview`) carry no diagnostics; advisory only reads committed `FeatureDto.diagnostics` (overflow_advisory.rs).
- Env-only failures w/o ONECAD_WORKER_PATH: host.rs:168 panics in onecad-interaction real_* and 2 onecad-ui tests.
