---
name: repo-rebind-edit-context
description: ONECAD-537 evidence-relative rebind context — evidenceBasis stamps, per-entry ResolveRefs split, claim settlement (root lane only), legacy stamp fenced on history hash, capability gate, real-worker U-prism twin fixture
metadata:
  type: project
---

How the persisted-id (DI-4) and projection-source rebinds decide the §7.5 edit context
(reworked 2026-11-11 on fix/hardening-runtime-refs after review; first attempt b31e2b8c
used the publishing plan's claim, which was wrong for redo/restart/reopen).

**Why:** a claim-free lane (redo, restart replay, open of a doc saved after an edit)
rebuilds post-edit geometry; a stale anchor then bound prong L's colour to prong R.

**How to apply:**

- `ElementEntry.evidence_basis: Option<EvidenceBasis>` (serde `evidenceBasis`, 64 lc hex,
  null/malformed refused by `preserved_fields`). Head basis = `executed_basis(timeline,
  target_step, snap.step_index)` = `history_prefix_hash_where` over Valid records ≤ last
  valid, computed in `finish_regen` AFTER writebacks, stored in `RebindHead.basis`.
- Code: `document_runtime/evidence_basis.rs` (basis fns, RebindHead, capture_basis,
  adopt_restamps, rebind_is_current) + `evidence_rebind.rs` (unlocked drive: two
  ResolveRefs — same-basis without field, foreign with `editedFrom: 0` — one bind).
- Every evidence writer stamps `capture_basis(snapshot)`: promote_selection, face entry,
  staging, component enrichment (`promotion.rs::rebase`), evidence translation `apply`.
- Legacy: `from_document` stamps unstamped DEFINITION evidence with the opened basis (root:
  first published head, see below); component working runtimes use `legacy_basis` fallback
  (never written, so unchanged replays stay byte-identical — `commit_component_replay`
  dirties/refuses read-only on ANY definition change).
- RE-STAMP REMOVED (2026-11-12 review): resolution never rewrites anchor/descriptor/basis;
  a foreign entry resolves under context every pass. The re-stamp's post-publish
  `persisted_generation` bump raced UI scene capture (4 native placement tests red).
- Invariant: no unlocked post-publish pass may bump `persisted_generation`; onecad-ui scene
  capture fences on it (`capture/assembly.rs::unchanged`) and fails the whole capture.
- `evidence_claims.rs::settle_claims` (ROOT lane only, `RebindScope.lane`): demote an AutoBind
  whose `element_id` is non-empty and != ref id (face held by another id), then several
  AutoBinds of one (body, TopoKey) → sole same-basis wins, else all NeedsRepair(Ambiguous).
  Component lane settles NOTHING: a synthesised NeedsRepair would drive persisted quarantine.
- `RebindFailure::{Transport, Refused}`: only NotConnected/Crashed/Timeout/Cancelled charge
  the budget; a refused request keeps the other request's answers (`unanswered`).
- Root legacy (2026-11-12 rework): `legacy_stamp: Option<EvidenceBasis>` = opened_basis at
  open (re-based after open-time parameter repairs). `after_mutation` →
  `settle_legacy_stamp_on_history_change` marks unconfirmed at the FIRST history change (so a
  save before the first head persists the marker). First head stamps only if scratch-history
  == opened AND executed basis == opened_basis(timeline) (no failed step/ceiling); else mark.
- FakeBackend `resolve_fails` = `Crashed`; seams `refuse_binds`, `refuse_edit_context`,
  `refuse_shared_keys` (worker's duplicate-TopoKey batch refusal), `bind_requests`.
  `resolve_responses` is returned verbatim for EVERY ResolveRefs: single-request passes only.
  Real worker: sole survivor with anchor 9.5 mm away AutoBinds (0.93).
- WorkerManager refuses `Some(editedFrom)` without hello `resolveRefs.editContext`.
- FakeBackend: `epoch` AtomicU64 for restart fences. Real-worker twin: U prism, shift profile −20 sketch X;
  `tests/reference_lifecycle/edit_rebind.rs`.
- C++ test trap: `json(2)` is number_INTEGER; the wire parser yields number_unsigned, so
  pass `std::uint64_t{2}` when a handler checks `is_number_unsigned()`.
- Component replay turns ANY unresolved rebind row into `UnresolvedTopology` unless a joint
  needs repair (quarantine). Every component face gets an id (`promote_anchored`), so the
  edit veto refused every definition edit of a symmetric part (prusa cold_reopen red).
  Interim fix (2026-11-12): `RebindLane::ComponentContextFree` set in `working_runtime`
  → `pending_evidence` marks nothing foreign (no editedFrom, no restamp). Stamps kept
  truthful but unread there: translation `apply`/`refresh_entry` confirm only entries whose
  basis was the prior/prefix head. Pins: `tests/rebind_component_lane.rs`,
  `tests/reference_lifecycle/component_lane.rs` (depth edit 25→26; −20 shift refuses even
  context-free because minted twins' anchors land on nothing).
- Lane crossings: component working runtime is built from `def.elements` only; root is not
  in `definitions`; `CopyRoot` copies root entries (incl. stale ones) into a definition;
  joint bindings target component ids but `edit_definition` marks them `needs_repair`.
- `legacyReferenceFace` seed dry run reuses `RebindHead.plan_edited_from` (the plan's step
  context); always-context broke the open-lane legacy chamfer repair (anchor-only edge).
- Known gap (not fixed): §7.2 op-input refs replayed claim-free after an ACCEPTED edit.
