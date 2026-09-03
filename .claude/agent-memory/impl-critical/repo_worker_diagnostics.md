---
name: repo-worker-diagnostics
description: How a worker op gets a warning onto a SUCCESSFUL planStep, what PlanExecutor drops, and the profile-lift/classify helpers
metadata:
  type: project
---

Getting an advisory out of a C++ op and onto the wire.

**Why:** the obvious channel (`perStepResults[].diagnostics`) is populated ONLY for a FAILED
step, so an Ok-step warning written there is invisible; and appending to a failed step's
diagnostics silently rewrites the user-visible failure message.

**How to apply:**

- `OpOutcome::diagnostics` already reaches an Ok step: `merge_outcome` folds it into
  `CandidateResult`, `candidate_diagnostics` bounds it, and `emit_plan_step` puts it in the
  `planStep` EVENT payload. No PlanExecutor change is needed for a success-path warning.
- `bounded_diagnostic` REBUILDS the object from an allowlist: `severity` (info|warning|error),
  `code` (≤128), `message` (≤4096), plus optional `stage` (≤64), `reasonCode` (≤64) and
  `evidence` (object, dump ≤64 KiB). Anything else is dropped silently.
- NEVER append an advisory to a Failed/Unsupported outcome: `execute_ops` sets
  `perStepResults[].message = diagnostics.back().message`, so a trailing advisory replaces the
  failure text. `ops::attach_profile_diagnostics` is the shared guard.
- A test that asserts an Ok-step diagnostic must capture `HandlerContext::emit`
  (`HandlerContext ctx{tok, [](int){}, [&](Envelope& f){ ... }}`); the event payload is in
  `Envelope::result`, the name in `event_name`, the step in `step_index`.
- `PreviewOp` reaches ops via `execute_candidate_op`, so preview and commit share every op
  executor and therefore `ops::build_profile_face` — a resolution change is automatically
  identical in both lanes; there is nothing extra to wire.
- Lifting a sketch UV point into the face's 3D frame: `loop::FaceBuilder::sketchPlaneToGpPln`
  + `toGpPnt` (made public 2026-09-03). Do NOT re-derive the frame — `gp_Ax3(origin, normal,
  xDir)` re-orthonormalizes, so a hand-rolled transform diverges from the built face. Classify
  with the 3D `BRepClass_FaceClassifier(face, gp_Pnt, tol)` overload.
- `ops::build_profile_face` now ends in two DEFAULTED params — `(sketch_params, region_id,
  version, err, diagnostics_out = nullptr, region_anchor = nullopt)`. Keep new optional
  parameters after `err` and defaulted; two concurrent WPs both needed `diagnostics_out` and
  this shape was the only one both could call without an overload.
