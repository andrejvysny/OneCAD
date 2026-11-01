---
name: edge-op-overflow-wiring
description: Fillet/Chamfer overflow=auto wiring facts - failure diagnostics, fixture lifecycle, history-test harness
metadata:
  type: project
---

- Failed-step diagnostics: PlanExecutor `candidate_diagnostics` adds the generic OP_FAILED diagnostic only if NO error diagnostic is present; an appended error diag (e.g. EDGE_OP_OVERFLOW_UNSUPPORTED) suppresses it. Native r>=4 fillet on 20x30x4 box fails with diag code FILLET_START_SOLUTION_FAILED (not OP_FAILED) and `lastValidStep: null`.
- NDJSON fixtures: a prepared ExecutePlan must be AcceptPrepared (or discarded) before the next ExecutePlan (PROTOCOL_ERROR otherwise); every plan step emits a planStep event that must be expected.
- test_fillet_history.cpp `execute_candidate_op` + `ft::op` is the cheapest op-level harness; override params by key to add `overflow`.
