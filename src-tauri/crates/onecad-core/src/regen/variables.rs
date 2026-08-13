//! Document-variable substitution — the pass that makes a [`Scalar`]'s `expr`
//! actually drive geometry (WP-VE.1).
//!
//! A [`Scalar`] carries `{ value, expr? }`, where `value` is the last evaluated
//! (cached) number and `expr` — in V1 — is a **bare document-variable name**
//! (`crate::document::variables`). Nothing evaluated it: every consumer read
//! `value`, so editing the variable table moved no geometry at all.
//!
//! This module is that evaluation, and it runs at **regen planning time on
//! EFFECTIVE COPIES**:
//!
//! * The stored document is never mutated here. The persisted `value` stays the
//!   last-known number, which is exactly what keeps a document loadable (and
//!   showing sane dimensions) for a build that does not run this pass.
//! * The substituted records become the regen mirror's timeline, and the mirror
//!   is what [`RegenPlanner`](super::planner::RegenPlanner) hashes AND what the
//!   wire lowering serializes. One source ⇒ the planner hash and the geometry
//!   the worker builds can never disagree, and a variable edit correctly
//!   invalidates every checkpoint at or after the step it moves (the checkpoint
//!   guard compares [`history_prefix_hash`](super::planner::history_prefix_hash)
//!   over these same effective records).
//! * A document with **no** expr-bound scalar substitutes to itself, byte for
//!   byte, so its hashes are identical to a build without this pass (golden-pin
//!   invariance).
//!
//! ## Failure is loud, per step, and never a stale fallback
//!
//! An `expr` naming a variable the table does not hold — or an `expr` that is
//! not a bare name at all (arithmetic is deferred) — does NOT fall back to the
//! cached `value`. That fallback is the silent-wrong-number failure mode this
//! whole codebase refuses (deterministic `NeedsRepair`/`Error` beats a silent
//! wrong result). Instead the offending step is reported through
//! [`UnresolvedVariable`], the caller stamps it
//! [`StepState::Error`](crate::history::StepState::Error) and plans strictly
//! BELOW it, and every downstream step behaves exactly as it does after any
//! other failed step.
//!
//! **Suppressed records are skipped entirely** — the same rule the planner and
//! `history_prefix_hash` apply. A suppressed op contributes no hash line and
//! never executes, so it can neither be substituted nor block anything.

use std::collections::BTreeMap;

use crate::document::record::{Operation, OperationRecord};
use crate::document::variables::{Scalar, VariableTable};
use crate::history::{StepState, Timeline};
use crate::ids::RecordId;

/// A [`Scalar`] whose `expr` could not be resolved against the document's
/// [`VariableTable`] — a per-step, recoverable planning refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedVariable {
    /// The timeline step carrying the offending record.
    pub step_index: usize,
    /// The record itself (stable across a re-plan; the step index is not).
    pub record_id: RecordId,
    /// `opType.field` label of the offending parameter (see
    /// [`KnownOperation::scalars_mut`](crate::document::record::KnownOperation::scalars_mut)).
    pub field: &'static str,
    /// The `expr` text verbatim.
    pub expr: String,
    /// The human-facing reason, naming both the field and the expression.
    pub message: String,
}

/// Resolves a V1 expression — a **bare variable name** — to the variable's
/// current value.
///
/// # Errors
/// A human-facing message when the expression is not a bare identifier, names
/// no variable in `vars`, or names a variable whose own value is itself
/// expression-driven (chained expressions are not V1).
pub fn resolve_expr(expr: &str, vars: &VariableTable) -> Result<f64, String> {
    let name = expr.trim();
    if !is_bare_name(name) {
        return Err(format!(
            "expression `{expr}` is not a bare variable name — V1 supports a bare \
             variable name only (no arithmetic)"
        ));
    }
    let var = vars
        .get(name)
        .ok_or_else(|| format!("variable `{name}` is not defined in this document"))?;
    if var.value.expr.is_some() {
        return Err(format!(
            "variable `{name}` is itself expression-driven — chained variable \
             expressions are not supported in V1"
        ));
    }
    Ok(var.value.value)
}

/// A bare V1 variable name: a non-empty ASCII identifier (`[A-Za-z_][A-Za-z0-9_]*`).
///
/// Deliberately stricter than "whatever [`VariableTable::get`] would match": an
/// `expr` like `w * 2` must fail LOUDLY as an unsupported expression rather than
/// look up a variable literally named `"w * 2"` and report it as missing.
///
/// Public because the authoring boundary (WP-VE.2's `upsert_variable` command)
/// must refuse exactly the names this resolver could never look up — a variable
/// the user can create but no `expr` can ever name is a trap, not a feature.
#[must_use]
pub fn is_bare_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Substitutes every expr-bound [`Scalar`] in `records` **in place** with its
/// document-variable value, returning the first unresolvable scalar per step.
///
/// `records` must be the caller's own EFFECTIVE COPY — never the stored
/// document's records (see the module docs). `expr` itself is preserved: only
/// the cached `value` moves, so the binding survives a save/reload round-trip.
///
/// A record with no expr-bound scalar is left byte-identical, so a document
/// without variables hashes exactly as it did before this pass existed.
pub fn substitute_variables(
    records: &mut [OperationRecord],
    vars: &VariableTable,
) -> BTreeMap<usize, UnresolvedVariable> {
    let mut unresolved: BTreeMap<usize, UnresolvedVariable> = BTreeMap::new();
    for (step_index, record) in records.iter_mut().enumerate() {
        if record.suppressed {
            continue;
        }
        let record_id = record.record_id;
        // A frozen (Opaque) node exposes no typed scalars and never regenerates.
        let Operation::Known(known) = &mut record.op else {
            continue;
        };
        for (field, scalar) in known.scalars_mut() {
            let Some(expr) = scalar.expr.clone() else {
                continue;
            };
            match resolve_expr(&expr, vars) {
                Ok(value) => scalar.value = value,
                Err(reason) => {
                    unresolved
                        .entry(step_index)
                        .or_insert_with(|| UnresolvedVariable {
                            step_index,
                            record_id,
                            field,
                            expr: expr.clone(),
                            message: format!("{field}: {reason}"),
                        });
                }
            }
        }
    }
    unresolved
}

/// The EFFECTIVE timeline for a regen: `src`'s records with every expr-bound
/// [`Scalar`] resolved against `vars`, plus the steps that could not resolve.
///
/// Shaped exactly like the plain regen-mirror rebuild it replaces
/// ([`Timeline::from_records`] + [`Timeline::set_cursor`], so every step starts
/// `Dirty`/`Suppressed`), except that an unresolvable step is stamped
/// [`StepState::Error`] up front — the state is a pure function of
/// (records, variables), so it needs no worker round-trip to be true, and the
/// history row shows the reason the moment the binding breaks.
///
/// Only steps inside the applied prefix `[0, cursor)` are stamped: a draft
/// beyond the rollback bar never executes.
#[must_use]
pub fn substituted_timeline(
    src: &Timeline,
    vars: &VariableTable,
) -> (Timeline, BTreeMap<usize, UnresolvedVariable>) {
    let mut records = src.records().to_vec();
    let unresolved = substitute_variables(&mut records, vars);
    let mut out = Timeline::from_records(records);
    out.set_cursor(src.cursor());
    for (&step, item) in &unresolved {
        if step < out.cursor() {
            let _ = out.mark_state(
                step,
                StepState::Error {
                    reason: item.message.clone(),
                },
            );
        }
    }
    (out, unresolved)
}

/// Copies the RESOLVED `value` of every expr-bound [`Scalar`] from `source` (an
/// effective, already-substituted record) onto `target` (the stored record),
/// leaving `expr` untouched. Returns whether anything changed.
///
/// The derived, no-undo write-back that keeps a SAVED document carrying current
/// numbers instead of the values from whenever the record was last hand-edited
/// — the same treatment `outputs` and a reseated mate `placement` already get.
/// Position-zipped over
/// [`KnownOperation::scalars_mut`](crate::document::record::KnownOperation::scalars_mut),
/// which is why that table's order is normative.
///
/// Refuses (no-op) unless both ops are the same `Known` variant exposing the
/// same scalar arity, and only ever moves a scalar whose `expr` matches the
/// source's — a mismatch means the two records drifted apart and nothing may be
/// copied blind.
pub fn write_back_resolved_values(target: &mut Operation, source: &Operation) -> bool {
    let (Operation::Known(dst), Operation::Known(src)) = (target, source) else {
        return false;
    };
    if std::mem::discriminant(&*dst) != std::mem::discriminant(src) {
        return false;
    }
    if !dst.scalars_mut().iter().any(|(_, s)| s.expr.is_some()) {
        return false; // nothing expression-driven — skip the source clone.
    }
    let mut src_copy = src.clone();
    let src_scalars: Vec<Scalar> = src_copy
        .scalars_mut()
        .into_iter()
        .map(|(_, s)| s.clone())
        .collect();
    let dst_scalars = dst.scalars_mut();
    if dst_scalars.len() != src_scalars.len() {
        return false;
    }
    let mut changed = false;
    for ((_, d), s) in dst_scalars.into_iter().zip(src_scalars) {
        if d.expr.is_some() && d.expr == s.expr && d.value != s.value {
            d.value = s.value;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, OperationRecord,
    };
    use crate::document::variables::{Unit, Variable};
    use crate::ids::VariableId;
    use crate::regen::history_prefix_hash;
    use uuid::Uuid;

    /// Distinct record ids per fixture so two records in one timeline never
    /// collide (the timeline rejects duplicates).
    fn next_id() -> RecordId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(1);
        RecordId(Uuid::from_u128(u128::from(
            SEQ.fetch_add(1, Ordering::Relaxed),
        )))
    }

    fn table(pairs: &[(&str, f64)]) -> VariableTable {
        let mut t = VariableTable::new();
        for (name, value) in pairs {
            t.upsert(Variable {
                id: VariableId::new(),
                name: (*name).to_string(),
                value: Scalar::new(*value),
                unit: Unit::Mm,
            });
        }
        t
    }

    fn extrude(distance: Scalar) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: None,
            distance,
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
        OperationRecord::new(next_id(), 0, "Extrude", op)
    }

    #[test]
    fn a_bare_name_resolves_to_the_table_value() {
        let vars = table(&[("width", 20.0)]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "width"))];
        assert!(substitute_variables(&mut records, &vars).is_empty());
        let Operation::Known(KnownOperation::Extrude(p)) = &records[0].op else {
            panic!()
        };
        assert_eq!(p.distance.value, 20.0);
        // The binding survives — only the cached value moved.
        assert_eq!(p.distance.expr.as_deref(), Some("width"));
    }

    #[test]
    fn a_missing_variable_fails_that_step_and_never_falls_back() {
        let vars = table(&[("width", 20.0)]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "height"))];
        let bad = substitute_variables(&mut records, &vars);
        let item = bad.get(&0).expect("step 0 unresolved");
        assert_eq!(item.field, "Extrude.distance");
        assert!(item.message.contains("height"), "{}", item.message);
        // The stale cached value is preserved but the step will not execute.
        let Operation::Known(KnownOperation::Extrude(p)) = &records[0].op else {
            panic!()
        };
        assert_eq!(p.distance.value, 10.0);
    }

    #[test]
    fn arithmetic_is_refused_as_unsupported_not_as_missing() {
        let vars = table(&[("w", 20.0)]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "w * 2"))];
        let bad = substitute_variables(&mut records, &vars);
        let msg = &bad.get(&0).expect("unresolved").message;
        assert!(msg.contains("bare variable name"), "{msg}");
    }

    #[test]
    fn whitespace_around_a_bare_name_is_tolerated() {
        let vars = table(&[("w", 3.5)]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "  w  "))];
        assert!(substitute_variables(&mut records, &vars).is_empty());
        let Operation::Known(KnownOperation::Extrude(p)) = &records[0].op else {
            panic!()
        };
        assert_eq!(p.distance.value, 3.5);
    }

    #[test]
    fn a_suppressed_record_is_neither_substituted_nor_blocking() {
        let vars = table(&[]);
        let mut rec = extrude(Scalar::with_expr(10.0, "nope"));
        rec.suppressed = true;
        let mut records = vec![rec];
        assert!(substitute_variables(&mut records, &vars).is_empty());
    }

    #[test]
    fn a_document_without_expressions_hashes_identically_with_and_without_variables() {
        let records = vec![extrude(Scalar::new(10.0))];
        let before = history_prefix_hash(&records);
        let mut effective = records.clone();
        assert!(substitute_variables(&mut effective, &table(&[("width", 999.0)])).is_empty());
        assert_eq!(
            effective, records,
            "a no-expr record must substitute to itself"
        );
        assert_eq!(history_prefix_hash(&effective), before);
    }

    #[test]
    fn substitution_moves_the_hash_when_the_variable_moves() {
        let records = vec![extrude(Scalar::with_expr(10.0, "width"))];
        let mut a = records.clone();
        let mut b = records.clone();
        substitute_variables(&mut a, &table(&[("width", 10.0)]));
        substitute_variables(&mut b, &table(&[("width", 20.0)]));
        assert_ne!(history_prefix_hash(&a), history_prefix_hash(&b));
    }

    #[test]
    fn write_back_moves_only_expression_driven_values() {
        let stored = extrude(Scalar::with_expr(10.0, "width"));
        let mut effective = stored.clone();
        substitute_variables(
            std::slice::from_mut(&mut effective),
            &table(&[("width", 20.0)]),
        );
        let mut target = stored.op.clone();
        assert!(write_back_resolved_values(&mut target, &effective.op));
        let Operation::Known(KnownOperation::Extrude(p)) = &target else {
            panic!()
        };
        assert_eq!(p.distance.value, 20.0);
        assert_eq!(p.distance.expr.as_deref(), Some("width"));
        // Idempotent.
        assert!(!write_back_resolved_values(&mut target, &effective.op));
    }

    #[test]
    fn write_back_ignores_a_literal_only_record() {
        let stored = extrude(Scalar::new(10.0));
        let other = extrude(Scalar::new(99.0));
        let mut target = stored.op.clone();
        assert!(!write_back_resolved_values(&mut target, &other.op));
        assert_eq!(target, stored.op);
    }

    #[test]
    fn substituted_timeline_stamps_the_failing_step_as_error() {
        let ok = extrude(Scalar::with_expr(1.0, "w"));
        let bad = extrude(Scalar::with_expr(1.0, "missing"));
        let src = Timeline::from_records(vec![ok, bad]);
        let (out, unresolved) = substituted_timeline(&src, &table(&[("w", 4.0)]));
        assert_eq!(unresolved.len(), 1);
        assert!(matches!(out.state(0), Some(StepState::Dirty)));
        assert!(matches!(out.state(1), Some(StepState::Error { .. })));
        assert_eq!(out.cursor(), src.cursor());
    }
}
