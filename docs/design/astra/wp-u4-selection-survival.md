# wp-u4-selection-survival
date: 2026-09-11
mode: break
model: gpt-6-astra
effort: xhigh
access: grounded: src/viewport/mesh/rebindPick.ts, src/viewport/mesh/meshSync.ts, src/ipc/promote.ts, src/viewport/ViewportRoot.tsx, src/viewport/engine/Picker.ts, src/viewport/engine/HighlightLayer.ts, src/tools/modelTools/ModelToolController.ts, src-tauri/src/document_runtime.rs, worker/src/session/ElementIdentity.cpp, worker/src/tess/Tessellate.cpp, src-tauri/tests/selection_promote.rs, protocol/SCHEMA.md (§7.5, §10). Transcript also shows one read outside the allowlist: worker/tests/test_wp6_ladder.cpp (a ladder unit test; no expected values, no contamination of a break — recorded, not re-run).
packet: sha256 04fb9a7c75a3
calls: 1 of ~3 for WP-U4; session 01a090eb-2976-73b3-948a-f54534bd970b recorded for followup
verdict: defective — C/E treat a reused snapshot ordinal as identity confirmation, and asynchronous confirmation lacks sufficient snapshot and selection-instance fencing.
verified by: Fable read each cited line (rebindPick.ts:44 `ordinalForRef` topoKey-first; :66 missing-index rule; :88 `confirmRef` id-keyed guard; ViewportRoot.tsx:136 id-keyed writeback; ElementIdentity.cpp:62 `resolve_pick` returns on an existing TopoKey before any anchor check; Tessellate.cpp:198 zero-triangle labels). Ordinal-inclusion computation for the chamfer case ({1..12} ⊂ {1..15}, zero ElementId labels) matches the selection_promote.rs stage counts. F5's SCHEMA citations (§10 :5012 teleport residual, :5065 margin exception) confirmed present.
rejected: F5 (backend ladder guarantees weaker than the packet's invariant wording) — true as a reading of §10, but the reconcile path uses QueryElement by elementId (a partition lookup, never the scoring ladder), so it does not bear on this package; recorded as a follow-up against the §10 ladder, not a fix here. "Hole seat clear is a UX choice" — accepted as analysis; decision taken: remove the hole-specific clear (the seat face survives and, once F1–F3 are fixed, reconciles correctly, which is the reviewer's four-holes flow).

## 1. Verdict

**Defective:** C/E treat a reused snapshot ordinal as identity confirmation, and asynchronous confirmation lacks sufficient snapshot and selection-instance fencing.

## 2. Findings, ranked by severity

**F1 — blocker; proved + numeric. “The current mesh names the same element.”**

`ordinalForRef` prefers `topoKey` over `elementId`. A valid nine-face table can contain `f:7` at ordinal 6 naming another face, while the selected element is labelled `el_x` at ordinal 8. Executing C with `{topoKey:"f:7", elementId:el_x}` produced **highlight ordinal 6, zero confirmation queries**. The correct ordinal was 8. [C/E lookup](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/mesh/rebindPick.ts:44)

The consumed-edge counterexample follows directly from the supplied measurements: after the 3 mm chamfer, `{1,…,12} ⊂ {1,…,15}` and there are zero ElementId labels. Therefore **every old `e:k`, including the consumed edge’s ordinal, still names something**. Undo the chamfer, select its restored input edge, then redo: the inspected redo path does not execute D. C accepts the reused ordinal. [Redo](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/tools/modelTools/ModelToolController.ts:9433)

For **face** ordinal reuse, the exact OCCT edit/key pair is **not established** by the packet. The flange test rediscovers its host every generation; it does not compare the old ordinal’s physical owner. If adding hole k moves that host from `f:p` to `f:q`, `p ≠ q`, and `f:p` remains unsubstituted, that hole edit triggers the defect. Missing evidence: the before/after host keys and ownership of old `f:p`.

**Neither safeguard repairs a current-head promotion of the wrong ordinal:**

- B rejects `S_old`; it accepts `S_head`. The frontend does not retain the pick’s snapshot in the inspected ref construction.
- `resolve_pick` returns immediately for an existing TopoKey; **the anchor is never checked**. Equal requested/returned strings therefore pass even when their physical meaning changed. [Worker resolution](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/worker/src/session/ElementIdentity.cpp:62)
- Edge authoring explicitly prefers `pick.topoKey` and omits its known `elementId`. Its adapter/backend handling is outside the allowlist, so final wrong-op acceptance remains conditional. [Edge preparation](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/tools/modelTools/ModelToolController.ts:3283)

**Smallest closure:** prefer persistent identity; permit raw TopoKey lookup only with matching publication provenance. Across regeneration, drop unpromoted refs; confirm unresolved persistent refs against that publication. Author tools from confirmed ElementIds. Merely reversing lookup priority leaves unpromoted ordinal reuse open.

---

**F2 — blocker; proved acceptance gap, inferred production scheduling. “The answer confirms this displayed mesh.”**

Schedule: mesh S=2 remains displayed; backend reaches S=3; `elementInfo` answers `f:9` from S=3 before the S=3 mesh arrives. `getEntry(bodyId) === next` still passes. C installs the S=3 ordinal into an S=2 ref without checking its membership, body, or kind. If S=2’s `f:9` is another face, E draws it; if absent, selection remains invisible. The latter was reproduced in the in-memory harness. [Confirmation](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/mesh/rebindPick.ts:88)

`QueryElement(elementId)` returns the partition entry’s body and kind; C ignores both. A binding transferred to another body would consequently be interpreted inside the original body. Actual transfer reachability is unverified. [Query result](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/worker/src/session/ElementIdentity.cpp:206)

**Smallest closure:** pass the displayed publication’s snapshot through the existing QueryElement snapshot field; validate snapshot, normalized body, kind, and drawable current-table label before accepting. Retain the mesh-instance guard.

Related inherited producer gap: `loadBody` checks `loadSeq` **before** awaiting color resolution, then swaps without rechecking. Recheck after that await to prevent an older load installing after a newer one. [Mesh ingestion](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/mesh/meshSync.ts:360)

---

**F3 — high; proved. “Deselection makes old replies harmless.”**

The guard identifies a selection by reusable `id`, not selection instance:

1. Old ref: `{id:"body#f:7", topoKey:"f:12", elementId:el_x}`.
2. Current mesh contains a different `f:7`; query for `el_x` waits.
3. Deselect; pick current `f:7`, creating a fresh ref with the same `id`.
4. Old query returns absent and **deletes the fresh selection**.

Reproduced. `promotePick` has the same id-only writeback issue and lacks C’s mesh guard. [Promotion writeback](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/ViewportRoot.tsx:136)

**Smallest closure:** capture a selection-instance token plus publication token; compare both before applying replies. Keep `EntityRef.id` unchanged.

**Checks that passed:** a distinct later mesh suppresses an older confirmation; simple deselection never re-adds the ref. Undo/redo pairs receive that protection when they install distinct entries. Entry reuse cannot be established without the excluded registry implementation.

---

**F4 — high; proved. “No evidence means selection may remain.”**

A missing edge index keeps the edge selected, performs zero queries, and cannot draw an edge highlight. That contradicts the stated local drawable-or-gone invariant. Pending confirmations likewise remain in ordinary selection while drawing nothing; positive answers are not checked for drawability. [Missing-index rule](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/mesh/rebindPick.ts:66), [edge drawing](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/engine/HighlightLayer.ts:214)

**Smallest closure:** remove unresolved refs from active, authorable selection. If preserving pending intent, hold it separately until confirmed and drawable; otherwise drop it.

**E’s pre-answer window:** when an index exists and C actually schedules confirmation, both lookups have already missed, so E returns `-1`. There is **no independent wrong-highlight window in that branch**. A stale TopoKey that draws incorrectly bypasses confirmation entirely—F1.

---

**F5 — high; source-cited assurance gap. “Backend confirmation guarantees physical identity and the stated scoring gate.”**

The inspected §10 explicitly documents a silent wrong bind when an exact congruent twin moves onto a stale anchor. It also permits an anchor-decisive exception below the stated `0.10` margin. Thus the packet’s unconditional backend guarantee is stronger than its cited specification. The actual ladder implementation is outside scope. [Teleport residual](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/protocol/SCHEMA.md:5012), [margin exception](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/protocol/SCHEMA.md:5065)

For the requested symmetric case, `0.91 − 0.91 = 0 < 0.10`: without unique lineage or valid disambiguating evidence, it must yield `NeedsRepair`. C’s raw-label acceptance does not evaluate that distinction.

**Smallest closure:** backend confirmation must exclude consumed identities and require lineage or the requested confidence/margin guarantees. Where lineage cannot distinguish a replacement twin, refuse. Frontend geometry cannot close this gap.

**Remaining required cases:**

| Case | Result and assessment |
|---|---|
| `f:7 → el_x`, ref lacks ElementId | **Proved:** dropped if `f:7` disappears; wrongly kept if another face inherits `f:7`—F1. Preserve only using the matching promotion acknowledgement; table position alone cannot recover identity. |
| `el_x → f:9`, binding dropped | **Proved conditionally on partition absence:** `query_by_id` returns absent; C drops the live geometric face. It performs no descriptor search. This is permitted conservative loss, not permission to infer `f:9`. |
| Hole seat | **Proved:** D intentionally clears a surviving face; its comment acknowledges survival. “Only when confirmed” permits this UX choice. Without D, an ElementId-bearing seat can remain correctly selected through its mesh label, subject to F1. Preserve it by removing the hole-specific clear; otherwise distinguish “completed input” from “consumed topology.” [Hole completion](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/tools/modelTools/ModelToolController.ts:5560) |
| A’s `kind: ""` | **Source-verified:** inspected `promoteOne` consumers use the ElementId and supply their own kind; no demonstrated kind failure. A returns a resolved Promise without promotion IO. It establishes “already minted,” not current liveness. Narrowing its return type would remove misleading metadata. [A](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/ipc/promote.ts:58) |

## 3. Missing degeneracy classes, with detection predicates

| Class | Detection predicate |
|---|---|
| Removed body | `ref.bodyId ∉ publishedBodies`. `dropBody` removes mesh/highlights but does not itself clear selection. Any external cleanup is unverified. [Removal](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src/viewport/mesh/meshSync.ts:438) |
| Split/merge or owner transfer | Confirmed owner differs from `ref.bodyId`, or lineage has multiple successors. Drop or require an explicit backend ownership/lineage decision. |
| Mirrored/patterned copies | Repeated TopoKeys across different bodies are harmless; repeated ElementIds across different owners require investigation. Same-facing congruent candidates with unresolved lineage require ambiguity handling. |
| Same-snapshot finer LOD / namespace aliases | `S_next = S_prev` but mesh instance or labels differ. Do not treat tessellation replacement as identity loss. Also detect distinct selection ids resolving to the same `(bodyId, elementId)`. |
| Named but untessellated element | Face triangle count `= 0`, or edge drawable segment count `= 0`. Label presence does not prove drawability. The tessellator explicitly emits such labels. [Zero-triangle face](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/worker/src/tess/Tessellate.cpp:198) |
| Small edit changes winner | Same intended lineage, but selected successor changes under a small parameter perturbation; or `consumed(element) && bindingPresent(element)`. Test both scale endpoints and symmetric same-facing candidates. |

## 4. Tests needed

| Target / layer | Trigger → required assertion |
|---|---|
| **F1 — Vitest C/E** | Nine-face table: unrelated `f:7` at ordinal 6, selected `el_x` at ordinal 8 → draw 8. Across changed snapshots, raw-only reused ordinal must be dropped. |
| **F1 — Rust real worker + e2e** | Existing 3 mm chamfer: undo → pick restored consumed edge → redo → selection gone, consumed ElementId absent, next operation cannot target the replacement ordinal. Compare physical evidence, not merely echoed keys. Add before/after flange host ownership to establish the missing exact face case. |
| **F1 — Rust promotion** | Promote old key with old snapshot → refusal. Send that key with head snapshot and a deliberately stale anchor → demonstrate that ordinal resolution ignores the anchor; frontend must never issue this request as continuation of the old pick. |
| **F2 — Vitest C / mesh ingestion** | Delay S=3 mesh while returning an S=3 query to displayed S=2; return wrong-body/wrong-kind or absent-table keys → none accepted. Delay older color resolution past newer load completion → older mesh never installs. |
| **F3 — Vitest / e2e** | Rewritten old ref, deselect, fresh same-id pick, old absent/positive reply → fresh selection untouched. Repeat for promotion replies, two swaps, and undo/redo pairs. |
| **F4 — Vitest C/E** | Missing edge table, unresolved query, zero drawable ranges → no active invisible selection and no authoring from pending intent. |
| **F5 — Rust real worker** | Same-facing symmetric candidates with equal evidence → `NeedsRepair`; consumed element → absent; move a congruent twin onto the stale anchor → no silent replacement. Perturb dimensions at 0.1 mm and 10 m scales and assert lineage identity or repair. |
| **Flips / A / D — Vitest** | Forward flip with/without acknowledgement; reverse flip with absent/present binding; valid `el_…` → zero promotion calls; successful hole clears its seat only under the chosen UX policy; failure preserves selection. |
| **Degeneracies — e2e / Vitest** | Delete/split/merge selected body, copy/mirror, same-snapshot LOD reload, alias re-pick → assert valid ownership, no stale resurrection, correct toggle semantics, and drawable-or-gone selection. |

## 5. Verified versus inferred

**Verified**

- Executed C’s actual source in memory with stubbed registry/store/client: reproduced ordinal preference, both flip outcomes, invisible retention, and same-id reply corruption; confirmed later-swap and simple-deselection guards.
- Inspected allowlisted implementation, authoring paths, worker queries, tessellation labels, and test assertions.
- Computed ordinal inclusion and symmetric margin above.
- Real-worker fixtures use **40×20×12 mm stock** and **Ø50/Ø24/Ø8 flange geometry**, not the packet’s larger examples. [Fixture dimensions](/Users/andrejvysny/workspace/CAD/OneCAD-Tauri/src-tauri/tests/selection_promote.rs:261)

**Inferred / unverified**

- Exact face ordinal reassignment under a particular flange edit.
- Production timing of the head/mesh race, ownership transfer, and any external selection cleanup.
- Excluded IPC adapter handling of snapshot defaults and ElementId-labelled edge preparation.
- Actual ladder enforcement of the stronger requested invariant.
- Reported worker/full-suite passes: supplied evidence, **not rerun**. No repository files changed.

## 6. If nothing qualifies

Not applicable: F1–F4 identify concrete implementation failures.