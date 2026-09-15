---
name: repo-tauri-agent-helper-protocol
description: tauri-agent macOS helper protocol — where the version/protocol number is asserted, why release_all osState is banned from automatic paths, and how the host shadow held-state works
metadata:
  type: project
---

The Swift helper's `helperVersion`/`helperProtocol` (`src/platform/macos/helper/main.swift` head) are
asserted in THREE places that must move together, none of them a runtime check in `src/`:
`tests/helperProtocol.test.ts` (`--version` banner and the `--selftest` report) and
`tests/fixtures/fake-helper.ts` (its own `version` reply, mirrored by two `helperClient.test.ts`
assertions). A bump that misses one is a red test, not a refused helper — there is no protocol
negotiation on this wire at all.

**Why:** WP-F bumped 1.1.0/2 → 1.2.0/3 to add `release_all {force: [...]}`. The review that
prompted it found `osState: true` — which releases everything the OS reports held, the developer's
own hand on the mouse and their own modifiers included — on both automatic recovery paths
(`drainIfNeeded`, `recover`). `osState` now survives only as a manual escape hatch: no caller in
`src/` passes it, and `grep -rn osState src/` is the check.

**How to apply:** the replacement is a host-side `potentialHeld` shadow set in `HelperClient`,
latched before a verb that can leave something held and unlatched after the verb that releases it.
It is deliberately a SUPERSET — the helper releases a `force` name only when
`CGEventSource.buttonState`/`flagsState` confirms it down, so an over-wide guess is a no-op while a
missing name is a stuck button. Clear it only on a REPLY, never on an attempt. Anything added to
`release_all`'s arguments must be clamped/ignored rather than rejected (see
[[repo-tauri-agent-native]] on exit 133): it is the recovery verb, and failing the whole request
over one bad name leaves real input held.

`tests/fixtures/fake-helper.ts --osheld=<names>` models inputs a HUMAN is holding; they are
reachable only by `osState:true` or an explicit `force` name, which is how a test proves the user's
hand was left alone. `tests/helperProtocol.test.ts` runs the REAL helper and must stay read-only
with respect to the machine — never add a test that actually holds a mouse button or presses a key.

`TAURI_AGENT_REQUIRE_HELPER=1 bun test` turns "the real helper cannot run" into a hard failure;
without it the whole Swift lane skips silently. Editing `main.swift` triggers a rebuild on its own
(cached by source SHA in `build.ts`).
