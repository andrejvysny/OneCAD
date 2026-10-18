use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::metamorph;
use crate::prepared::PreparedCase;
use crate::result::{attach_replay, is_red, refresh_digest, Backend};
use crate::runner;
use crate::suite::{GeneratedCase, Variant};
use crate::{AppError, AppResult};

pub fn run_cases(
    runner_path: &Path,
    generated: &[GeneratedCase],
    backends: &[Backend],
    out_dir: &Path,
) -> AppResult<i32> {
    std::fs::create_dir_all(out_dir).map_err(|e| AppError::environment(format!("out dir: {e}")))?;
    let mut records = Vec::with_capacity(generated.len() * backends.len());
    for item in generated {
        for backend in backends {
            records.push(run_repeated(
                runner_path,
                &item.case,
                *backend,
                &item.variant,
                out_dir,
            ));
        }
    }
    apply_metamorph(&mut records, generated);
    apply_differential(&mut records);
    records.iter_mut().for_each(refresh_digest);
    write_jsonl(&out_dir.join("results.jsonl"), &records)?;
    Ok(if records.iter().any(is_red) { 1 } else { 0 })
}

/// The relation a metamorphic variant is expected to preserve.
///
/// Only the rigid variants preserve the shape outright. A uniform scale is a
/// SIMILARITY — the worker scales the geometry and the requested radius by the
/// same factor (`worker/src/benchmark/Execution.cpp` and `Geometry.cpp`), so
/// mass properties are expected to move by `k³`/`k²`, not to stay equal. A
/// parameter epsilon changes the operation itself, so nothing is preserved
/// except CONTINUITY of the response: same classification, and a shape that
/// moves proportionally to the nudge instead of jumping.
///
/// The relation is a pure function of the variant name, which is why it needs
/// no room in the frozen result-v1 `metamorphEvidence` block.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Relation {
    Equivalence,
    Similarity { factor: f64 },
    Continuity { relative_delta: f64 },
}

/// Similarity-compensated properties are recomputed through a power of the
/// factor, so they cannot carry the near-exact agreement the rigid variants do.
/// Measured worst residual over `fillet/matrix:m1`, both backends: 5.6e-12.
const SIMILARITY_RELATIVE_TOLERANCE: f64 = 1e-9;

/// Ceiling on the relative volume and area response to a relative radius nudge.
/// A blend's contribution grows as r², so the response is a small multiple of
/// `|δ|`; measured worst over m1 is 0.037 (volume) and 0.080 (area), while a
/// topological jump is orders of magnitude above it.
const CONTINUITY_VOLUME_COEFFICIENT: f64 = 0.5;

/// Ceiling on how far a surface sample may move under the nudge, in multiples
/// of `radius · |δ|`. The blend's tangency lines move by roughly
/// `radius · δ / tan(θ/2)` on a θ dihedral, so the sharpest pair in the matrix
/// (30°) dominates: measured worst is 3.58, and this covers down to ~14°.
const CONTINUITY_DISPLACEMENT_COEFFICIENT: f64 = 8.0;

/// Below this fraction of the base volume a signed response is numerical noise,
/// so the direction check abstains rather than gating on a rounding error.
const CONTINUITY_DIRECTION_NOISE: f64 = 1e-12;

#[derive(Clone, Copy)]
struct CaseContext {
    point_tolerance: f64,
    max_radius: f64,
}

fn apply_metamorph(records: &mut [Value], generated: &[GeneratedCase]) {
    let contexts: BTreeMap<&str, CaseContext> = generated
        .iter()
        .map(|item| {
            (
                item.case.case_id.as_str(),
                CaseContext {
                    point_tolerance: item.case.point_tolerance,
                    max_radius: item.case.max_radius,
                },
            )
        })
        .collect();
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        groups
            .entry((text(record, "caseId"), text(record, "backend")))
            .or_default()
            .push(index);
    }
    for indices in groups.values() {
        let Some(base) = indices
            .iter()
            .copied()
            .find(|index| variant_name(&records[*index]) == "base")
        else {
            continue;
        };
        let context = contexts
            .get(text(&records[base], "caseId").as_str())
            .copied()
            .unwrap_or(CaseContext {
                point_tolerance: 1e-6,
                max_radius: 0.0,
            });
        for index in indices.iter().copied().filter(|index| *index != base) {
            attach_metamorph(records, base, index, context);
        }
    }
}

fn attach_metamorph(records: &mut [Value], base: usize, variant: usize, context: CaseContext) {
    let relation = relation_of(&records[variant]);
    let status_match =
        text(&records[base], "operationState") == text(&records[variant], "operationState");
    let properties_match = properties_match(&records[base], &records[variant], relation);
    let semantic_match =
        validator_signature(&records[base]) == validator_signature(&records[variant]);
    let tolerance = shape_tolerance(relation, context);
    let evidence = metamorph::compare(&records[base], &records[variant], tolerance);
    let (surface_samples_match, point_classification_match) = evidence
        .as_ref()
        .map(|result| {
            (
                result.surface_samples_match,
                result.point_classification_match,
            )
        })
        // Absence of geometry on both sides (e.g. both variants correctly
        // refused an expected-limit case) is not evidence of a mismatch.
        .unwrap_or((true, true));
    let shape_ok = evidence
        .as_ref()
        .is_none_or(|result| result.surface_samples_match && result.point_classification_match);
    let passed = status_match && properties_match && semantic_match && shape_ok;
    let status = if evidence.is_none() {
        "notRun"
    } else if passed {
        "pass"
    } else {
        "fail"
    };
    let name = variant_name(&records[variant]);
    records[variant]["metamorph"] = json!({
        "variant": name,
        "status": status,
        "normalizedPropertiesMatch": properties_match,
        "semanticEvidenceMatch": semantic_match,
        "surfaceSamplesMatch": surface_samples_match,
        "pointClassificationMatch": point_classification_match,
    });
    if !passed
        && matches!(
            text(&records[variant], "expectedDomain").as_str(),
            "supported" | "expectedLimit"
        )
    {
        records[variant]["verdict"] = json!("fail");
        records[variant]["failureClass"] = json!("semanticFailed");
    }
}

fn relation_of(record: &Value) -> Relation {
    let variant = record.get("campaignVariant");
    match variant_name(record).as_str() {
        "scaled" => variant
            .and_then(|value| value.pointer("/scale/factor"))
            .and_then(Value::as_f64)
            .filter(|factor| factor.is_finite() && *factor > 0.0)
            .map_or(Relation::Equivalence, |factor| Relation::Similarity {
                factor,
            }),
        "parameterEpsilon" => variant
            .and_then(|value| value.pointer("/parameterEpsilon/relativeDelta"))
            .and_then(Value::as_f64)
            .filter(|delta| delta.is_finite() && *delta != 0.0)
            .map_or(Relation::Equivalence, |relative_delta| {
                Relation::Continuity { relative_delta }
            }),
        _ => Relation::Equivalence,
    }
}

fn shape_tolerance(relation: Relation, context: CaseContext) -> f64 {
    match relation {
        Relation::Equivalence | Relation::Similarity { .. } => context.point_tolerance,
        // The blend surface genuinely moves by about `radius · delta`; a shape
        // that stays inside that band responded continuously, one that jumps
        // does not.
        Relation::Continuity { relative_delta } => context
            .point_tolerance
            .max(CONTINUITY_DISPLACEMENT_COEFFICIENT * context.max_radius * relative_delta.abs()),
    }
}

const VOLUME_POINTER: &str = "/outputAudit/massProperties/volume";
const AREA_POINTER: &str = "/outputAudit/massProperties/area";

fn properties_match(base: &Value, variant: &Value, relation: Relation) -> bool {
    match relation {
        Relation::Equivalence => invariant_number_match(base, variant, 1.0, 1.0),
        Relation::Similarity { factor } => {
            invariant_number_match(base, variant, factor.powi(3), factor.powi(2))
        }
        Relation::Continuity { relative_delta } => continuity_match(base, variant, relative_delta),
    }
}

/// Compares mass properties after mapping the base values through the relation's
/// expected exponent. `1.0`/`1.0` is plain equality, which is what every rigid
/// variant asks for.
fn invariant_number_match(
    base: &Value,
    variant: &Value,
    volume_ratio: f64,
    area_ratio: f64,
) -> bool {
    let similarity = volume_ratio != 1.0 || area_ratio != 1.0;
    [(VOLUME_POINTER, volume_ratio), (AREA_POINTER, area_ratio)]
        .iter()
        .all(|(pointer, ratio)| {
            let a = base.pointer(pointer).and_then(Value::as_f64);
            let b = variant.pointer(pointer).and_then(Value::as_f64);
            match (a, b) {
                (Some(a), Some(b)) => {
                    let expected = a * ratio;
                    let allowance = if similarity {
                        (expected.abs() * SIMILARITY_RELATIVE_TOLERANCE).max(1e-7)
                    } else {
                        1e-7_f64.max(expected.abs() * 1e-9)
                    };
                    (expected - b).abs() <= allowance
                }
                (None, None) => true,
                _ => false,
            }
        })
}

/// A parameter nudge is expected to move the result, so this gates the RESPONSE
/// rather than equality: the volume must move by no more than the nudge's own
/// order, and — on the convex edges this matrix builds — a larger blend must
/// remove material, never add it.
fn continuity_match(base: &Value, variant: &Value, relative_delta: f64) -> bool {
    let base_volume = base.pointer(VOLUME_POINTER).and_then(Value::as_f64);
    let variant_volume = variant.pointer(VOLUME_POINTER).and_then(Value::as_f64);
    let base_area = base.pointer(AREA_POINTER).and_then(Value::as_f64);
    let variant_area = variant.pointer(AREA_POINTER).and_then(Value::as_f64);
    let (Some(base_volume), Some(variant_volume)) = (base_volume, variant_volume) else {
        // Absent on both sides is the same "nothing to compare" case the rigid
        // relations treat as no evidence of a mismatch.
        return base_volume.is_none()
            && variant_volume.is_none()
            && base_area.is_none()
            && variant_area.is_none();
    };
    let (Some(base_area), Some(variant_area)) = (base_area, variant_area) else {
        return false;
    };
    if base_volume.abs() <= f64::EPSILON {
        return false;
    }
    let allowance = CONTINUITY_VOLUME_COEFFICIENT * relative_delta.abs();
    let volume_response = (variant_volume - base_volume) / base_volume;
    let area_response = if base_area.abs() > f64::EPSILON {
        (variant_area - base_area) / base_area
    } else {
        return false;
    };
    let bounded = volume_response.abs() <= allowance && area_response.abs() <= allowance;
    let directed = volume_response.abs() <= CONTINUITY_DIRECTION_NOISE
        || volume_response.signum() != relative_delta.signum();
    bounded && directed
}

fn validator_signature(value: &Value) -> Vec<(String, String)> {
    value
        .get("validators")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let kind = item
                .get("kind")
                .or_else(|| item.get("type"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    item.get("passed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        .to_string()
                });
            (kind, status)
        })
        .collect()
}

pub fn run_one(
    runner_path: &Path,
    case: PreparedCase,
    variant: Variant,
    backends: &[Backend],
    out_dir: &Path,
) -> AppResult<i32> {
    run_cases(
        runner_path,
        &[GeneratedCase { case, variant }],
        backends,
        out_dir,
    )
}

fn run_repeated(
    runner_path: &Path,
    case: &PreparedCase,
    backend: Backend,
    variant: &Variant,
    out_dir: &Path,
) -> Value {
    let root = artifact_dir(out_dir, case, backend, variant);
    let canonical_dir = root.join("canonical");
    let replay_dir = root.join("replay");
    let _ = std::fs::create_dir_all(&canonical_dir);
    let _ = std::fs::create_dir_all(&replay_dir);
    let mut canonical = runner::execute(runner_path, case, backend, variant, &canonical_dir);
    let mut replay = runner::execute(runner_path, case, backend, variant, &replay_dir);
    rebase_artifacts(&mut canonical, out_dir, &canonical_dir);
    rebase_artifacts(&mut replay, out_dir, &replay_dir);
    canonical["campaignVariant"] = json!(variant);
    replay["campaignVariant"] = json!(variant);
    refresh_digest(&mut canonical);
    refresh_digest(&mut replay);
    attach_replay(canonical, &replay)
}

pub(crate) fn artifact_dir(
    out_dir: &Path,
    case: &PreparedCase,
    backend: Backend,
    variant: &Variant,
) -> std::path::PathBuf {
    let variant_name = serde_json::to_value(variant.name)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "base".into());
    out_dir
        .join("artifacts")
        .join(&case.case_id)
        .join(variant_name)
        .join(backend.wire())
}

pub(crate) fn rebase_artifacts(value: &mut Value, out_dir: &Path, artifact_dir: &Path) {
    let Some(artifacts) = value.get_mut("artifacts").and_then(Value::as_object_mut) else {
        return;
    };
    for path in artifacts.values_mut() {
        let Some(relative) = path.as_str() else {
            continue;
        };
        let absolute = artifact_dir.join(relative);
        if let Ok(rebased) = absolute.strip_prefix(out_dir) {
            *path = json!(portable_path(rebased));
        }
    }
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn apply_differential(records: &mut [Value]) {
    let mut pairs: BTreeMap<(String, String), (Option<usize>, Option<usize>)> = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        let key = (text(record, "caseId"), variant_name(record));
        let entry = pairs.entry(key).or_default();
        if text(record, "backend") == "raw-occt" {
            entry.0 = Some(index);
        } else {
            entry.1 = Some(index);
        }
    }
    for (_, (raw, onecad)) in pairs {
        let (Some(raw), Some(onecad)) = (raw, onecad) else {
            continue;
        };
        let input_match =
            text(&records[raw], "inputDigest") == text(&records[onecad], "inputDigest");
        let raw_pass = verdict(&records[raw]) == "pass";
        let onecad_pass = verdict(&records[onecad]) == "pass";
        let states_match =
            text(&records[raw], "operationState") == text(&records[onecad], "operationState");
        let supported = text(&records[onecad], "expectedDomain") == "supported";
        let classification = if !input_match {
            "input-mismatch"
        } else if !raw_pass && onecad_pass {
            "rescued"
        } else if supported && raw_pass && !onecad_pass {
            "onecad-regression"
        } else if !states_match || raw_pass != onecad_pass {
            "status-difference"
        } else {
            "same-status"
        };
        records[raw]["differential"] = json!({"classification":classification});
        records[onecad]["differential"] = json!({"classification":classification});
        if classification == "input-mismatch" {
            mark_input_mismatch(&mut records[raw]);
            mark_input_mismatch(&mut records[onecad]);
        } else if classification == "onecad-regression" {
            records[onecad]["verdict"] = json!("fail");
            records[onecad]["failureClass"] = json!("semanticFailed");
        }
    }
}

fn mark_input_mismatch(record: &mut Value) {
    record["verdict"] = json!("fail");
    record["failureClass"] = json!("inputMismatch");
}

fn variant_name(value: &Value) -> String {
    value
        .pointer("/campaignVariant/name")
        .and_then(Value::as_str)
        .unwrap_or("base")
        .to_string()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn verdict(value: &Value) -> &str {
    value
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("fail")
}

pub fn write_jsonl(path: &Path, records: &[Value]) -> AppResult<()> {
    let mut file =
        std::fs::File::create(path).map_err(|e| AppError::environment(format!("results: {e}")))?;
    for record in records {
        serde_json::to_writer(&mut file, record)
            .map_err(|e| AppError::environment(format!("results: {e}")))?;
        file.write_all(b"\n")
            .map_err(|e| AppError::environment(format!("results: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(backend: &str, verdict: &str) -> Value {
        json!({
            "caseId":"case-01",
            "backend":backend,
            "verdict":verdict,
            "failureClass":if verdict == "pass" {"none"} else {"kernelRejected"},
            "expectedDomain":"supported",
            "operationState":if verdict == "pass" {"success"} else {"rejected"},
            "inputDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "campaignVariant":{"name":"base"}
        })
    }

    #[test]
    fn differential_classifies_rescue() {
        let mut records = vec![record("raw-occt", "fail"), record("onecad", "pass")];
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("rescued"))
        );
        assert_eq!(records[1]["verdict"], "pass");
    }

    #[test]
    fn differential_gates_onecad_regression() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "fail")];
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("onecad-regression"))
        );
        assert_eq!(records[1]["failureClass"], "semanticFailed");
    }

    #[test]
    fn exploratory_status_difference_does_not_create_regression() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "fail")];
        records[0]["expectedDomain"] = json!("exploratory");
        records[1]["expectedDomain"] = json!("exploratory");
        records[0]["inputDigest"] = json!("a");
        records[1]["inputDigest"] = json!("a");
        apply_differential(&mut records);
        assert_eq!(
            records[1].pointer("/differential/classification"),
            Some(&json!("status-difference"))
        );
    }

    fn mass(volume: f64, area: f64) -> Value {
        json!({"outputAudit":{"massProperties":{"volume":volume,"area":area}}})
    }

    fn scaled(volume: f64, area: f64, factor: f64) -> Value {
        let mut value = mass(volume, area);
        value["campaignVariant"] = json!({"name":"scaled","scale":{"factor":factor}});
        value
    }

    fn epsilon(volume: f64, area: f64, relative_delta: f64) -> Value {
        let mut value = mass(volume, area);
        value["campaignVariant"] = json!({
            "name":"parameterEpsilon",
            "parameterEpsilon":{"parameter":"operation.radius","relativeDelta":relative_delta}
        });
        value
    }

    #[test]
    fn a_rigid_variant_still_demands_equal_properties() {
        let base = mass(1000.0, 600.0);
        let mut variant = mass(1000.0, 600.0);
        variant["campaignVariant"] = json!({"name":"rotated"});
        assert_eq!(relation_of(&variant), Relation::Equivalence);
        assert!(properties_match(&base, &variant, Relation::Equivalence));
        let moved = mass(1000.001, 600.0);
        assert!(!properties_match(&base, &moved, Relation::Equivalence));
    }

    #[test]
    fn a_uniform_scale_is_compared_through_its_own_exponents() {
        let base = mass(1000.0, 600.0);
        let variant = scaled(8000.0, 2400.0, 2.0);
        assert_eq!(relation_of(&variant), Relation::Similarity { factor: 2.0 });
        assert!(properties_match(&base, &variant, relation_of(&variant)));
        // Equality — what the old gate demanded — is now the failing answer.
        let unscaled = scaled(1000.0, 600.0, 2.0);
        assert!(!properties_match(&base, &unscaled, relation_of(&unscaled)));
    }

    /// The gate this replaces would have passed a kernel that scaled the solid
    /// but left the blend at the original radius, since it only ever compared
    /// the variant with itself. This one does not.
    #[test]
    fn a_similarity_that_skips_the_blend_fails() {
        let base = mass(1000.0, 600.0);
        let variant = scaled(8000.0 * 1.0001, 2400.0, 2.0);
        assert!(!properties_match(&base, &variant, relation_of(&variant)));
    }

    #[test]
    fn a_parameter_nudge_gates_a_bounded_signed_response() {
        let base = mass(1000.0, 600.0);
        let delta = 1e-3;
        assert_eq!(
            relation_of(&epsilon(0.0, 0.0, delta)),
            Relation::Continuity {
                relative_delta: delta
            }
        );
        // A larger blend on a convex edge removes material: small, negative.
        let responded = epsilon(999.8, 599.9, delta);
        assert!(properties_match(&base, &responded, relation_of(&responded)));
        // Gaining volume as the radius grows is not a continuous response.
        let inverted = epsilon(1000.2, 600.1, delta);
        assert!(!properties_match(&base, &inverted, relation_of(&inverted)));
        // Neither is a jump far larger than the nudge.
        let jumped = epsilon(950.0, 590.0, delta);
        assert!(!properties_match(&base, &jumped, relation_of(&jumped)));
    }

    #[test]
    fn only_the_continuity_relation_widens_the_shape_tolerance() {
        let context = CaseContext {
            point_tolerance: 1e-6,
            max_radius: 4.0,
        };
        assert_eq!(shape_tolerance(Relation::Equivalence, context), 1e-6);
        assert_eq!(
            shape_tolerance(Relation::Similarity { factor: 2.0 }, context),
            1e-6
        );
        assert_eq!(
            shape_tolerance(
                Relation::Continuity {
                    relative_delta: 1e-3
                },
                context
            ),
            // 8 x radius x delta, the blend's own displacement scale.
            8.0 * 4.0 * 1e-3
        );
    }

    #[test]
    fn differential_gates_input_mismatch() {
        let mut records = vec![record("raw-occt", "pass"), record("onecad", "pass")];
        records[0]["inputDigest"] = json!("a");
        records[1]["inputDigest"] = json!("b");
        apply_differential(&mut records);
        assert!(records
            .iter()
            .all(|value| value["failureClass"] == "inputMismatch"));
    }
}
