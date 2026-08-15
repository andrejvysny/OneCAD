//! Forward migration of older containers + the version-aware open flow.
//!
//! A [`MigrationRegistry`] chains version-to-version [`MigrationStep`]s applied to
//! the raw `document.json` [`Value`](serde_json::Value) **before** typed
//! deserialization (plan task 5; V1/V2 §9). Steps transform the document at two
//! levels: the whole-document value, and — with `RecordSchemaVersion` awareness —
//! each timeline record value.
//!
//! Open policy (V1/V2 §9.2/§9.3):
//!
//! * **Same version** → read/write after idempotent policy normalizations (legacy
//!   region ids and absent-version Pattern V1 → V2 + downstream repair seeds).
//! * **Older version** → build a [`MigrationPlan`], apply it. If the chain does not
//!   reach the current version, or any step's confidence is below `High`, the
//!   document opens **read-only** with a [`MigrationReport`] (guided report).
//! * **Newer version** → best-effort **read-only**. Unknown ops ride through as
//!   [`Opaque`](crate::document::record::Operation::Opaque) frozen nodes and
//!   unknown fields via `extra` (the record codec already does this), so the
//!   document loads without a transform but is never written back.
//!
//! The version-step registry remains an **empty chain**. Same-version policy
//! normalizations live outside it because they do not change
//! `globalSchemaVersion`, but they still mark records/caches stale and emit
//! migration diagnostics.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::document::Document;

use super::{document_io, Diagnostic, IoError, IoResult};

/// Confidence that a migration produced a faithful document (V1/V2 §9.4). Ordered
/// `Low < Medium < High`; anything below `High` forces a read-only open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MigrationConfidence {
    Low,
    Medium,
    High,
}

/// One version-to-version transform over the raw document JSON.
///
/// A step upgrades a document from [`from_version`](MigrationStep::from_version) to
/// [`to_version`](MigrationStep::to_version). It may rewrite the whole-document
/// value ([`migrate_document`](MigrationStep::migrate_document)) and, if it
/// [`touches_records`](MigrationStep::touches_records), each timeline record value
/// ([`migrate_record`](MigrationStep::migrate_record) — `RecordSchemaVersion`-aware).
// `from_version`/`to_version` describe the version range this step spans; they are
// plain accessors, not constructors — the `wrong_self_convention` lint's
// `from_*`-takes-no-self heuristic does not apply.
#[allow(clippy::wrong_self_convention)]
pub trait MigrationStep: Send + Sync {
    /// The `globalSchemaVersion` this step upgrades **from**.
    fn from_version(&self) -> u32;
    /// The `globalSchemaVersion` this step upgrades **to** (must exceed
    /// `from_version`).
    fn to_version(&self) -> u32;
    /// Confidence this step preserves document semantics.
    fn confidence(&self) -> MigrationConfidence;
    /// Rewrites the whole-document value in place.
    ///
    /// # Errors
    /// A human-facing message if the transform cannot be applied.
    fn migrate_document(&self, document: &mut Value) -> Result<(), String>;
    /// Whether this step also transforms individual timeline records.
    fn touches_records(&self) -> bool {
        false
    }
    /// Rewrites a single timeline record value in place (only called when
    /// [`touches_records`](MigrationStep::touches_records) is `true`). The record's
    /// `recordSchemaVersion` is available in `record["recordSchemaVersion"]`.
    ///
    /// # Errors
    /// A human-facing message if the record cannot be transformed.
    fn migrate_record(&self, record: &mut Value) -> Result<(), String> {
        let _ = record;
        Ok(())
    }
    /// Free-form notes surfaced in the [`MigrationReport`] (e.g. lossy fields).
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Read-only metadata about one planned step (no trait object — `Clone`/`Debug`
/// for the report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepMeta {
    pub from_version: u32,
    pub to_version: u32,
    pub confidence: MigrationConfidence,
}

/// A planned migration chain from a stored version up toward the current version
/// (V1/V2 §9.4 `MigrationPlan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    /// The stored document's `globalSchemaVersion`.
    pub from_version: u32,
    /// The version this build targets.
    pub target_version: u32,
    /// The ordered chain of steps (empty when already current).
    pub steps: Vec<StepMeta>,
    /// Minimum confidence across the chain (`Low` if the chain is incomplete).
    pub confidence: MigrationConfidence,
    /// Whether any step transforms individual records.
    pub per_record: bool,
    /// Whether the chain reaches [`target_version`](MigrationPlan::target_version).
    pub complete: bool,
    /// Guided-report notes (missing-step gaps, per-step notes).
    pub notes: Vec<String>,
}

impl MigrationPlan {
    /// Whether applying this plan yields a fully-current, high-confidence document
    /// (⇒ the document may open read/write).
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.complete && self.confidence == MigrationConfidence::High
    }
}

/// The migration outcome attached to a [`LoadOutcome`] (V1/V2 §9.4 guided report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// The plan that was built (older-version open) — `None` slot unused for the
    /// newer-version case, which carries no plan.
    pub plan: Option<MigrationPlan>,
    /// Whether the transform chain was applied to the document value.
    pub applied: bool,
    /// Why the document was forced read-only, if it was.
    pub read_only_reason: Option<String>,
}

/// The result of opening a document from a container (plan task 5).
#[derive(Debug)]
pub struct LoadOutcome {
    /// The loaded (and, if older, migrated) document.
    pub document: Document,
    /// Whether the document is read-only (newer schema, or a low-confidence /
    /// incomplete migration).
    pub read_only: bool,
    /// Migration detail, when a migration was considered.
    pub migration_report: Option<MigrationReport>,
    /// Whether the container's caches are stale w.r.t. the loaded records
    /// (`opsHash` mismatch, or a migration changed the records). Caches must not be
    /// used when set (Invariant 7).
    pub stale_caches: bool,
    /// Non-fatal load observations (ops.jsonl divergence, migration notes, …).
    pub diagnostics: Vec<Diagnostic>,
}

/// An ordered registry of [`MigrationStep`]s keyed by `from_version`.
#[derive(Default)]
pub struct MigrationRegistry {
    steps: BTreeMap<u32, Box<dyn MigrationStep>>,
}

impl std::fmt::Debug for MigrationRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationRegistry")
            .field("from_versions", &self.steps.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MigrationRegistry {
    /// An empty registry (the v2.0 default — no legacy chain yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a step, keyed by its `from_version`. A later registration at the
    /// same `from_version` replaces the earlier one.
    pub fn register(&mut self, step: Box<dyn MigrationStep>) {
        self.steps.insert(step.from_version(), step);
    }

    /// Builds a [`MigrationPlan`] from `from_version` up to `target_version`
    /// (pure metadata; does not mutate anything).
    #[must_use]
    pub fn plan(&self, from_version: u32, target_version: u32) -> MigrationPlan {
        let mut steps = Vec::new();
        let mut notes = Vec::new();
        let mut per_record = false;
        let mut cur = from_version;
        let mut complete = true;
        while cur < target_version {
            match self.steps.get(&cur) {
                Some(step) if step.to_version() > cur => {
                    steps.push(StepMeta {
                        from_version: step.from_version(),
                        to_version: step.to_version(),
                        confidence: step.confidence(),
                    });
                    notes.extend(step.notes());
                    per_record |= step.touches_records();
                    cur = step.to_version();
                }
                Some(_) => {
                    notes.push(format!(
                        "migration step from schema version {cur} does not advance the version"
                    ));
                    complete = false;
                    break;
                }
                None => {
                    notes.push(format!(
                        "no migration registered from schema version {cur} (target {target_version})"
                    ));
                    complete = false;
                    break;
                }
            }
        }
        let confidence = if complete {
            steps
                .iter()
                .map(|s| s.confidence)
                .min()
                .unwrap_or(MigrationConfidence::High)
        } else {
            MigrationConfidence::Low
        };
        MigrationPlan {
            from_version,
            target_version,
            steps,
            confidence,
            per_record,
            complete,
            notes,
        }
    }

    /// Applies the registered chain to `value`, transforming it in place from
    /// `from_version` toward `target_version` (stops where the chain runs out).
    ///
    /// # Errors
    /// [`IoError::Corrupt`] if a step's transform fails.
    pub fn apply(&self, value: &mut Value, from_version: u32, target_version: u32) -> IoResult<()> {
        let mut cur = from_version;
        while cur < target_version {
            let step = match self.steps.get(&cur) {
                Some(s) if s.to_version() > cur => s,
                _ => break,
            };
            step.migrate_document(value).map_err(|e| {
                IoError::Corrupt(format!(
                    "migration {}→{}: {e}",
                    step.from_version(),
                    step.to_version()
                ))
            })?;
            if step.touches_records() {
                if let Some(records) = value
                    .pointer_mut("/timeline/records")
                    .and_then(Value::as_array_mut)
                {
                    for record in records.iter_mut() {
                        step.migrate_record(record).map_err(|e| {
                            IoError::Corrupt(format!(
                                "migration {}→{} (record): {e}",
                                step.from_version(),
                                step.to_version()
                            ))
                        })?;
                    }
                }
            }
            cur = step.to_version();
        }
        Ok(())
    }
}

/// The migrated document value plus the derived open policy (before typed
/// deserialization). Consumed by the container reader.
#[derive(Debug)]
pub struct MigratedValue {
    /// The (possibly transformed) document JSON, ready for typed decode.
    pub value: Value,
    /// Whether the document must open read-only.
    pub read_only: bool,
    /// Whether a migration changed the records (⇒ caches are stale regardless of
    /// `opsHash`).
    pub records_changed: bool,
    /// Migration detail for the report.
    pub report: Option<MigrationReport>,
    /// Diagnostics to fold into the load outcome.
    pub diagnostics: Vec<Diagnostic>,
}

/// Whether `s` is a normative region id (SCHEMA §7.4: `"r_"` + 16 lowercase hex).
fn is_normative_region_id(s: &str) -> bool {
    match s.strip_prefix("r_") {
        Some(hex) => {
            hex.len() == 16
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

/// Load-time normalization (M4a): a pre-M4a document may persist a **legacy
/// placeholder** `regionId` (e.g. `"r0"`, `"region-legacy-1"`) that, under the new
/// strict region rule ([`SCHEMA §7.4`]), would `OP_FAILED` forever with no path out
/// (a non-empty id that matches no detected region is a hard failure). Any
/// Extrude/Revolve/Sweep `params.profile.regionId` that is **non-empty** and NOT of
/// the normative `^r_[0-9a-f]{16}$` form is rewritten to `""` (the first-region V1
/// fallback), with a load diagnostic. A valid id — or an already-empty one — is left
/// untouched. This is a same-version normalization (M4a did not bump the schema), so
/// it runs on every writable open, not as a version-bump [`MigrationStep`].
///
/// Returns the diagnostics (empty ⇒ nothing changed).
fn sanitize_region_ids(value: &mut Value) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let Some(records) = value
        .pointer_mut("/timeline/records")
        .and_then(Value::as_array_mut)
    else {
        return diags;
    };
    for rec in records.iter_mut() {
        let op_type = rec
            .get("opType")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !matches!(op_type.as_str(), "Extrude" | "Revolve" | "Sweep") {
            continue;
        }
        let Some(region) = rec.pointer_mut("/params/profile/regionId") else {
            continue;
        };
        let Some(id) = region.as_str() else {
            continue;
        };
        if id.is_empty() || is_normative_region_id(id) {
            continue;
        }
        let old = id.to_string();
        *region = Value::String(String::new());
        diags.push(Diagnostic::warning(
            "region-id-legacy-reset",
            format!(
                "legacy non-normative regionId {old:?} on a {op_type} op was reset to \"\" \
                 (first-region fallback); re-select the region to pin it"
            ),
        ));
    }
    diags
}

fn value_contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text == needle,
        Value::Array(values) => values.iter().any(|v| value_contains_string(v, needle)),
        Value::Object(values) => values.values().any(|v| value_contains_string(v, needle)),
        _ => false,
    }
}

/// Same-version policy migration for absent-version Pattern V1 records. V2 changes
/// body identity (one aggregate becomes modified-source or ordinal children), so
/// every downstream reference to the retired aggregate is seeded NeedsRepair.
/// No child is selected automatically.
fn migrate_absent_pattern_result_policies(value: &mut Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut retired_bodies = Vec::<String>::new();
    let mut repair_seeds = Vec::<Value>::new();

    {
        let Some(records) = value
            .pointer_mut("/timeline/records")
            .and_then(Value::as_array_mut)
        else {
            return diagnostics;
        };

        for index in 0..records.len() {
            let op_type = records[index]
                .get("opType")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if !matches!(op_type.as_str(), "LinearPattern" | "CircularPattern") {
                continue;
            }
            let already_versioned = records[index]
                .pointer("/params/resultPolicyVersion")
                .is_some();
            if already_versioned {
                continue;
            }
            let Some(pattern_id) = records[index]
                .get("recordId")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                diagnostics.push(Diagnostic::warning(
                    "pattern-result-policy-v2-migration-skipped",
                    "an absent-version Pattern has no readable recordId; left unchanged",
                ));
                continue;
            };

            let downstream: Vec<(usize, usize, String)> = records
                .iter()
                .enumerate()
                .skip(index + 1)
                .filter(|(_, record)| value_contains_string(record, &pattern_id))
                .map(|(position, record)| {
                    let step = record
                        .get("stepIndex")
                        .and_then(Value::as_u64)
                        .and_then(|v| usize::try_from(v).ok())
                        .unwrap_or(position);
                    let record_id = record
                        .get("recordId")
                        .and_then(Value::as_str)
                        .unwrap_or("downstream-op")
                        .to_owned();
                    (position, step, record_id)
                })
                .collect();

            let Some(params) = records[index]
                .get_mut("params")
                .and_then(Value::as_object_mut)
            else {
                diagnostics.push(Diagnostic::warning(
                    "pattern-result-policy-v2-migration-skipped",
                    format!("Pattern {pattern_id} has no params object; left unchanged"),
                ));
                continue;
            };
            params.insert("resultPolicyVersion".into(), Value::from(2u8));
            if let Some(outputs) = records[index]
                .get_mut("outputs")
                .and_then(Value::as_array_mut)
            {
                outputs.clear();
            }
            retired_bodies.push(pattern_id.clone());
            diagnostics.push(Diagnostic::info(
                "pattern-result-policy-v2-migrated",
                format!("migrated {op_type} {pattern_id} from absent V1 to V2"),
            ));

            for (_position, step, downstream_id) in downstream {
                repair_seeds.push(serde_json::json!({
                    "stepIndex": step,
                    "refId": format!("{downstream_id}.patternV1Aggregate"),
                    "ladderFailed": "descriptor",
                    "reason": "no-candidates",
                    "candidates": [],
                    "uiLabel": format!(
                        "Pattern V1 aggregate {pattern_id} migrated to V2 outputs; reselect this body reference"
                    ),
                    "seeded": true
                }));
            }
        }
    }

    if retired_bodies.is_empty() {
        return diagnostics;
    }

    if let Some(bodies) = value
        .pointer_mut("/bodies/bodies")
        .and_then(Value::as_array_mut)
    {
        bodies.retain(|body| {
            body.get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !retired_bodies.iter().any(|retired| retired == id))
        });
    }
    if let Some(elements) = value.get_mut("elements").and_then(Value::as_object_mut) {
        elements.retain(|_, entry| {
            entry
                .get("body")
                .and_then(Value::as_str)
                .is_none_or(|id| !retired_bodies.iter().any(|retired| retired == id))
        });
    }
    if !repair_seeds.is_empty() {
        if !value.get("repair").is_some_and(Value::is_array) {
            value["repair"] = Value::Array(Vec::new());
        }
        if let Some(repair) = value.get_mut("repair").and_then(Value::as_array_mut) {
            for seed in repair_seeds {
                let duplicate = repair.iter().any(|item| {
                    item.get("stepIndex") == seed.get("stepIndex")
                        && item.get("refId") == seed.get("refId")
                });
                if !duplicate {
                    repair.push(seed);
                }
            }
        }
    }
    diagnostics
}

/// Runs the version-aware open policy over a raw document value (plan task 5).
///
/// * `stored_version` is the manifest's `globalSchemaVersion`.
/// * `target_version` is [`GLOBAL_SCHEMA_VERSION`](super::manifest::GLOBAL_SCHEMA_VERSION).
///
/// Never fails on a *policy* decision (read-only is an outcome, not an error);
/// only a hard transform failure yields [`IoError::Corrupt`].
pub fn open_policy(
    registry: &MigrationRegistry,
    mut value: Value,
    stored_version: u32,
    target_version: u32,
) -> IoResult<MigratedValue> {
    use std::cmp::Ordering;
    match stored_version.cmp(&target_version) {
        Ordering::Equal => {
            // Same version: still normalize legacy placeholder region ids (M4a).
            let mut diagnostics = sanitize_region_ids(&mut value);
            diagnostics.extend(migrate_absent_pattern_result_policies(&mut value));
            let records_changed = !diagnostics.is_empty();
            Ok(MigratedValue {
                value,
                read_only: false,
                records_changed,
                report: None,
                diagnostics,
            })
        }
        Ordering::Greater => {
            // Newer file: best-effort read-only, no transform (Opaque ride-through).
            let diag = Diagnostic::warning(
                "newer-schema-read-only",
                format!(
                    "document schema version {stored_version} is newer than this build's \
                     {target_version}; opened read-only (best effort)"
                ),
            );
            Ok(MigratedValue {
                value,
                read_only: true,
                records_changed: false,
                report: Some(MigrationReport {
                    plan: None,
                    applied: false,
                    read_only_reason: Some(diag.message.clone()),
                }),
                diagnostics: vec![diag],
            })
        }
        Ordering::Less => {
            let plan = registry.plan(stored_version, target_version);
            registry.apply(&mut value, stored_version, target_version)?;
            let applied = !plan.steps.is_empty();
            let read_only = !plan.is_lossless();
            let read_only_reason = read_only.then(|| {
                if !plan.complete {
                    format!(
                        "migration from schema version {stored_version} could not reach \
                         {target_version}; opened read-only"
                    )
                } else {
                    format!(
                        "migration from schema version {stored_version} is {:?}-confidence; \
                         opened read-only",
                        plan.confidence
                    )
                }
            });
            // Also normalize legacy placeholder region ids on the migrated value (M4a).
            let region_diags = sanitize_region_ids(&mut value);
            let pattern_diags = migrate_absent_pattern_result_policies(&mut value);
            let records_changed = applied || !region_diags.is_empty() || !pattern_diags.is_empty();
            let mut diagnostics = Vec::new();
            if applied {
                diagnostics.push(Diagnostic::info(
                    "migration-applied",
                    format!(
                        "migrated document from schema version {stored_version} to {} \
                         ({} step(s), {:?} confidence)",
                        plan.steps.last().map_or(stored_version, |s| s.to_version),
                        plan.steps.len(),
                        plan.confidence
                    ),
                ));
            }
            diagnostics.extend(region_diags);
            diagnostics.extend(pattern_diags);
            if let Some(reason) = &read_only_reason {
                diagnostics.push(Diagnostic::warning("migration-read-only", reason.clone()));
            }
            Ok(MigratedValue {
                value,
                read_only,
                records_changed,
                report: Some(MigrationReport {
                    plan: Some(plan),
                    applied,
                    read_only_reason,
                }),
                diagnostics,
            })
        }
    }
}

/// Convenience: run [`open_policy`] and then typed-decode the value into a
/// [`Document`], returning both the document and the policy metadata.
pub(crate) fn migrate_and_decode(
    registry: &MigrationRegistry,
    value: Value,
    stored_version: u32,
    target_version: u32,
) -> IoResult<(Document, MigratedValue)> {
    let migrated = open_policy(registry, value, stored_version, target_version)?;
    let document = document_io::document_from_value(migrated.value.clone())?;
    Ok((document, migrated))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic v0→v1 step: renames a top-level document field. High confidence.
    struct RenameStep;
    impl MigrationStep for RenameStep {
        fn from_version(&self) -> u32 {
            0
        }
        fn to_version(&self) -> u32 {
            1
        }
        fn confidence(&self) -> MigrationConfidence {
            MigrationConfidence::High
        }
        fn migrate_document(&self, document: &mut Value) -> Result<(), String> {
            if let Some(obj) = document.as_object_mut() {
                if let Some(v) = obj.remove("oldName") {
                    obj.insert("newName".into(), v);
                }
                obj.insert("schemaVersion".into(), Value::from(1u32));
            }
            Ok(())
        }
        fn notes(&self) -> Vec<String> {
            vec!["renamed oldName → newName".into()]
        }
    }

    /// A synthetic low-confidence step (forces read-only) that also touches records.
    struct LossyStep;
    impl MigrationStep for LossyStep {
        fn from_version(&self) -> u32 {
            0
        }
        fn to_version(&self) -> u32 {
            1
        }
        fn confidence(&self) -> MigrationConfidence {
            MigrationConfidence::Low
        }
        fn migrate_document(&self, _document: &mut Value) -> Result<(), String> {
            Ok(())
        }
        fn touches_records(&self) -> bool {
            true
        }
        fn migrate_record(&self, record: &mut Value) -> Result<(), String> {
            if let Some(obj) = record.as_object_mut() {
                obj.insert("migrated".into(), Value::Bool(true));
            }
            Ok(())
        }
    }

    #[test]
    fn empty_registry_same_version_is_read_write() {
        let reg = MigrationRegistry::new();
        let out = open_policy(&reg, serde_json::json!({"schemaVersion":1}), 1, 1).unwrap();
        assert!(!out.read_only);
        assert!(out.report.is_none());
    }

    #[test]
    fn newer_version_opens_read_only_without_transform() {
        let reg = MigrationRegistry::new();
        let v = serde_json::json!({"schemaVersion":5});
        let out = open_policy(&reg, v.clone(), 5, 1).unwrap();
        assert!(out.read_only);
        assert_eq!(out.value, v); // untouched
        assert_eq!(out.diagnostics[0].code, "newer-schema-read-only");
    }

    #[test]
    fn high_confidence_chain_migrates_read_write() {
        let mut reg = MigrationRegistry::new();
        reg.register(Box::new(RenameStep));
        let plan = reg.plan(0, 1);
        assert!(plan.is_lossless());
        let out = open_policy(&reg, serde_json::json!({"oldName":7}), 0, 1).unwrap();
        assert!(!out.read_only);
        assert!(out.records_changed);
        assert_eq!(out.value["newName"], serde_json::json!(7));
    }

    #[test]
    fn low_confidence_chain_forces_read_only_and_touches_records() {
        let mut reg = MigrationRegistry::new();
        reg.register(Box::new(LossyStep));
        let plan = reg.plan(0, 1);
        assert!(!plan.is_lossless());
        assert!(plan.per_record);
        let v = serde_json::json!({"timeline":{"records":[{"recordSchemaVersion":1}]}});
        let out = open_policy(&reg, v, 0, 1).unwrap();
        assert!(out.read_only);
        assert_eq!(
            out.value["timeline"]["records"][0]["migrated"],
            Value::Bool(true)
        );
    }

    #[test]
    fn legacy_region_id_is_reset_on_load_with_diagnostic() {
        // M4a NOTE-3: a same-version pre-M4a doc with a legacy placeholder regionId
        // (would OP_FAILED forever under the strict rule) is reset to "" on load.
        let v = serde_json::json!({
            "timeline": { "records": [
                { "opType": "Extrude", "params": { "profile": { "sketchId": "sk", "regionId": "r0" } } },
                { "opType": "Revolve", "params": { "profile": { "regionId": "region-legacy-1" } } },
                { "opType": "Extrude", "params": { "profile": { "regionId": "r_0123456789abcdef" } } },
                { "opType": "Extrude", "params": { "profile": { "regionId": "" } } },
                { "opType": "Boolean", "params": { "targetBodyId": "b" } },
            ]}
        });
        let out = open_policy(&MigrationRegistry::new(), v, 1, 1).unwrap();
        let recs = &out.value["timeline"]["records"];
        assert_eq!(recs[0]["params"]["profile"]["regionId"], "", "\"r0\" reset");
        assert_eq!(
            recs[1]["params"]["profile"]["regionId"], "",
            "\"region-legacy-1\" reset"
        );
        assert_eq!(
            recs[2]["params"]["profile"]["regionId"], "r_0123456789abcdef",
            "a valid normative id is UNTOUCHED"
        );
        assert_eq!(
            recs[3]["params"]["profile"]["regionId"], "",
            "empty stays empty"
        );
        assert!(out.records_changed, "a rewrite marks caches stale");
        let n = out
            .diagnostics
            .iter()
            .filter(|d| d.code == "region-id-legacy-reset")
            .count();
        assert_eq!(n, 2, "one diagnostic per reset (the two placeholders)");
    }

    #[test]
    fn absent_patterns_migrate_to_v2_and_seed_downstream_repair() {
        let value = serde_json::json!({
            "timeline": { "records": [
                { "recordId": "pattern-1", "stepIndex": 0, "opType": "LinearPattern",
                  "params": { "sourceBodyId": "source", "count": 3, "fuseResult": false },
                  "outputs": ["pattern-1"] },
                { "recordId": "consumer-1", "stepIndex": 1, "opType": "Hole",
                  "params": { "targetBodyId": "pattern-1" },
                  "inputs": { "bodies": ["pattern-1"] } },
                { "recordId": "pattern-2", "stepIndex": 2, "opType": "CircularPattern",
                  "params": { "resultPolicyVersion": 2 } },
                { "recordId": "pattern-3", "stepIndex": 3, "opType": "LinearPattern",
                  "params": { "resultPolicyVersion": 3 } },
                { "recordId": "hole-1", "stepIndex": 4, "opType": "Hole", "params": {} }
            ]},
            "bodies": { "bodies": [
                { "id": "pattern-1", "name": "Legacy aggregate", "visible": true,
                  "createdBy": "pattern-1" },
                { "id": "source", "name": "Source", "visible": true,
                  "createdBy": "source" }
            ]},
            "elements": { "el_old": { "body": "pattern-1", "kind": "face" } },
            "repair": []
        });
        let out = open_policy(&MigrationRegistry::new(), value, 1, 1).unwrap();
        let records = &out.value["timeline"]["records"];
        assert_eq!(records[0]["params"]["resultPolicyVersion"], 2);
        assert_eq!(records[0]["outputs"], serde_json::json!([]));
        assert_eq!(records[2]["params"]["resultPolicyVersion"], 2);
        assert_eq!(records[3]["params"]["resultPolicyVersion"], 3);
        assert!(records[4]["params"].get("resultPolicyVersion").is_none());
        assert!(out.value["bodies"]["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|body| body["id"] != "pattern-1"));
        assert!(out.value["elements"].get("el_old").is_none());
        assert_eq!(out.value["repair"][0]["stepIndex"], 1);
        assert_eq!(out.value["repair"][0]["reason"], "no-candidates");
        assert_eq!(out.value["repair"][0]["seeded"], true);
        assert!(out.records_changed);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.code == "pattern-result-policy-v2-migrated"));

        let second = open_policy(&MigrationRegistry::new(), out.value, 1, 1).unwrap();
        assert!(!second.records_changed, "migration is idempotent");
        assert_eq!(second.value["repair"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn valid_region_id_predicate() {
        assert!(is_normative_region_id("r_0123456789abcdef"));
        assert!(is_normative_region_id("r_ffffffffffffffff"));
        assert!(!is_normative_region_id("r0"));
        assert!(!is_normative_region_id("region-legacy-1"));
        assert!(
            !is_normative_region_id("r_0123456789ABCDEF"),
            "uppercase rejected"
        );
        assert!(!is_normative_region_id("r_0123"), "wrong length rejected");
        assert!(!is_normative_region_id(""));
    }

    #[test]
    fn incomplete_chain_is_low_confidence_read_only() {
        // Registry can only go 0→1 but the file is v0 and target is v2 (gap 1→2).
        let mut reg = MigrationRegistry::new();
        reg.register(Box::new(RenameStep));
        let plan = reg.plan(0, 2);
        assert!(!plan.complete);
        assert_eq!(plan.confidence, MigrationConfidence::Low);
        let out = open_policy(&reg, serde_json::json!({"oldName":1}), 0, 2).unwrap();
        assert!(out.read_only);
    }
}
