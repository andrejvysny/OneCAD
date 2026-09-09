---
name: wpg-fillet-acceptance-envelope
description: worker/tests/test_fillet_acceptance_envelope.cpp assertion shape and CandidateResult/FilletBuildResult diagnostic wiring for fillet acceptance tests
metadata:
  type: project
---

`worker/tests/test_fillet_acceptance_envelope.cpp` (ctest `fillet_acceptance_envelope`, registered `worker/tests/CMakeLists.txt` near the `fillet_builder`/`fillet_characterization` foreach block) is the tracked, asserting successor to the WP-G scratchpad probe. It is RED by design on cases 1-2 (approximated blends) until the WP-G G2 kernel change lands (SCHEMA §7.3 amendment, `FILLET_BLEND_APPROXIMATED`); cases 3/4/5/5b pass today.

Key facts for extending it or writing similar op-path-vs-direct-kernel tests:
- `session::CandidateResult.diagnostics` is `vector<nlohmann::json>` in the shape `OperationDiagnostic::to_json()` produces: `{severity, code, message, stage?, evidence?}` — `evidence` is omitted if empty or >65536 bytes serialized.
- `kf::FilletBuilder::build()` only populates `FilletBuildResult.diagnostics` on FAILURE paths today (via `fail_with_diagnostic`); a passing build's `result.diagnostics` is empty. `FilletChamferOp.cpp`'s `publish_result()` does NOT currently forward `FilletBuildResult.diagnostics` into the op's `OpOutcome.diagnostics` on success — only `build_fillet()`'s failure branch does (`failure.diagnostics.push_back(diagnostic.to_json())`). So a future "success warning" (e.g. `FILLET_BLEND_APPROXIMATED`) needs BOTH `FilletBuilder::accept_result()` to attach it AND `publish_result`/`OpOutcome` wiring to carry it through to the op path — check both when picking up G2.
- The op path's resolution ladder returns `NeedsRepair` for the case-5 (boss top-edge-near-seam) probe geometry, unrelated to the fillet acceptance question — that case must be asserted through `kf::FilletBuilder::build()` directly, not `session::execute_candidate_op`.
- `Standard_Failure::GetMessageString()` is deprecated in OCCT 8.0.1 in favor of `.what()`, but `FilletBuilder.cpp` itself still uses `GetMessageString()` — matching that existing pattern is fine since the worker builds `-Wall -Wextra` with no `-Werror` (deprecation warnings are non-fatal, pre-existing).
- A "coarseness ceiling" (`FILLET_BLEND_TOO_COARSE`) test case could not be reliably constructed from OCCT primitives without also tripping the profile-error gate for unrelated reasons — left unimplemented, flagged as a G2/unit-test follow-up.
