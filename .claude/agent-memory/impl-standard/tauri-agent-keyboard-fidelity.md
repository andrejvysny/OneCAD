---
name: tauri-agent-keyboard-fidelity
description: WP-H keyboard fidelity contract — envelope fidelity field, release_all delivery honesty, layout verb wiring across tauri-agent
metadata:
  type: project
---

`tools/tauri-agent` ActionResult gained `fidelity?: "text_entry" | "physical_key"` (envelope.ts).
Keyboard tools set it by post-processing `finishAction`'s return (`{ ...result, fidelity: "..." }")
in `src/mcp/tools/keyboard.ts` — `actionPipeline.ts`'s `ActionSpec`/`finishAction` were NOT touched
(out of scope for WP-H), so fidelity is layered on top, not threaded through the pipeline.

`deliveryFor(phase, stateChanging)` cannot express "input was posted AND retrySafe: true" — its own
doc comment states retrySafe is true exactly when phase is "not_started", never otherwise. So
`keyboard_release_all`'s honest fix (input WAS posted) makes `retrySafe: false` even though the verb
is idempotent; that combination is not expressible via the shared helper without hand-building the
Delivery object, which the brief said not to do.

`SessionOrchestrator.status()` in `src/session/orchestrator.ts` builds `SessionStatus` from an
explicit field list, not a generic spread. A brief that says "add a field only inside `#preflight()`"
but "surface it in SessionStatus" is unsatisfiable without also touching the class-field
declarations and the `status()` return object — both are one-line, non-conflicting additions next to
the existing `permissions`/`identityWarning` precedent, not touching `start()`/`reconnectBridge()`/
`#recover()`. Mirrors the existing `identityWarning` pattern for a new `keyboardLayoutWarning` field.

`main.swift` helper's `NativeInput` gained an optional `layout?()` verb (Carbon
`TISCopyCurrentKeyboardInputSource`/`TISGetInputSourceProperty`) — kept OPTIONAL on the
`NativeInput` interface (`adapter.ts`) specifically so `tests/fixtures/fakeEnv.ts`'s `FakeInput`
(used by most tool-pipeline tests, not in WP-H's allowed-edit list) does not need updating.
`tests/orchestrator.test.ts`'s own `platformFor` fake DOES implement it (that file was in scope).

This machine's real active layout is `com.apple.keylayout.Slovak` (not US/ANSI) — useful as a live
non-ANSI fixture for any future real-helper `layout` test; `helperProtocol.test.ts` intentionally
only asserts shape (non-empty id + boolean), never a specific id, since the layout is host-dependent.
