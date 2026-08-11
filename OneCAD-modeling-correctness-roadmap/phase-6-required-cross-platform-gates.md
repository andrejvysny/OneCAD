# Phase 6 Specification — Required Cross-Platform Release Gates

Duration: 2–4 weeks; overlap only with spare or external capacity  
Prerequisites: Phase 0 and Phase 4 stability evidence  
Priority: P2 release confidence  
Gate name: `REQUIRED-CROSS-PLATFORM`

## Rationale

CI now runs meaningful Linux and macOS lanes. `TODO.md:39` records that no checks were required at the reviewed commit, but branch-protection state must be verified from a dated GitHub ruleset/settings snapshot before acting on that claim. Linux Chromium remains unclassified and Windows is absent. A green workflow is evidence only if it blocks known-bad changes and its platform claims are explicit.

## Goals

1. Configure required checks after their stability criteria are met.
2. Keep whole-test retries explicit and visible.
3. Move suitable frontend/Chromium work to the stable Linux runner after prerequisites.
4. Classify the Linux Chromium Boolean-pick difference.
5. Add Windows core/protocol/process smoke.
6. Preserve macOS packaging and WebKit fidelity.

## Non-goals

- Replacing macOS shipping gates with Linux.
- Cross-host digest equality.
- Running untrusted fork code on the self-hosted runner.
- Full signed/notarized release automation without credentials.

## Work package 6.1 — Required-check policy

### Candidate required checks

- frontend build and Vitest,
- trusted or hosted worker CTest path appropriate to event trust,
- macOS full Rust workspace with real worker,
- Chromium Playwright,
- WebKit Playwright,
- T0 semantics,
- packaging linkage smoke on release branches.

### Stability entrance criteria

A check can become required when:

- its assertions are condition-based rather than deadline-based,
- failure artifacts are retained,
- it has no unclassified flake over an agreed sample,
- fork/trust routing is safe,
- owners understand expected duration and cost.

## Work package 6.2 — Enforce the settled zero-retry baseline

Current and settled baseline is retries 0. Preserve it unless the user explicitly reopens the decision.

Use:

- `trace: retain-on-failure`,
- screenshots on failure,
- video only for selected hard-to-reproduce suites,
- one click per consequential action,
- readiness and postcondition polling.

If the user later chooses one CI-only retry, treat that as a recorded policy change: the first attempt must remain visible and the flake backlog must stay tracked. A retry must never turn a known failing check into the only green signal.

## Work package 6.3 — Linux runner prerequisites

Install and verify:

- unzip for Bun setup,
- Chromium runtime libraries listed in `ci.yml`,
- browser cache strategy,
- fonts and SwiftShader dependencies.

Maintain the trusted-event `if:` conditions. Do not add `pull_request_target` or unfiltered fork execution.

Relevant source: `.github/workflows/ci.yml:20-27,47-70,391-435`.

## Work package 6.4 — Linux Chromium Boolean pick

### Investigation matrix

- software rendering versus available GPU path,
- device pixel ratio,
- camera settlement,
- line/face layer raycast behavior,
- body mesh readiness,
- WebGL precision and readback,
- mock seed geometry and viewport size.

### Required outcome

Classify as one of:

- product defect reproduced under a valid rendering environment,
- test coordinate/race defect,
- unsupported headless rendering environment with a documented reason.

Do not simply widen a timeout. Add a probe that records the ray, candidate layers and hit result.

## Work package 6.5 — Windows smoke

Mandatory hosted-Windows subset:

- Rust core and protocol tests that do not require a staged OCCT worker,
- container save/open path semantics,
- MESH1 validation,
- externalBin naming/path construction tests,
- protocol fixture parsing.

Second step, only when a Windows worker artifact is available:

- sidecar process spawn,
- stdout hygiene and epoch restart,
- Tauri externalBin staging smoke,
- worker protocol codec/fixtures.

Add pinned OCCT worker CTest on Windows only when artifact/build cost is understood. The Phase 6 acceptance gate requires the mandatory subset; worker-dependent Windows checks remain separately classified until prerequisites exist.

## Work package 6.6 — Failure artifacts and observability

For each required lane retain:

- exact commit and toolchain,
- OCCT fingerprint,
- worker stderr,
- Playwright trace/screenshot,
- kernelbench summary and case artifacts,
- machine identity for same-host digest lanes.

Redact nothing needed for diagnosis, but never mix secrets into artifacts.

## Work package 6.7 — Release gate matrix

Document which claims each platform proves:

| Claim | Linux | macOS | Windows |
|---|---|---|---|
| OCCT fingerprint | yes | yes | planned |
| Worker CTest | yes | yes | later |
| Rust real-worker workspace | limited | yes | smoke first |
| Chromium UX | after classification | yes | future |
| WebKit/WKWebView proxy | no | yes | no |
| Packaging linkage | ELF RPATH evidence | app dylib relocation | sidecar/DLL smoke |
| Same-host digest | yes, stable runner | no hosted digest | later |
| Portable semantics | yes | yes | later |

## Acceptance gates

- Required checks configured and documented.
- Self-hosted trust guard preserved.
- Zero-retry baseline recorded and enforced unless explicitly reopened.
- Linux Chromium classified.
- Mandatory hosted-Windows subset green; worker-dependent checks explicitly classified by prerequisite.
- macOS packaging and WebKit remain required where appropriate.
- T0 semantics match Linux/macOS.

## Risks

- Required checks can block a solo-founder direct-commit workflow if enabled before flake removal.
- Hosted images change toolchains; digests must remain off hosted cross-machine lanes.
- Windows OCCT build time can dominate CI cost.
- A Linux headless rendering difference may not represent shipping Tauri behavior; classify the claim rather than forcing parity.

## Rollback

Branch protection changes are reversible in GitHub settings. Workflow changes should land separately from product correctness patches.
