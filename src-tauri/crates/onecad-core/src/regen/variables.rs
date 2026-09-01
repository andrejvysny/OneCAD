//! Document-variable substitution — the pass that makes a [`Scalar`]'s `expr`
//! actually drive geometry (WP-VE.1, widened to real expressions by WP-VE.2).
//!
//! A [`Scalar`] carries `{ value, expr? }`, where `value` is the last evaluated
//! (cached) number and `expr` is an **arithmetic expression** over document
//! variables and unit-suffixed literals (`depth * 2 + 5mm`), evaluated by
//! [`crate::expr`]. Nothing evaluated it before this pass: every consumer read
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
//! ## Two passes, in order
//!
//! 1. [`resolve_variable_table`] resolves the [`VariableTable`] itself —
//!    depth-first with a visiting stack, so a variable may reference another
//!    variable (`plate = w * 2`) to a depth of [`MAX_VARIABLE_CHAIN`]. A cycle
//!    is a deterministic error naming the whole path, never a hang and never a
//!    partial value. A variable's DIMENSION is inferred from its own expression
//!    (`45deg` ⇒ angle); one with no expression at all is dimensionless, so it
//!    reads as whatever unit the call site means.
//! 2. [`substitute_variables`] evaluates every registered op-parameter
//!    expression against that resolution, at the [`Dimension`] the parameter's
//!    registry entry declares
//!    ([`KnownOperation::scalars_mut`](crate::document::record::KnownOperation::scalars_mut)).
//!    The site dimension is what turns `"45deg"` in an `Extrude.distance` into a
//!    loud refusal instead of a silent 45 mm. A bare number inside an expression
//!    is read as ALREADY being in the site's canonical unit (mm / deg), so
//!    `depth * 2` in a length field means millimetres with no display-unit
//!    guesswork anywhere on this path.
//!
//! ## Failure is loud, per step, and never a stale fallback
//!
//! An expression that does not parse, names a variable the table does not hold,
//! names one that is itself broken, or produces the wrong dimension for its call
//! site does NOT fall back to the cached `value`. That fallback is the
//! silent-wrong-number failure mode this whole codebase refuses (deterministic
//! `NeedsRepair`/`Error` beats a silent wrong result). Instead the offending step
//! is reported through [`UnresolvedVariable`], the caller stamps it
//! [`StepState::Error`](crate::history::StepState) and plans strictly BELOW it,
//! and every downstream step behaves exactly as it does after any other failed
//! step.
//!
//! **Suppressed records are skipped entirely** — the same rule the planner and
//! `history_prefix_hash` apply. A suppressed op contributes no hash line and
//! never executes, so it can neither be substituted nor block anything.

use std::collections::BTreeMap;

use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::document::variables::{Scalar, VariableTable};
use crate::expr::{self, Dimension, Env, ExprErrorKind, Value};
use crate::history::{StepState, Timeline};
use crate::ids::RecordId;

/// Hard cap on how deep a variable may reference another variable. Bounds the
/// resolver's recursion; a chain past it is an error, not a stack overflow.
pub const MAX_VARIABLE_CHAIN: usize = 64;

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
    /// The failure MODE, for machine routing (a projection diagnostic's
    /// `reasonCode`) — callers must never match on `message` text.
    pub kind: ExprErrorKind,
    /// The human-facing reason, naming both the field and the expression.
    pub message: String,
}

/// A [`Variable`](crate::document::variables::Variable) whose own expression
/// could not be resolved against the rest of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableError {
    /// The variable's name.
    pub name: String,
    /// The failure MODE, for machine routing — never match on `message` text.
    pub kind: ExprErrorKind,
    /// The human-facing reason.
    pub message: String,
}

/// Resolves the whole [`VariableTable`] into `name → `[`Value`] plus the
/// variables that could not resolve.
///
/// Depth-first with a visiting stack, so a variable may be defined in terms of
/// another one regardless of declaration order. Three refusals, all
/// deterministic and all naming the cause:
///
/// * **Cycle** — every member of the cycle gets the same
///   [`ExprErrorKind::Cycle`] error naming the whole path (`a → b → a`). The
///   expression engine has no notion of a variable graph and never raises this
///   kind itself; this function owns it.
/// * **Over-long chain** — deeper than [`MAX_VARIABLE_CHAIN`].
/// * **Bad expression** — whatever [`crate::expr`] refused it for, verbatim.
///
/// A variable with no expression is a bare number, and a bare number is
/// [`Dimension::Scalar`]: it coerces into whatever unit the call site means, so
/// `depth = 10` drives a length field as 10 mm and an angle field as 10°. A
/// variable that WANTS a fixed dimension says so in its expression (`10mm`).
///
/// An errored variable is ABSENT from the value map, so anything referencing it
/// fails too rather than silently reading a stale number.
#[must_use]
pub fn resolve_variable_table(
    vars: &VariableTable,
) -> (BTreeMap<String, Value>, Vec<VariableError>) {
    let mut values = BTreeMap::new();
    let mut errors: Vec<VariableError> = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    for var in vars.iter() {
        resolve_variable(&var.name, vars, &mut values, &mut errors, &mut stack);
    }
    (values, errors)
}

/// First error per name wins: the cycle detection stamps every member of a cycle
/// up front, and the unwinding evaluation must not overwrite that specific
/// diagnosis with the generic "undefined variable" it then hits.
fn push_variable_error(
    errors: &mut Vec<VariableError>,
    name: &str,
    kind: ExprErrorKind,
    message: String,
) {
    if errors.iter().any(|e| e.name == name) {
        return;
    }
    errors.push(VariableError {
        name: name.to_string(),
        kind,
        message,
    });
}

fn resolve_variable<'a>(
    name: &'a str,
    vars: &'a VariableTable,
    values: &mut BTreeMap<String, Value>,
    errors: &mut Vec<VariableError>,
    stack: &mut Vec<&'a str>,
) {
    if values.contains_key(name) || errors.iter().any(|e| e.name == name) {
        return;
    }
    let Some(var) = vars.get(name) else {
        return; // not a variable at all — the evaluator reports it at the use site.
    };
    if let Some(at) = stack.iter().position(|n| *n == name) {
        let mut path: Vec<&str> = stack[at..].to_vec();
        path.push(name);
        let message = format!("variable cycle: {}", path.join(" → "));
        for member in stack[at..].iter().copied() {
            push_variable_error(errors, member, ExprErrorKind::Cycle, message.clone());
        }
        return;
    }
    if stack.len() >= MAX_VARIABLE_CHAIN {
        push_variable_error(
            errors,
            name,
            ExprErrorKind::TooDeep,
            format!("variable reference chain exceeds {MAX_VARIABLE_CHAIN}"),
        );
        return;
    }
    let Some(text) = var.value.expr.as_deref() else {
        values.insert(
            name.to_string(),
            Value {
                number: var.value.value,
                dim: Dimension::Scalar,
            },
        );
        return;
    };
    let ast = match expr::parse(text) {
        Ok(ast) => ast,
        Err(e) => {
            push_variable_error(errors, name, e.kind, e.message);
            return;
        }
    };
    stack.push(&var.name);
    for referenced in expr::references(&ast) {
        // Re-look-up through the table so the recursive borrow lives as long as
        // `vars`, not as long as this local AST.
        if let Some(dep) = vars.get(referenced) {
            resolve_variable(&dep.name, vars, values, errors, stack);
        }
    }
    stack.pop();
    // A dependency that is itself broken must be reported as such, not as
    // "undefined variable `b`" about a `b` that is plainly in the table — the
    // same guard (and the same reason) as `evaluate_at_site`. Checked AFTER the
    // recursion, so every dependency has had its chance to resolve.
    if let Some(bad) = broken_reference(&ast, errors) {
        let (kind, message) = (bad.kind, bad.message.clone());
        let bad_name = bad.name.clone();
        push_variable_error(
            errors,
            name,
            kind,
            format!("variable `{bad_name}` is itself unresolved: {message}"),
        );
        return;
    }
    let evaluated = {
        let lookup = |n: &str| values.get(n).copied();
        expr::evaluate_value(&ast, &Env::new(&lookup))
    };
    match evaluated {
        Ok(value) => {
            values.insert(name.to_string(), value);
        }
        Err(e) => push_variable_error(errors, name, e.kind, e.message),
    }
}

/// The first identifier in `ast` naming a variable that is ITSELF unresolved.
///
/// Shared by the variable pass and the parameter pass so both explain a broken
/// dependency the same way; the evaluator cannot, because to it an unresolvable
/// name and a non-existent one are the same missing `Env` entry.
fn broken_reference<'a>(
    ast: &expr::Expr,
    errors: &'a [VariableError],
) -> Option<&'a VariableError> {
    expr::references(ast)
        .into_iter()
        .find_map(|r| errors.iter().find(|e| e.name == r))
}

/// Evaluates one expression at its call-site [`Dimension`], returning the
/// dimensioned [`Value`] or `(kind, reason)`.
///
/// `var_errors` is consulted BEFORE evaluation so a parameter bound to a broken
/// variable reports the real cause. Without it the evaluator would say
/// "undefined variable `w`" about a `w` that plainly exists in the table and is
/// merely unresolvable — a message that sends the user looking in the wrong
/// place.
///
/// The site boundary is enforced by calling [`expr::check_site`] rather than
/// re-deriving its rule here, so "which dimensions may coerce into which site"
/// stays defined in exactly one place — and the tree is walked once, not twice.
fn evaluate_at_site(
    text: &str,
    site: Dimension,
    env: &Env,
    var_errors: &[VariableError],
) -> Result<Value, (ExprErrorKind, String)> {
    let ast = expr::parse(text).map_err(|e| (e.kind, e.message))?;
    if let Some(bad) = broken_reference(&ast, var_errors) {
        return Err((
            bad.kind,
            format!(
                "variable `{}` is itself unresolved: {}",
                bad.name, bad.message
            ),
        ));
    }
    // ONE walk: evaluate, then apply the site boundary to the value already in
    // hand. Going back through `expr::evaluate` would re-walk the tree for every
    // bound scalar on every regen, to recompute a number we just computed.
    let value = expr::evaluate_value(&ast, env).map_err(|e| (e.kind, e.message))?;
    expr::check_site(&ast, value, site).map_err(|e| (e.kind, e.message))?;
    Ok(value)
}

/// Evaluates `text` at `site` against `vars` — the standalone entry point for a
/// live authoring preview, where the caller has an expression and a call site
/// but no record to put them in.
///
/// Pure: it reads the table and mutates nothing.
///
/// # Errors
/// `(kind, reason)` — the failure MODE for machine routing plus the
/// human-facing text. See [`evaluate_at_site`].
pub fn evaluate_expression(
    text: &str,
    site: Dimension,
    vars: &VariableTable,
) -> Result<Value, (ExprErrorKind, String)> {
    let (values, var_errors) = resolve_variable_table(vars);
    let lookup = |n: &str| values.get(n).copied();
    evaluate_at_site(text, site, &Env::new(&lookup), &var_errors)
}

/// Checks the `Scalar` expressions this edit INTRODUCES or CHANGES, against
/// `vars` — the EDIT-TIME gate, so an expression that could never resolve is
/// refused before it is ever written rather than discovered at the next regen.
///
/// `prior` is the record's previous operation (`None` for an insert, where every
/// expression is new by definition). An expression the edit leaves BYTE
/// IDENTICAL is deliberately not re-checked: unlike every other validator in the
/// edit chain, this one depends on state the record does not own, so a binding
/// that was legal when authored can be invalidated by a later variable edit. Had
/// this checked the whole op, that record would become uneditable on its OTHER
/// fields until its broken binding was fixed — punishing the user for a change
/// they made somewhere else. Later breakage is a DIAGNOSTIC, not a refusal: it
/// keeps surfacing per-step as `EXPR_UNRESOLVED` at regen (see the module docs),
/// and the user clears it by typing a literal into the field.
///
/// Comparison is keyed by the `opType.field` LABEL from
/// [`KnownOperation::scalars_mut`](crate::document::record::KnownOperation::scalars_mut),
/// never by position — see [`unchanged_expressions`] for why the obvious
/// position zip is unsound here. A field the prior op did not expose under that
/// exact label (a new optional scalar, a swapped Chamfer mode, a different op
/// variant entirely) is new by definition and is validated.
///
/// `Err` carries one `field: reason` message (the first failure). An
/// [`Operation::Opaque`] frozen node exposes no typed scalars and is always
/// `Ok`.
///
/// # Errors
/// The first newly introduced or changed `field: reason` that fails to parse,
/// references a missing or broken variable, or produces the wrong dimension for
/// its call site.
pub fn validate_op_expressions(
    op: &Operation,
    prior: Option<&Operation>,
    vars: &VariableTable,
) -> Result<(), String> {
    let Operation::Known(known) = op else {
        return Ok(());
    };
    let mut probe = known.clone();
    let unchanged = unchanged_expressions(&mut probe, prior);
    if unchanged.iter().all(|skip| *skip) && !unchanged.is_empty() {
        return Ok(()); // nothing new to check — skip resolving the table at all.
    }
    let (values, var_errors) = resolve_variable_table(vars);
    let lookup = |n: &str| values.get(n).copied();
    let env = Env::new(&lookup);
    for (index, (field, site, scalar)) in probe.scalars_mut().into_iter().enumerate() {
        if unchanged.get(index).copied().unwrap_or(false) {
            continue;
        }
        let Some(text) = scalar.expr.as_deref() else {
            continue;
        };
        if let Err((_, reason)) = evaluate_at_site(text, site, &env, &var_errors) {
            return Err(format!("{field}: {reason}"));
        }
    }
    Ok(())
}

/// Per registered scalar of `probe`: true iff `prior` carries the byte-identical
/// `expr` under the SAME field label. All-false when there is no comparable
/// prior.
///
/// **Keyed by label, never by position.** A position zip looks tempting (the
/// registry's order is normative and `write_back_resolved_values` relies on it)
/// but it is wrong for this job, because two ops of the same variant can expose
/// the same NUMBER of scalars with different MEANINGS at the same index: a
/// Chamfer swapped from distance-distance to distance-angle still has arity 2,
/// and index 1 silently changes from `Chamfer.distance2` (a length) to
/// `Chamfer.angleDeg` (an angle). Under a position zip an unchanged expression
/// text at that index reads as "nothing to check" and a genuine dimension
/// mismatch reaches the record. The label is the field's identity, so a slot
/// that changed meaning is simply absent from the prior map and gets validated
/// — which also removes the need for any variant/arity escape hatch.
fn unchanged_expressions(probe: &mut KnownOperation, prior: Option<&Operation>) -> Vec<bool> {
    let Some(Operation::Known(prior_known)) = prior else {
        return vec![false; probe.scalars_mut().len()];
    };
    let mut prior_copy = prior_known.clone();
    let prior_exprs: BTreeMap<&'static str, Option<String>> = prior_copy
        .scalars_mut()
        .into_iter()
        .map(|(field, _, s)| (field, s.expr.clone()))
        .collect();
    probe
        .scalars_mut()
        .into_iter()
        .map(|(field, _, scalar)| prior_exprs.get(field) == Some(&scalar.expr))
        .collect()
}

/// A legal variable NAME: a non-empty ASCII identifier
/// (`[A-Za-z_][A-Za-z0-9_]*`) — exactly what the expression lexer tokenizes as
/// an identifier.
///
/// The authoring boundary (the `upsert_variable` command) refuses anything else,
/// because a variable the user can create but no expression can ever name is a
/// trap, not a feature. Note this is about the variable's NAME; its VALUE may be
/// any expression [`crate::expr`] accepts.
#[must_use]
pub fn is_bare_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Rewrites every reference to `old` in `text` as `new`, returning `None` when
/// `text` does not reference `old` at all.
///
/// **Token-wise, via the parser's identifier spans — never textual search.** A
/// substring rewrite would corrupt `width2` while renaming `width`, and would
/// rename a FUNCTION call (`sin(x)`) if a variable were ever named `sin`; the
/// AST distinguishes an [`expr::Expr::Ident`] from an [`expr::Expr::Call`] name
/// and from a unit suffix, so this cannot.
///
/// The caller is responsible for `new` being a legal name ([`is_bare_name`]) —
/// splicing an illegal one in would produce text that no longer parses.
///
/// # Errors
/// The expression's own parse error, when `text` cannot be parsed at all. A
/// rename must then be REFUSED rather than applied to some sites and not others:
/// a half-renamed document is a silently wrong document.
pub fn rename_reference(text: &str, old: &str, new: &str) -> Result<Option<String>, String> {
    let ast = expr::parse(text).map_err(|e| e.message)?;
    let mut spans = Vec::new();
    collect_ident_spans(&ast, old, &mut spans);
    if spans.is_empty() {
        return Ok(None);
    }
    // Splice back-to-front so earlier spans keep their byte offsets.
    spans.sort_unstable_by_key(|(start, _)| std::cmp::Reverse(*start));
    let mut out = text.to_string();
    for (start, end) in spans {
        out.replace_range(start..end, new);
    }
    Ok(Some(out))
}

fn collect_ident_spans(node: &expr::Expr, name: &str, out: &mut Vec<(usize, usize)>) {
    match node {
        expr::Expr::Number { .. } => {}
        expr::Expr::Ident { name: n, span } => {
            if n == name {
                out.push(*span);
            }
        }
        expr::Expr::Unary { expr, .. } => collect_ident_spans(expr, name, out),
        expr::Expr::Binary { lhs, rhs, .. } => {
            collect_ident_spans(lhs, name, out);
            collect_ident_spans(rhs, name, out);
        }
        // `Call.name` is deliberately NOT a rewrite site: a builtin is not a
        // variable, so `sin(sin)` renames only the argument.
        expr::Expr::Call { args, .. } => {
            for arg in args {
                collect_ident_spans(arg, name, out);
            }
        }
    }
}

/// Substitutes every expr-bound [`Scalar`] in `records` **in place** with the
/// number its expression evaluates to, returning the first unresolvable scalar
/// per step.
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
    let (values, var_errors) = resolve_variable_table(vars);
    let lookup = |n: &str| values.get(n).copied();
    let env = Env::new(&lookup);
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
        for (field, site, scalar) in known.scalars_mut() {
            let Some(text) = scalar.expr.clone() else {
                continue;
            };
            match evaluate_at_site(&text, site, &env, &var_errors) {
                Ok(value) => scalar.value = value.number,
                Err((kind, reason)) => {
                    unresolved
                        .entry(step_index)
                        .or_insert_with(|| UnresolvedVariable {
                            step_index,
                            record_id,
                            field,
                            expr: text.clone(),
                            kind,
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
/// **The returned map is the ONE view of a broken binding**, and it holds only
/// steps inside the applied prefix `[0, cursor)`. A draft beyond the rollback
/// bar never executes, so it cannot fail: it must not be stamped
/// [`StepState::Error`], must not gate the plan ceiling, must not appear in a
/// report's `failed_steps`, and must not draw a diagnostic on its history row.
/// Filtering HERE rather than at each consumer is what keeps those four answers
/// the same — the earlier shape stamped a filtered set but returned an
/// unfiltered one, so the three runtime consumers all disagreed with the
/// timeline they were describing.
#[must_use]
pub fn substituted_timeline(
    src: &Timeline,
    vars: &VariableTable,
) -> (Timeline, BTreeMap<usize, UnresolvedVariable>) {
    let mut records = src.records().to_vec();
    let unresolved = substitute_variables(&mut records, vars);
    let mut out = Timeline::from_records(records);
    out.set_cursor(src.cursor());
    let cursor = out.cursor();
    let unresolved: BTreeMap<usize, UnresolvedVariable> = unresolved
        .into_iter()
        .filter(|(step, _)| *step < cursor)
        .collect();
    for (&step, item) in &unresolved {
        let _ = out.mark_state(
            step,
            StepState::Error {
                reason: item.message.clone(),
            },
        );
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
    if !dst.scalars_mut().iter().any(|(_, _, s)| s.expr.is_some()) {
        return false; // nothing expression-driven — skip the source clone.
    }
    let mut src_copy = src.clone();
    let src_scalars: Vec<Scalar> = src_copy
        .scalars_mut()
        .into_iter()
        .map(|(_, _, s)| s.clone())
        .collect();
    let dst_scalars = dst.scalars_mut();
    if dst_scalars.len() != src_scalars.len() {
        return false;
    }
    let mut changed = false;
    for ((_, _, d), s) in dst_scalars.into_iter().zip(src_scalars) {
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

    fn distance_of(rec: &OperationRecord) -> &Scalar {
        let Operation::Known(KnownOperation::Extrude(p)) = &rec.op else {
            panic!("expected an Extrude record")
        };
        &p.distance
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
    fn arithmetic_over_a_variable_resolves() {
        let vars = table(&[("w", 20.0)]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "w * 2 + 5mm"))];
        assert!(substitute_variables(&mut records, &vars).is_empty());
        assert_eq!(distance_of(&records[0]).value, 45.0);
    }

    /// The site dimension is the whole point of the registry's `Dimension`
    /// column: an angle literal in a length field is a REFUSAL, never a silent
    /// 45 mm.
    #[test]
    fn an_angle_expression_in_a_length_field_is_a_dimension_mismatch() {
        let vars = table(&[]);
        let mut records = vec![extrude(Scalar::with_expr(10.0, "45deg"))];
        let bad = substitute_variables(&mut records, &vars);
        let item = bad.get(&0).expect("unresolved");
        assert_eq!(item.kind, ExprErrorKind::DimensionMismatch);
        assert_eq!(distance_of(&records[0]).value, 10.0, "no stale fallback");
    }

    /// A unit-suffixed literal converts to the canonical millimetre before it
    /// ever reaches a parameter.
    #[test]
    fn a_unit_suffixed_literal_converts_to_canonical_millimetres() {
        let vars = table(&[]);
        let mut records = vec![extrude(Scalar::with_expr(0.0, "1in"))];
        assert!(substitute_variables(&mut records, &vars).is_empty());
        assert!((distance_of(&records[0]).value - 25.4).abs() < 1e-12);
    }

    /// The chained reference WP-VE.1 refused outright.
    #[test]
    fn a_chained_variable_reference_resolves() {
        let mut vars = table(&[("w", 5.0)]);
        vars.upsert(Variable {
            id: VariableId::new(),
            name: "plate".to_string(),
            value: Scalar::with_expr(0.0, "w * 3"),
            unit: Unit::Mm,
        });
        let (values, errors) = resolve_variable_table(&vars);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(values["plate"].number, 15.0);

        let mut records = vec![extrude(Scalar::with_expr(0.0, "plate + 1mm"))];
        assert!(substitute_variables(&mut records, &vars).is_empty());
        assert_eq!(distance_of(&records[0]).value, 16.0);
    }

    #[test]
    fn a_variable_cycle_errors_every_member_and_names_the_path() {
        let mut vars = VariableTable::new();
        for (name, expr) in [("a", "b + 1"), ("b", "a + 1")] {
            vars.upsert(Variable {
                id: VariableId::new(),
                name: name.to_string(),
                value: Scalar::with_expr(0.0, expr),
                unit: Unit::Mm,
            });
        }
        let (values, errors) = resolve_variable_table(&vars);
        assert!(
            values.is_empty(),
            "a cycle yields no partial value: {values:?}"
        );
        assert_eq!(errors.len(), 2);
        for e in &errors {
            assert_eq!(e.kind, ExprErrorKind::Cycle);
            assert!(e.message.contains("a → b → a"), "{}", e.message);
        }
        // Deterministic order: the cycle is reported from where it closes.
        assert_eq!(errors[0].name, "a");
        assert_eq!(errors[1].name, "b");
    }

    #[test]
    fn a_reference_chain_past_the_cap_errors_instead_of_recursing() {
        // Declared DEEPEST-FIRST so resolving the head actually walks the whole
        // chain (declaration order would resolve each link before its user and
        // never nest at all).
        let mut vars = VariableTable::new();
        for i in (1..=(MAX_VARIABLE_CHAIN + 2)).rev() {
            vars.upsert(Variable {
                id: VariableId::new(),
                name: format!("v{i}"),
                value: Scalar::with_expr(0.0, format!("v{} + 1", i - 1)),
                unit: Unit::Mm,
            });
        }
        vars.upsert(Variable {
            id: VariableId::new(),
            name: "v0".to_string(),
            value: Scalar::new(1.0),
            unit: Unit::Mm,
        });
        let (_, errors) = resolve_variable_table(&vars);
        assert!(!errors.is_empty(), "an over-long chain must be refused");
        assert!(errors.iter().all(
            |e| e.kind == ExprErrorKind::TooDeep || e.kind == ExprErrorKind::UndefinedVariable
        ));
        // A long chain is NOT a cycle: the two refusals stay distinguishable, so
        // a UI can say "too many hops" rather than "you made a loop".
        assert!(errors.iter().all(|e| e.kind != ExprErrorKind::Cycle));
    }

    /// A parameter bound to a BROKEN variable must name the real cause, not
    /// claim the variable is missing when it is plainly in the table.
    #[test]
    fn a_parameter_bound_to_a_broken_variable_reports_the_real_cause() {
        let mut vars = VariableTable::new();
        vars.upsert(Variable {
            id: VariableId::new(),
            name: "w".to_string(),
            value: Scalar::with_expr(0.0, "1 +"),
            unit: Unit::Mm,
        });
        let mut records = vec![extrude(Scalar::with_expr(10.0, "w"))];
        let bad = substitute_variables(&mut records, &vars);
        let msg = &bad.get(&0).expect("unresolved").message;
        assert!(msg.contains("`w` is itself unresolved"), "{msg}");
    }

    #[test]
    fn validate_op_expressions_is_the_edit_time_gate() {
        let vars = table(&[("w", 20.0)]);
        assert!(
            validate_op_expressions(&extrude(Scalar::with_expr(0.0, "w / 2")).op, None, &vars)
                .is_ok()
        );
        let err =
            validate_op_expressions(&extrude(Scalar::with_expr(0.0, "45deg")).op, None, &vars)
                .expect_err("an angle in a length field is refused at edit time");
        assert!(err.starts_with("Extrude.distance:"), "{err}");
        let err = validate_op_expressions(&extrude(Scalar::with_expr(0.0, "nope")).op, None, &vars)
            .expect_err("an undefined variable is refused at edit time");
        assert!(err.contains("nope"), "{err}");
    }

    /// The scoping rule: an expression the edit leaves ALONE is not re-checked,
    /// so a record whose binding a later variable delete broke stays editable on
    /// its other fields.
    #[test]
    fn validation_is_scoped_to_the_expressions_an_edit_changes() {
        let empty = table(&[]);
        let broken = extrude(Scalar::with_expr(10.0, "depth"));

        // Whole-op check (as on an insert): refused, `depth` is gone.
        assert!(validate_op_expressions(&broken.op, None, &empty).is_err());

        // Same op as its OWN prior — nothing changed, so nothing is re-checked.
        assert!(validate_op_expressions(&broken.op, Some(&broken.op), &empty).is_ok());

        // Editing an UNRELATED field on that record is likewise accepted: the
        // broken `distance` expression is byte-identical to the prior's.
        let mut other_field = broken.clone();
        let Operation::Known(KnownOperation::Extrude(p)) = &mut other_field.op else {
            panic!()
        };
        p.draft_angle_deg = Scalar::new(3.0);
        assert!(validate_op_expressions(&other_field.op, Some(&broken.op), &empty).is_ok());

        // Typing a LITERAL over the broken binding is how the user clears it.
        let mut cleared = broken.clone();
        let Operation::Known(KnownOperation::Extrude(p)) = &mut cleared.op else {
            panic!()
        };
        p.distance = Scalar::new(12.0);
        assert!(validate_op_expressions(&cleared.op, Some(&broken.op), &empty).is_ok());

        // But CHANGING that expression to another bad one is still refused.
        let mut worse = broken.clone();
        let Operation::Known(KnownOperation::Extrude(p)) = &mut worse.op else {
            panic!()
        };
        p.distance = Scalar::with_expr(10.0, "depth * 2");
        let err = validate_op_expressions(&worse.op, Some(&broken.op), &empty)
            .expect_err("a CHANGED expression is checked");
        assert!(err.starts_with("Extrude.distance:"), "{err}");
    }

    /// F2 — the label-keyed comparison, against the case a position zip gets
    /// WRONG: a Chamfer mode swap keeps arity 2 while index 1 changes meaning
    /// from `distance2` (a length) to `angleDeg` (an angle). A position zip sees
    /// "same expression text at index 1, nothing to check" and lets a genuine
    /// dimension mismatch into the record.
    #[test]
    fn a_chamfer_mode_swap_is_validated_even_though_the_arity_is_unchanged() {
        use crate::document::record::ChamferParams;

        let chamfer = |distance2: Option<Scalar>, angle_deg: Option<Scalar>| {
            Operation::Known(KnownOperation::Chamfer(ChamferParams {
                radius: Scalar::new(1.0),
                distance2,
                angle_deg,
                edge_ids: Vec::new(),
                edges: Vec::new(),
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Default::default(),
            }))
        };
        // `w` is a LENGTH (its own expression says so), so `w` is legal in
        // `distance2` and a dimension mismatch in `angleDeg`.
        let mut vars = VariableTable::new();
        vars.upsert(Variable {
            id: VariableId::new(),
            name: "w".into(),
            value: Scalar::with_expr(10.0, "10mm"),
            unit: Unit::Mm,
        });

        let prior = chamfer(Some(Scalar::with_expr(10.0, "w")), None);
        assert!(validate_op_expressions(&prior, None, &vars).is_ok());

        // Same arity, same text at index 1 — and refused, because the LABEL
        // changed.
        let swapped = chamfer(None, Some(Scalar::with_expr(10.0, "w")));
        let err = validate_op_expressions(&swapped, Some(&prior), &vars)
            .expect_err("a slot that changed meaning must be re-validated");
        assert!(err.starts_with("Chamfer.angleDeg:"), "{err}");

        // A NEW optional scalar is likewise new by label, not by arity.
        let grown = chamfer(
            Some(Scalar::with_expr(10.0, "w")),
            Some(Scalar::with_expr(10.0, "w")),
        );
        let err = validate_op_expressions(&grown, Some(&prior), &vars)
            .expect_err("a newly exposed field is validated");
        assert!(err.starts_with("Chamfer.angleDeg:"), "{err}");
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

    /// F3 — a VARIABLE chained to a broken one must name the real cause, the
    /// same way a PARAMETER bound to one already did. `b` plainly exists, so
    /// "undefined variable `b`" would send the user to the wrong place.
    #[test]
    fn a_variable_chained_to_a_broken_variable_reports_the_real_cause() {
        let mut vars = VariableTable::new();
        for (name, text) in [("a", "b * 2"), ("b", "1 +")] {
            vars.upsert(Variable {
                id: VariableId::new(),
                name: name.into(),
                value: Scalar::with_expr(0.0, text),
                unit: Unit::Mm,
            });
        }
        let (values, errors) = resolve_variable_table(&vars);
        assert!(values.is_empty(), "{values:?}");

        let a = errors.iter().find(|e| e.name == "a").expect("`a` errors");
        assert!(
            a.message.contains("`b` is itself unresolved"),
            "{}",
            a.message
        );
        // `b` itself still reports the ROOT cause, not the relayed one.
        let b = errors.iter().find(|e| e.name == "b").expect("`b` errors");
        assert_eq!(b.kind, ExprErrorKind::Parse);
    }

    /// F6 — a draft beyond the rollback bar can never execute, so a broken
    /// binding on one is not a failure: it is absent from the returned map (the
    /// ONE view three runtime consumers read) and its step is left Dirty.
    #[test]
    fn a_broken_binding_on_a_draft_beyond_the_cursor_is_not_reported() {
        let ok = extrude(Scalar::with_expr(1.0, "w"));
        let draft = extrude(Scalar::with_expr(1.0, "missing"));
        let mut src = Timeline::from_records(vec![ok, draft]);
        src.set_cursor(1); // the second record sits BEYOND the rollback bar.

        let (out, unresolved) = substituted_timeline(&src, &table(&[("w", 4.0)]));
        assert!(
            unresolved.is_empty(),
            "a draft's broken binding must not be reported: {unresolved:?}"
        );
        assert!(matches!(out.state(1), Some(StepState::Dirty)));

        // Move the bar past it and the same binding IS a failure.
        let mut applied = src.clone();
        applied.set_cursor(2);
        let (out, unresolved) = substituted_timeline(&applied, &table(&[("w", 4.0)]));
        assert_eq!(unresolved.len(), 1);
        assert!(matches!(out.state(1), Some(StepState::Error { .. })));
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
