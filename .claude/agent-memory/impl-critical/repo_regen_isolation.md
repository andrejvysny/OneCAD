---
name: repo-regen-isolation
description: OneCAD regen dependency-closure facts — why graph.downstream() is not a stable closure, what timeline_of(n) actually builds, and the two single-consumer seams around a regen publish
metadata:
  type: project
---

Facts a WP-D1-style "run the independent branch past a halt" change depends on.

**Why:** WP-D1 (isolate a failed step's blast radius) was briefed on the assumption
that `{H} ∪ graph.downstream(H)` is a safe closure and that the existing executor
fixtures pin unrelated behaviour. Neither holds; both cost a full trace to discover.

**How to apply:**

- A dependency-closure exclusion has TWO sides and they must be duals. The proof
  side (may this record run?) and the taint side (what did the excluded record
  touch?) see different populations: an EXCLUDED record only ever reaches the taint
  side, so every clause that protects a record as a consumer must have a twin that
  protects it as the excluded one. Otherwise a halt whose own effect is unbounded —
  `Operation::Opaque`, or element inputs with no body and no `outputs` — taints
  nothing, has no downstream (no `outputs` ⇒ nobody's producer ⇒ no forward edges),
  and admits everything above it. That was the WP-D1 BLOCKER; the fix is
  `body_effect_is_knowable` + a `poisoned` flag.
- The legacy bare-`edge_ids` Fillet/Chamfer (no typed `edges[].primary`) is LIVE,
  not historical: `EditSession::validate_fillet_lockstep` accepts it whenever
  `tangent_closure_version != Some(1)`. Any analysis that assumes a fillet declares
  its body is wrong for that shape.
- An optional extra pass over a published result must be adopted CONDITIONALLY.
  Substituting a second pass's `DrivenRegen` unconditionally means a worker death
  inside its window destroys the first pass's publish too — strictly worse than not
  having the feature. Gate on `Outcome::Published` and fall back.
- `crates/onecad-core/tests/support/mod.rs` `timeline_of(n)` builds `n` records from
  `extrude_record`, which sets `profile: None`, `target_body: None`,
  `boolean_mode: NewBody`. Every record therefore derives EMPTY
  `OperationInputs` — no bodies, no sketches, no elements — so in the dependency
  graph all `n` records are mutually independent. Any test on this fixture that
  asserts "the successor of a halted step stays `Dirty`" is asserting the LINEAR
  early-stop rule, not a dependency fact. `regen_executor.rs:212`, `:268` and `:663`
  are exactly that, and they collide head-on with any independent-branch execution.
- `DependencyGraph` body edges come from `Node::output_bodies` = `record.outputs`
  (`history/graph.rs:78`), and `edit/session.rs:368-372` CLEARS `outputs` for every
  executed record with no produced bodies — which includes a step that FAILED. So on
  the second regen while a mid-timeline op is still failing, the edge H→downstream
  disappears and `graph.downstream(H)` is empty. `downstream()` is evidence of a
  dependency, never proof of independence. Sketch edges are safe: `output_sketch`
  is read off the op params, not off `outputs`.
- `ModelSnapshot::step_index` has exactly ONE consumer outside core —
  `document_runtime.rs:2920`, the checkpoint-mint gate (`== head_step`). Publishing
  a snapshot whose `step_index` reaches the head opens that gate.
- `CheckpointStore::save` is a per-step insert that OVERWRITES (see the VF-M2
  comment, `document_runtime.rs:2929-2934`), so minting a bad checkpoint at a step
  destroys the good one there. A checkpoint whose stored `history_prefix_hash` does
  not equal `history_prefix_hash(&records[0..=step])` is never selected
  (`planner.rs` `choose_checkpoint`), so the damage is lost acceleration, not lost
  geometry.
- `compute_hashes` (`planner.rs:247-267`) derives `prefix_hashes` from `planned_ops`,
  not from `records`, and `expected_prefix_echo` (`executor.rs:1070-1085`) indexes it
  by EXECUTION order. A plan that omits records therefore needs no wire, SCHEMA or
  fixture change — with `start_step == 0` the base hash is the empty anchor.
- `RegenExecutor::run`'s F12 replay-from-0 retry (`executor.rs:437-490`) is the
  in-place template for any "re-plan and re-drive inside one `run()`" change:
  `AttemptOutcome` variant, re-plan via `RegenPlanner::without_checkpoint`, one
  `DiscardOnDrop` guard covering both attempts. Its `.with_edited_from(filter(> 0))`
  is coupled to `as_checkpoint_fallback_replay()`, not general — do not copy it
  without that flag.
- A multi-pass regen belongs in `DocumentRuntime`, never in `RegenExecutor`:
  `RegenSession` carries bodies/timeline/repair/elements and NO graph, so the
  executor structurally cannot compute a dependency closure, and Invariant 6
  (publish `≤ m−1`, one plan halts whole) is its pinned contract. The exclusion set
  must reach `RegenPlanner` as DATA — `begin_regen` hands the planner an EMPTY
  `DependencyGraph`; the real one is `self.session.graph()`.
- The frontend's per-feature status comes from `self.regen.timeline` via
  `feature_dto`, NOT from `ModelSnapshot::step_states`. Anything that must change
  what the user sees has to land on the scratch session before `commit_snapshot`
  swaps it in. `dto::feature_status` folds `Dirty` and `Suppressed` into one status,
  so "did not run" must be `Dirty` — `Suppressed` reads as user intent.
- `RegenSession.timeline` (the substituted mirror) is a full `records().to_vec()`
  clone, so `outputs` and `suppressed` survive substitution and are safe to read
  alongside `session.graph()`.
- `sync_record_outputs` (`edit/session.rs`) only writes records in the pass's
  `executed` set, so a record excluded from a plan keeps its last-known `outputs` —
  which keeps its graph edges alive for the next regen. The safe direction.
- `document_runtime/tests.rs` `FakeBackend` reports `StoppedReason::Completed` for
  every plan unless `with_failing_steps(&[..])` is used; `plan_calls()` is the
  observable for "how many `ExecutePlan`s did this cost".
