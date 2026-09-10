//! WP-D1 blast-radius isolation: which records may still execute past a halted step.
//!
//! One broken feature must not delete or block bodies **outside its dependency
//! closure**. When a plan stops early at record `H` (`OpFailed` / `NeedsRepair`),
//! the caller issues one further from-0 regen that excludes `H` and everything
//! that cannot be *proved* independent of it, so an unrelated body — an imported
//! vendor component, the other half of a fit-check — stays on screen.
//!
//! [`isolation_scope`] computes that exclusion set. It is a pure function of
//! (records, graph, halt, target) and lives here rather than in the planner
//! because the planner is handed an EMPTY [`DependencyGraph`] by construction
//! (`RegenPlanner::plan`'s `let _ = graph`; the app hands it
//! `DependencyGraph::new()` because a linear timeline needs no ordering help).
//! The real graph lives on the document session, so the exclusion set must reach
//! the planner as DATA.
//!
//! ## Why "not in `downstream(H)`" is not a proof of independence
//!
//! The exclusion set is a **positive proof of independence, never an absence of
//! edges**. Several independent holes make a missing edge meaningless — see
//! [`independence_is_provable`], which is the single place all of them are
//! enumerated. Over-exclusion costs one un-regenerated feature the user can see
//! and retry; under-exclusion re-binds a topological reference against geometry
//! the document does not describe, which is the H5-B defect this whole migration
//! exists to kill.
//!
//! The proof has a **dual**: an excluded record never reaches
//! [`independence_is_provable`] at all, only [`taint_from`], so an excluded record
//! whose own effect cannot be bounded must poison the rest of the pass rather than
//! contribute an empty taint set. [`body_effect_is_knowable`] is that rule.

use std::collections::{BTreeSet, HashSet};

use crate::document::record::{Operation, OperationRecord};
use crate::history::DependencyGraph;
use crate::ids::{BodyId, RecordId};

/// What an isolation pass may and may not execute (see [`isolation_scope`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IsolationScope {
    /// Records the isolation plan must skip: the halted record plus every record
    /// above it whose independence is not provable.
    pub excluded: BTreeSet<RecordId>,
    /// The timeline steps of [`excluded`](Self::excluded), ascending, halt first.
    /// These are published `Dirty` — never `Suppressed`, which is user state.
    pub excluded_steps: Vec<usize>,
    /// The steps above the halt the isolation plan WILL execute. **Empty ⇒ do not
    /// run the pass**: it would reproduce the halted result exactly, at the cost
    /// of a full extra replay.
    pub independent_steps: Vec<usize>,
}

impl IsolationScope {
    /// Whether an isolation pass is worth running at all.
    #[must_use]
    pub fn is_worth_running(&self) -> bool {
        !self.independent_steps.is_empty()
    }
}

/// The records an isolation pass must skip after a halt at `halted_step`, and the
/// ones it may run, over the timeline slice `[0, target_step]`.
///
/// Seeded with `{H} ∪ graph.downstream(H)` and then grown forward: each later
/// record is admitted only if [`independence_is_provable`] says so, and every
/// record that is not admitted **taints the bodies it touches** so the records
/// after it inherit the exclusion. Suppressed records are ignored — they never
/// execute, so they are neither excluded nor independent.
///
/// Records *below* the halt are never considered: they executed successfully in
/// the halted pass and must execute again in the isolation pass.
#[must_use]
pub fn isolation_scope(
    records: &[OperationRecord],
    graph: &DependencyGraph,
    halted_step: usize,
    target_step: usize,
) -> IsolationScope {
    let Some(halted) = records.get(halted_step) else {
        return IsolationScope::default();
    };
    let mut scope = IsolationScope {
        excluded: graph.downstream(halted.record_id),
        excluded_steps: vec![halted_step],
        independent_steps: Vec::new(),
    };
    scope.excluded.insert(halted.record_id);

    // The halt itself taints before anything downstream of it is judged — and if
    // its own effect cannot be bounded, nothing above it can be proved at all.
    let mut tainted: HashSet<BodyId> = HashSet::new();
    taint_from(halted, &mut tainted);
    let mut poisoned = !body_effect_is_knowable(halted);

    let last = target_step.min(records.len().saturating_sub(1));
    for step in (halted_step + 1)..=last {
        let Some(record) = records.get(step) else {
            continue;
        };
        if record.suppressed {
            continue; // never planned either way.
        }
        // Once poisoned, stop attempting proofs: an unbounded effect below makes
        // every later admission a guess.
        if !poisoned && independence_is_provable(record, graph, &scope.excluded, &tainted) {
            scope.independent_steps.push(step);
        } else {
            scope.excluded.insert(record.record_id);
            scope.excluded_steps.push(step);
            taint_from(record, &mut tainted);
            poisoned = poisoned || !body_effect_is_knowable(record);
        }
    }
    scope
}

/// Whether `record` can be **proved** independent of everything in `excluded`.
///
/// Every clause below exists because some earlier signal is untrustworthy in a
/// way that fails OPEN. A `false` here always means "cannot prove", never "is
/// dependent" — the conservative reading is the correct one.
///
/// 1. **[`Operation::Opaque`] can never be proved.** `derive_inputs` returns an
///    empty [`OperationInputs`](crate::document::record::OperationInputs) for a
///    frozen node *by construction*, so an opaque record looks input-free and
///    therefore independent of everything. It is not; it is unknown.
/// 2. **A record whose upstream closure meets `excluded`.** This is the ordinary
///    graph answer, and it subsumes `downstream(H)` because `excluded` contains
///    `H`. It also closes chains that run *through* a record excluded for one of
///    the other reasons here, which `downstream(H)` alone would not see.
/// 3. **An input with no producer at all.** A body or sketch nobody in the graph
///    produces is either base geometry or — the case that matters — output of a
///    record whose `outputs` were cleared. Either way the edge that would have
///    tied this record to its real producer does not exist.
/// 4. **Element-only inputs** (`history/graph.rs` divergence 2). A v2
///    [`ElementId`](crate::ids::ElementId) is opaque and does **not** embed a
///    `BodyId` — that linkage lives in the worker's ElementMap partition, not in
///    pure core — so element inputs form no edge here. A Fillet carrying bare
///    `edge_ids` and no typed `edges[].primary` therefore declares elements, zero
///    bodies, and no edge, and reads as independent of the step that built the
///    body it cuts. Elements are only safe when the record ALSO declares a body
///    input, which the clauses above have already proved independent.
/// 5. **A tainted body.** `DependencyGraph` builds its producer index from
///    `OperationRecord::outputs` (`graph.rs`, `Node::output_bodies`), and
///    `DocumentSession::sync_record_outputs` CLEARS `outputs` for an executed
///    record that produced nothing — which is exactly what a failed step is. So
///    on the second regen while `H` is still failing, the edge `H → consumer`
///    has vanished and clause 2 sees nothing. Body-level taint is the
///    outputs-independent replacement: an op that *consumes* a body may modify
///    it, so every excluded record taints `outputs ∪ derive_inputs().bodies`
///    (see [`taint_from`]) and a consumer of a tainted body is refused.
///
/// Sketches need no taint clause: `sketch_producers` is derived from the op's own
/// params (`Node::output_sketch`), never from `outputs`, so it cannot go stale
/// the way `body_producers` does.
///
/// **`derive_inputs` matches all 19 [`KnownOperation`] variants with NO catch-all
/// arm** (`document/record.rs`), so adding an operation is a compile error there
/// rather than a silent fail-open here. Do not add a `_ => {}` to that match.
///
/// [`KnownOperation`]: crate::document::record::KnownOperation
fn independence_is_provable(
    record: &OperationRecord,
    graph: &DependencyGraph,
    excluded: &BTreeSet<RecordId>,
    tainted_bodies: &HashSet<BodyId>,
) -> bool {
    // (1) a frozen node exposes no typed deps — unknown, not independent.
    if matches!(record.op, Operation::Opaque(_)) {
        return false;
    }
    // (2) the graph's own answer, transitively.
    if graph
        .upstream(record.record_id)
        .iter()
        .any(|up| excluded.contains(up))
    {
        return false;
    }
    let inputs = record.op.derive_inputs();
    for body in &inputs.bodies {
        // (5) something excluded may have modified this body.
        if tainted_bodies.contains(body) {
            return false;
        }
        // (3) no producer, or a producer that is itself excluded.
        match graph.body_producer(*body) {
            None => return false,
            Some(producer) if excluded.contains(&producer) => return false,
            Some(_) => {}
        }
    }
    for sketch in &inputs.sketches {
        match graph.sketch_producer(*sketch) {
            None => return false,
            Some(producer) if excluded.contains(&producer) => return false,
            Some(_) => {}
        }
    }
    // (4) element inputs prove nothing on their own.
    if !inputs.elements.is_empty() && inputs.bodies.is_empty() {
        return false;
    }
    true
}

/// The bodies an excluded record may have changed: what it produced, plus what it
/// consumed (an op that consumes a body may modify it in place — a fillet, a
/// shell, a boolean all do).
///
/// This is only half the story. An empty result here means "touches no body" ONLY
/// if [`body_effect_is_knowable`] agrees — see there.
fn taint_from(record: &OperationRecord, tainted: &mut HashSet<BodyId>) {
    tainted.extend(record.outputs.iter().copied());
    tainted.extend(record.op.derive_inputs().bodies);
}

/// Whether an excluded record's effect on the body registry can be **bounded** at
/// all — the dual of [`independence_is_provable`], and the reason
/// [`taint_from`] returning nothing is not the same as "changed nothing".
///
/// An excluded record only ever reaches [`taint_from`], never the proof side, so
/// without this the clauses that protect a record as a CONSUMER would not protect
/// it as the EXCLUDED one. `false` ⇒ nothing above this record is provably
/// independent and [`isolation_scope`] stops admitting.
///
/// Unknown, and why:
///
/// * **[`Operation::Opaque`]** — `derive_inputs` returns nothing for a frozen node
///   by construction, so it looks body-free however much geometry it moves.
/// * **Element inputs with no body and no `outputs`** — it names elements of a
///   body it does not identify, and `outputs` is not a fallback: a FAILED step's
///   `outputs` are CLEARED at commit by
///   `DocumentSession::sync_record_outputs` (`edit/session.rs` —
///   `outputs.get(&id).cloned().unwrap_or_default()` for every executed record).
///   The first halt still has its stale `outputs` and taints correctly; that same
///   commit wipes them, so from the next edit onward there is nothing left to
///   taint with, nothing downstream (no `outputs` ⇒ this record is nobody's
///   producer ⇒ no forward edges), and a later Shell on the very body this record
///   cut would read as independent. That is the H5-B silent wrong bind — strictly
///   worse than the truncation WP-D1 removes. The shape is live, not theoretical:
///   `EditSession::validate_fillet_lockstep` accepts a Fillet/Chamfer with
///   `edges: []` and bare `edge_ids` whenever `tangent_closure_version != Some(1)`.
///
/// A record with NO element inputs and no body inputs is genuinely body-free — a
/// plain `Sketch` op, say. Its sketch edges come from `Node::output_sketch`,
/// derived from the op's own params and never from `outputs`, so they cannot go
/// stale the way `body_producers` does.
fn body_effect_is_knowable(record: &OperationRecord) -> bool {
    if matches!(record.op, Operation::Opaque(_)) {
        return false;
    }
    let inputs = record.op.derive_inputs();
    !(!inputs.elements.is_empty() && inputs.bodies.is_empty() && record.outputs.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, OpaqueOperation,
        PlaneKind, ShellParams, SketchOpParams, SketchPlaneRef,
    };
    use crate::document::refs::{ElementKind, ElementRef, PrimaryRef};
    use crate::document::variables::Scalar;
    use crate::ids::ElementId;
    use crate::math::Vec3;
    use uuid::Uuid;

    /// The frozen `{opType, …}` body an `Operation::Opaque` carries.
    fn opaque_raw() -> serde_json::Map<String, serde_json::Value> {
        let mut raw = serde_json::Map::new();
        raw.insert("opType".into(), serde_json::Value::from("Future"));
        raw
    }

    fn rid(n: u128) -> RecordId {
        RecordId(Uuid::from_u128(n))
    }

    fn bid(n: u128) -> BodyId {
        BodyId(Uuid::from_u128(n))
    }

    /// A standalone NewBody extrude — no inputs of any kind.
    fn extrude(seed: u128) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: None,
            distance: Scalar::new(5.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }));
        OperationRecord::new(rid(seed), 0, "Extrude", op)
    }

    /// An extrude that CLAIMS to have produced `body` (what a successful regen
    /// writes back via `sync_record_outputs`).
    fn extrude_producing(seed: u128, body: BodyId) -> OperationRecord {
        let mut rec = extrude(seed);
        rec.outputs = vec![body];
        rec
    }

    /// A plain world-plane `Sketch` op — genuinely body-free (no bodies, no
    /// elements), so excluding it must not poison the pass.
    fn sketch_record(seed: u128, sketch: u128) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: crate::ids::SketchId(Uuid::from_u128(sketch)),
            plane: SketchPlaneRef {
                kind: PlaneKind::Xy,
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                x_axis: Vec3::new_unchecked(1.0, 0.0, 0.0),
                y_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
                extra: Default::default(),
            },
            entities: Vec::new(),
            constraints: Vec::new(),
            host_face: None,
            extra: Default::default(),
        }));
        OperationRecord::new(rid(seed), 0, "Sketch", op)
    }

    /// A fillet addressed the legacy way: element ids only, no typed body.
    fn bare_fillet(seed: u128) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(1.0),
            edge_ids: vec![ElementId::new("el_bare")],
            edges: Vec::new(),
            chain_tangent_edges: true,
            tangent_closure_version: None,
            extra: Default::default(),
        }));
        OperationRecord::new(rid(seed), 0, "Fillet", op)
    }

    /// A shell on `body` — declares the body AND face elements, the shape that
    /// slips through every clause except the taint one.
    fn shell_on(seed: u128, body: BodyId) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Shell(ShellParams {
            target_body: Some(body),
            thickness: Scalar::new(2.0),
            open_faces: Vec::new(),
            faces: vec![ElementRef {
                primary: Some(PrimaryRef {
                    body,
                    element: ElementId::new("el_face"),
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                extra: Default::default(),
            }],
            extra: Default::default(),
        }));
        OperationRecord::new(rid(seed), 0, "Shell", op)
    }

    fn graph_of(records: &[OperationRecord]) -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.rebuild_from_records(records);
        g
    }

    #[test]
    fn standalone_new_bodies_are_provably_independent() {
        let records = vec![extrude(0x10), extrude(0x11), extrude(0x12)];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.independent_steps, vec![2]);
        assert_eq!(scope.excluded_steps, vec![1]);
        assert!(scope.is_worth_running());
    }

    #[test]
    fn a_graph_dependent_successor_is_excluded() {
        // 0 makes B, 1 (the halt) shells B, 2 shells B again ⇒ 2 is downstream.
        let b = bid(0xB0);
        let records = vec![
            extrude_producing(0x10, b),
            {
                let mut r = shell_on(0x11, b);
                r.outputs = vec![b];
                r
            },
            shell_on(0x12, b),
        ];
        let graph = graph_of(&records);
        assert!(graph.downstream(rid(0x11)).contains(&rid(0x12)));
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.excluded_steps, vec![1, 2]);
        assert!(!scope.is_worth_running());
    }

    // ── the four not-provable shapes ─────────────────────────────────────────

    #[test]
    fn shape_1_an_opaque_record_is_never_provable() {
        let opaque = OperationRecord::new(
            rid(0x12),
            0,
            "Future",
            Operation::Opaque(OpaqueOperation { raw: opaque_raw() }),
        );
        let records = vec![extrude(0x10), extrude(0x11), opaque];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.excluded_steps, vec![1, 2], "opaque cannot be proved");
        assert!(scope.independent_steps.is_empty());
    }

    #[test]
    fn shape_2_element_only_inputs_are_never_provable() {
        // `history/graph.rs` divergence 2: the bare fillet forms NO edge, so the
        // graph reads it as independent. The predicate must refuse it anyway.
        let records = vec![extrude(0x10), extrude(0x11), bare_fillet(0x12)];
        let graph = graph_of(&records);
        assert!(
            !graph.downstream(rid(0x11)).contains(&rid(0x12)),
            "the graph really is blind to this one"
        );
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.excluded_steps, vec![1, 2]);
        assert!(scope.independent_steps.is_empty());
    }

    #[test]
    fn shape_3_an_input_with_no_producer_is_never_provable() {
        // Step 2 shells a body NOBODY in the graph produces — the shape a halted
        // record that created a body leaves behind once its outputs are gone.
        let orphan = bid(0xDEAD);
        let records = vec![extrude(0x10), extrude(0x11), shell_on(0x12, orphan)];
        let graph = graph_of(&records);
        assert!(graph.body_producer(orphan).is_none());
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.excluded_steps, vec![1, 2]);
        assert!(scope.independent_steps.is_empty());
    }

    #[test]
    fn shape_4_a_body_tainted_by_the_halt_is_never_provable() {
        // The reachable fail-open: step 0 makes B, step 1 (the halt) FAILED so its
        // `outputs` were cleared, step 2 shells B. No edge 1 → 2 survives, and
        // clauses 1-3 all pass — only taint refuses it.
        let b = bid(0xB0);
        let mut halted = shell_on(0x11, b);
        halted.outputs = Vec::new(); // cleared by `sync_record_outputs` on failure.
        let records = vec![extrude_producing(0x10, b), halted, shell_on(0x12, b)];
        let graph = graph_of(&records);
        assert!(
            !graph.downstream(rid(0x11)).contains(&rid(0x12)),
            "with outputs cleared the graph edge really has vanished"
        );
        assert_eq!(graph.body_producer(b), Some(rid(0x10)), "clause 3 passes");
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(
            scope.excluded_steps,
            vec![1, 2],
            "the halt consumed B, so B is tainted and its later consumer is refused"
        );
        assert!(scope.independent_steps.is_empty());
    }

    // ── scope mechanics ──────────────────────────────────────────────────────

    /// 0 makes B · 1 halt (standalone, taints nothing) · 2 opaque · 3 shells B.
    ///
    /// Step 3 must be refused whether or not the opaque record at 2 happens to
    /// carry `outputs`: WITH them the taint set names B, WITHOUT them the record's
    /// effect is simply unknown and the pass is poisoned. Both paths, because the
    /// with-`outputs` case alone would accept the buggy answer for the far more
    /// common empty one (a failed step's `outputs` are cleared at commit).
    #[test]
    fn an_excluded_record_of_unknown_effect_refuses_every_later_consumer() {
        let b = bid(0xB0);
        for outputs in [vec![b], Vec::new()] {
            let mut opaque = OperationRecord::new(
                rid(0x12),
                0,
                "Future",
                Operation::Opaque(OpaqueOperation { raw: opaque_raw() }),
            );
            opaque.outputs = outputs.clone();
            let records = vec![
                extrude_producing(0x10, b),
                extrude(0x11),
                opaque,
                shell_on(0x13, b),
            ];
            let graph = graph_of(&records);
            let scope = isolation_scope(&records, &graph, 1, 3);
            assert_eq!(
                scope.excluded_steps,
                vec![1, 2, 3],
                "opaque outputs = {outputs:?}"
            );
            assert!(scope.independent_steps.is_empty(), "outputs = {outputs:?}");
        }
    }

    /// THE BLOCKER (adversarial review, 2026-09-10). A legacy Fillet addressed by
    /// bare `edge_ids` — which `EditSession::validate_fillet_lockstep` accepts
    /// whenever `tangent_closure_version != Some(1)` — declares elements, no body.
    /// Once its `outputs` are cleared by the commit that recorded its failure it
    /// taints nothing, produces nothing, and so has no downstream: every clause on
    /// the PROOF side is blind because the halt never reaches the proof side.
    ///
    /// Without [`body_effect_is_knowable`] the Shell at step 2 is admitted and
    /// re-resolves its face refs against the UN-FILLETED body — the H5-B silent
    /// wrong bind, strictly worse than the truncation WP-D1 removes.
    #[test]
    fn a_bare_edge_fillet_halt_with_cleared_outputs_poisons_the_whole_pass() {
        let b = bid(0xB0);
        let halted = bare_fillet(0x11); // outputs already empty, as after the clear
        assert!(halted.outputs.is_empty());
        let records = vec![extrude_producing(0x10, b), halted, shell_on(0x12, b)];
        let graph = graph_of(&records);
        // Every signal the proof side could have used really is absent.
        assert!(graph.downstream(rid(0x11)).is_empty(), "no forward edges");
        assert_eq!(
            graph.body_producer(b),
            Some(rid(0x10)),
            "producer is step 0"
        );

        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(scope.excluded_steps, vec![1, 2]);
        assert!(
            scope.independent_steps.is_empty(),
            "a halt whose body effect cannot be bounded admits NOTHING above it"
        );
        assert!(
            !scope.is_worth_running(),
            "so no isolation pass runs at all"
        );
    }

    /// The poison is not all-or-nothing: records admitted BELOW the unbounded one
    /// keep their proof (nothing can depend on a later record), only records above
    /// it are refused.
    #[test]
    fn poisoning_refuses_only_the_records_above_it() {
        let b = bid(0xB0);
        let records = vec![
            extrude(0x10),              // 0
            extrude(0x11),              // 1 — the halt, standalone, bounded
            extrude(0x12),              // 2 — provably independent
            bare_fillet(0x13),          // 3 — excluded AND unbounded
            extrude(0x14),              // 4 — would be provable, but sits above 3
            extrude_producing(0x15, b), // 5 — likewise
        ];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 1, 5);
        assert_eq!(scope.independent_steps, vec![2]);
        assert_eq!(scope.excluded_steps, vec![1, 3, 4, 5]);
        assert!(scope.is_worth_running());
    }

    /// A body-free record is genuinely body-free: a plain `Sketch` op declares no
    /// bodies AND no elements, so excluding it does not poison the pass.
    #[test]
    fn a_body_free_record_does_not_poison() {
        let records = vec![
            extrude(0x10),
            extrude(0x11),
            sketch_record(0x12, 0x5C),
            extrude(0x13),
        ];
        let graph = graph_of(&records);
        // Step 2's sketch is unused, so it is admitted, not excluded — the point is
        // that the halt at 1 (also body-free) never poisons anything.
        let scope = isolation_scope(&records, &graph, 1, 3);
        assert_eq!(scope.independent_steps, vec![2, 3]);
        assert_eq!(scope.excluded_steps, vec![1]);
    }

    #[test]
    fn suppressed_records_are_neither_excluded_nor_independent() {
        let mut records = vec![extrude(0x10), extrude(0x11), extrude(0x12), extrude(0x13)];
        records[2].suppressed = true;
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 1, 3);
        assert_eq!(scope.independent_steps, vec![3]);
        assert_eq!(scope.excluded_steps, vec![1]);
    }

    #[test]
    fn records_below_the_halt_are_never_excluded() {
        let records = vec![extrude(0x10), extrude(0x11), extrude(0x12)];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 2, 2);
        assert_eq!(scope.excluded_steps, vec![2]);
        assert!(
            !scope.is_worth_running(),
            "a halt at the target leaves nothing to isolate"
        );
    }

    #[test]
    fn the_target_bounds_the_scan() {
        let records = vec![extrude(0x10), extrude(0x11), extrude(0x12), extrude(0x13)];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 1, 2);
        assert_eq!(
            scope.independent_steps,
            vec![2],
            "step 3 is above the target"
        );
    }

    #[test]
    fn an_out_of_range_halt_yields_an_empty_scope() {
        let records = vec![extrude(0x10)];
        let graph = graph_of(&records);
        let scope = isolation_scope(&records, &graph, 9, 9);
        assert_eq!(scope, IsolationScope::default());
        assert!(!scope.is_worth_running());
    }
}
