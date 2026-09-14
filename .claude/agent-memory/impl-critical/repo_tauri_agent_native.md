---
name: repo-tauri-agent-native
description: Gotchas for tools/tauri-agent's macOS native backend — Swift trapping conversions, testing without posting real HID events, bun stdin typing, screencapture -l lying
metadata:
  type: project
---

Facts that bit while hardening `tools/tauri-agent/src/platform/**` (the real-HID backend).

- **Swift's fixed-width initialisers trap, they do not saturate.** `pid_t(1e10)`, `Int32(1e20)`,
  `UInt32(durationMs * 1000)` abort the process with SIGTRAP — verified: a 3-line repro exits
  **133**. In a helper that posts HID events that means dying mid-drag with a button still held
  at the OS level, so every numeric argument off the wire goes through `isFinite` + clamp +
  `Int32(exactly:)`/`pid_t(exactly:)` before it reaches a fixed-width type.
- **Never post a click, scroll or key from a test or a manual probe.** They land in whatever
  application is frontmost — a developer's editor. `tests/fixtures/fake-helper.ts` speaks the same
  JSON-line protocol and posts nothing; drive `HelperClient`/`MacInput` through it (they take an
  injectable argv/client). Against the real helper, only cursor reads and a move to the point the
  cursor is already at are safe. This applies to ad-hoc `printf ... | helper` probes too.
- **`TAURI_AGENT_REQUIRE_HELPER=1`** turns `tests/helperProtocol.test.ts`'s macOS/swiftc skip into
  a hard failure (mirror of `ONECAD_REQUIRE_WORKER`). Without it the whole file skips silently.
- **`Bun.stdin.stream()` is not async-iterable** under this repo's `bun-types`; `for await` over it
  fails `tsc` with TS2504. Use `.getReader()` and a read loop.
- **`screencapture -l <windowId>` can photograph a DIFFERENT window of the same pid**, and prints
  `could not create image from window` to stderr (often with exit 0) when the id is stale. The only
  local evidence is the image size: one capture of a known point-size area has one scale, so a
  width ratio that disagrees with the height ratio means the wrong window or display.
- **`NSRunningApplication.activate` is not proof of frontmost.** Verify with a separate read-only
  `NSWorkspace.shared.frontmostApplication` check; re-calling the activating verb to confirm biases
  its own answer.

**Why:** each of these is invisible from the type signatures, and the first two cost the user their
desktop rather than a failed test.
**How to apply:** when touching `tools/tauri-agent/src/platform/**` or any test that drives it.
