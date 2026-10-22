//! Recoverable profile refusals remove fills, never the accepted sketch or drawable scene.
use crate::{
    bridge::Backend,
    profiles::{Profile, ProfileBudget, decode_profiles_budgeted},
};
use onecad_app::{api, dto::DocumentProjection, error::ApiError};
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord},
        refs::SketchRegionRef,
    },
    ids::SketchId,
    regen::Diagnostic,
};
use onecad_scene::DVec3;

#[derive(Debug)]
pub struct Failure {
    pub sketch: SketchId,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Hidden profile geometry is retained only for authoring; never rendered or picked.
pub struct Anchor {
    pub reference: SketchRegionRef,
    pub base: DVec3,
    pub normal: DVec3,
    pub triangle: [DVec3; 3],
}
#[derive(Default)]
pub struct Captured {
    pub profiles: Vec<Profile>,
    pub anchors: Vec<Anchor>,
    pub failures: Vec<Failure>,
}
fn referenced(records: &[OperationRecord]) -> Vec<&SketchRegionRef> {
    records
        .iter()
        .filter_map(|record| match &record.op {
            Operation::Known(KnownOperation::Extrude(p)) => p.profile.as_ref(),
            Operation::Known(KnownOperation::Revolve(p)) => p.profile.as_ref(),
            _ => None,
        })
        .collect()
}
fn anchor(profile: &Profile) -> Option<Anchor> {
    let triangle = profile.triangles.iter().find_map(|t| {
        let [a, b, c] = t.map(|i| profile.vertices[i as usize]);
        ((b - a).cross(c - a).length_squared() > 1e-20).then_some([a, b, c])
    })?;
    let normal = profile.plane().normal;
    Some(Anchor {
        reference: profile.reference.clone(),
        base: profile.centroid(),
        normal: DVec3::new(normal.x, normal.y, normal.z).try_normalize()?,
        triangle,
    })
}
pub async fn load(
    backend: &Backend,
    projection: &DocumentProjection,
    records: &[OperationRecord],
) -> Result<Captured, String> {
    let references = referenced(records);
    let mut captured = Captured::default();
    let mut profile_budget = ProfileBudget::default();
    let mut evidence_budget = EvidenceBudget::default();
    for sketch in projection.sketches.values().filter(|sketch| {
        sketch.visible || references.iter().any(|r| r.sketch.to_string() == sketch.id)
    }) {
        let id: SketchId = sketch
            .id
            .parse()
            .map_err(|error| format!("sketch id: {error}"))?;
        let plane = api::get_sketch_plane(&backend.state, id)
            .await
            .map_err(|error| error.to_string())?;
        match api::get_sketch_regions(&backend.state, sketch.id.clone()).await {
            Ok(regions) => {
                let profiles = decode_profiles_budgeted(id, plane, regions, &mut profile_budget)?;
                if sketch.visible {
                    captured.profiles.extend(profiles);
                } else {
                    retain_hidden(
                        &mut captured.anchors,
                        &profiles,
                        &references,
                        &mut profile_budget,
                    )?;
                }
            }
            Err(error) => {
                let failure = recoverable(id, error)?;
                if captured.failures.len() >= 64 {
                    return Err("Native profile refusal sketch count exceeded".into());
                }
                evidence_budget.failure(&failure)?;
                captured.failures.push(failure);
            }
        }
    }
    Ok(captured)
}

fn retain_hidden(
    anchors: &mut Vec<Anchor>,
    profiles: &[Profile],
    references: &[&SketchRegionRef],
    budget: &mut ProfileBudget,
) -> Result<(), String> {
    for profile in profiles.iter().filter(|p| {
        references
            .iter()
            .any(|r| r.sketch == p.reference.sketch && r.region == p.reference.region)
    }) {
        if let Some(anchor) = anchor(profile) {
            // Four slots cover Vec growth, including its initial allocation; cloned refs have no extra map entries.
            let bytes = (4 * std::mem::size_of::<Anchor>())
                .checked_add(anchor.reference.region.0.capacity())
                .ok_or("profile anchor metadata overflow")?;
            budget.retain_metadata(bytes)?;
            anchors.push(anchor);
        }
    }
    Ok(())
}

fn recoverable(sketch: SketchId, error: ApiError) -> Result<Failure, String> {
    let ApiError::OpFailed {
        message,
        diagnostics,
    } = error
    else {
        return Err(error.to_string());
    };
    let known = diagnostics.iter().any(known_profile_error);
    let invalid = diagnostics.len() > 64
        || diagnostics.iter().any(|diagnostic| {
            (diagnostic.severity == onecad_core::regen::Severity::Error
                && !known_profile_error(diagnostic))
                || diagnostic.reason_code.as_deref() == Some("SKETCH_PROFILE_LIMIT_EXCEEDED")
                || matches!(
                    diagnostic.code.as_str(),
                    "RESOURCE_LIMIT" | "PROTOCOL_ERROR" | "WORKER_CRASH" | "STALE_PREVIEW"
                )
        });
    if !known || invalid {
        return Err(format!("operation failed: {message}"));
    }
    Ok(Failure {
        sketch,
        message: format!("operation failed: {message}"),
        diagnostics,
    })
}

fn known_profile_error(diagnostic: &Diagnostic) -> bool {
    diagnostic.severity == onecad_core::regen::Severity::Error
        && diagnostic.code == "OP_FAILED"
        && diagnostic.stage.as_deref() == Some("profile")
        && matches!(
            diagnostic.reason_code.as_deref(),
            Some(
                "SKETCH_PROFILE_OVERLAPPING_CURVES"
                    | "SKETCH_PROFILE_DISCONTINUOUS"
                    | "SKETCH_PROFILE_REFINEMENT_FAILED"
                    | "SKETCH_PROFILE_GRAPH_BUILD_FAILED"
            )
        )
}

#[derive(Default)]
struct EvidenceBudget {
    bytes: usize,
}

impl EvidenceBudget {
    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        crate::budget::charge(
            &mut self.bytes,
            bytes,
            1 << 20,
            "profile diagnostic metadata",
        )
    }

    fn text(&mut self, text: &str) -> Result<(), String> {
        self.charge(
            text.len()
                .checked_add(64)
                .ok_or("profile diagnostic metadata overflow")?,
        )
    }

    fn failure(&mut self, failure: &Failure) -> Result<(), String> {
        self.charge(128)?;
        self.text(&failure.message)?;
        for diagnostic in &failure.diagnostics {
            self.charge(128)?;
            self.text(&diagnostic.code)?;
            self.text(&diagnostic.message)?;
            for text in [&diagnostic.stage, &diagnostic.reason_code]
                .into_iter()
                .flatten()
            {
                self.text(text)?;
            }
            if let Some(evidence) = &diagnostic.evidence {
                self.value(evidence, 0)?;
            }
        }
        Ok(())
    }

    fn value(&mut self, value: &serde_json::Value, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("Native profile diagnostic nesting exceeded".into());
        }
        self.charge(128)?;
        match value {
            serde_json::Value::String(text) => self.text(text)?,
            serde_json::Value::Array(values) => {
                for value in values {
                    self.value(value, depth + 1)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    self.text(key)?;
                    self.value(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::regen::Severity;
    fn error(reason: &str) -> ApiError {
        ApiError::OpFailed {
            message: "SketchRegions: overlap refused".into(),
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                code: "OP_FAILED".into(),
                message: "two sources overlap".into(),
                stage: Some("profile".into()),
                reason_code: Some(reason.into()),
                evidence: None,
            }],
        }
    }
    #[test]
    fn hidden_anchor_allocations_share_the_existing_profile_reservation() {
        let profiles = crate::profiles::decode_profiles(
            SketchId::new(),
            onecad_core::sketch::SketchPlane::xy(),
            onecad_app::dto::FinishSketchDto {
                region_identity_version: 3,
                diagnostics: vec![],
                regions: vec![onecad_app::dto::SketchRegionDto {
                    region_id: "cell-v3:anchor-budget".into(),
                    outer_loop: vec![],
                    holes: vec![],
                    preview_triangles: Some(onecad_app::dto::PreviewTrianglesDto {
                        positions: vec![0.0, 0.0, 10.0, 0.0, 0.0, 10.0],
                        indices: vec![0, 1, 2],
                        holes_subtracted: 0,
                    }),
                }],
            },
        )
        .unwrap();
        let references = vec![&profiles[0].reference];
        let mut anchors = Vec::new();
        let mut budget = ProfileBudget::default();
        retain_hidden(&mut anchors, &profiles, &references, &mut budget).unwrap();
        assert_eq!(anchors.len(), 1);
        assert!(
            budget
                .retain_metadata(crate::budget::MAX_PROFILE_BYTES)
                .is_err()
        );
        let mut full = ProfileBudget::default();
        full.retain_metadata(crate::budget::MAX_PROFILE_BYTES)
            .unwrap();
        let mut refused = Vec::new();
        assert!(retain_hidden(&mut refused, &profiles, &references, &mut full).is_err());
        assert!(refused.is_empty(), "reserve before retaining allocation");
    }
    #[test]
    fn only_known_profile_formation_refusals_can_omit_fills() {
        let id = SketchId::new();
        let failure = recoverable(id, error("SKETCH_PROFILE_OVERLAPPING_CURVES")).unwrap();
        assert_eq!(failure.sketch, id);
        assert_eq!(
            failure.diagnostics[0].reason_code.as_deref(),
            Some("SKETCH_PROFILE_OVERLAPPING_CURVES")
        );
        for error in [
            error("SKETCH_PROFILE_LIMIT_EXCEEDED"),
            error("unknown"),
            ApiError::Worker("protocol failure".into()),
            ApiError::InvalidCommand("invalid reference".into()),
            ApiError::StalePreview("changed source".into()),
        ] {
            assert!(recoverable(id, error).is_err());
        }
    }
    #[test]
    fn profile_evidence_is_bounded_before_display_retention() {
        let mut failure =
            recoverable(SketchId::new(), error("SKETCH_PROFILE_OVERLAPPING_CURVES")).unwrap();
        failure.diagnostics[0].evidence = Some(serde_json::json!({ "large": "x".repeat(1 << 20) }));
        assert!(EvidenceBudget::default().failure(&failure).is_err());
    }
    #[test]
    fn mixed_unknown_error_stays_fatal_while_real_advisories_are_retained() {
        let id = SketchId::new();
        let ApiError::OpFailed {
            message,
            diagnostics,
        } = error("SKETCH_PROFILE_OVERLAPPING_CURVES")
        else {
            panic!()
        };
        let unknown = Diagnostic {
            severity: Severity::Error,
            code: "INTEGRITY_BAD_HASH".into(),
            message: "source hash mismatch".into(),
            stage: Some("integrity".into()),
            reason_code: None,
            evidence: None,
        };
        let mut mixed = diagnostics.clone();
        mixed.push(unknown);
        assert!(
            recoverable(
                id,
                ApiError::OpFailed {
                    message: message.clone(),
                    diagnostics: mixed
                }
            )
            .is_err()
        );
        let advisory = Diagnostic {
            severity: Severity::Warning,
            code: "SKETCH_ENTITY_DEGENERATE".into(),
            message: "tiny edge omitted".into(),
            stage: Some("profile".into()),
            reason_code: None,
            evidence: None,
        };
        let mut mixed = diagnostics;
        mixed.push(advisory);
        assert_eq!(
            recoverable(
                id,
                ApiError::OpFailed {
                    message,
                    diagnostics: mixed
                }
            )
            .unwrap()
            .diagnostics
            .len(),
            2
        );
    }
}
