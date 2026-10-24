//! The initial solid Loft contract keeps section identity and authored order explicit.
use std::collections::BTreeSet;

use super::{BooleanMode, LoftParams};

pub const LOFT_MAX_PROFILES: usize = 32;

impl LoftParams {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=LOFT_MAX_PROFILES).contains(&self.profiles.len()) {
            return Err("Loft requires 2–32 ordered closed profiles".into());
        }
        if !self.is_solid || self.boolean_mode != BooleanMode::NewBody {
            return Err("Loft supports solid NewBody results only".into());
        }
        for key in [
            "sections",
            "guideRails",
            "guides",
            "rails",
            "spine",
            "startCondition",
            "endCondition",
            "continuity",
            "points",
        ] {
            if self.extra.contains_key(key) {
                return Err(format!("Loft {key} is unsupported"));
            }
        }
        let mut seen = BTreeSet::new();
        for profile in &self.profiles {
            if profile.sketch.0.is_nil() || profile.region.as_str().is_empty() {
                return Err("Loft requires explicit sketch and region identities".into());
            }
            if !seen.insert((profile.sketch, profile.region.as_str())) {
                return Err("Loft section is selected more than once".into());
            }
            if profile
                .region_anchor
                .is_some_and(|a| a.iter().any(|v| !v.is_finite()))
            {
                return Err("Loft section anchor must be finite".into());
            }
            if profile
                .region_identity_version
                .is_some_and(|v| !matches!(v, 2 | 3))
            {
                return Err("Loft region identity version is unsupported".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::refs::SketchRegionRef,
        ids::{RegionId, SketchId},
    };

    fn params() -> LoftParams {
        LoftParams {
            profiles: (0..3)
                .map(|i| SketchRegionRef {
                    sketch: SketchId::new(),
                    region: RegionId::new(format!("r{i}")),
                    region_identity_version: Some(2),
                    region_anchor: Some([1.0, 2.0]),
                    extra: Default::default(),
                })
                .collect(),
            is_solid: true,
            is_ruled: false,
            boolean_mode: BooleanMode::NewBody,
            extra: Default::default(),
        }
    }

    #[test]
    fn solid_sections_keep_order_and_round_trip_without_schema_changes() {
        let mut p = params();
        p.validate().unwrap();
        let encoded = serde_json::to_value(&p).unwrap();
        assert_eq!(serde_json::from_value::<LoftParams>(encoded).unwrap(), p);
        p.profiles.reverse();
        p.is_ruled = true;
        p.validate().unwrap();
    }

    #[test]
    fn invalid_and_unsupported_sections_refuse_before_preview() {
        let original = params();
        for count in [0, 1, 33] {
            let mut p = original.clone();
            p.profiles = (0..count).map(|_| original.profiles[0].clone()).collect();
            assert!(p.validate().is_err());
        }
        let mut p = original.clone();
        p.profiles[1] = p.profiles[0].clone();
        assert!(p.validate().unwrap_err().contains("more than once"));
        p = original.clone();
        p.is_solid = false;
        assert!(p.validate().is_err());
        p = original.clone();
        p.boolean_mode = BooleanMode::Cut;
        assert!(p.validate().is_err());
        p = original.clone();
        p.profiles[0].region_anchor = Some([f64::NAN, 0.0]);
        assert!(p.validate().is_err());
        p = original.clone();
        p.profiles[0].region_identity_version = Some(99);
        assert!(p.validate().is_err());
        p.profiles[0].region_identity_version = Some(1);
        assert!(p.validate().is_err());
        p = original;
        p.extra.insert("guideRails".into(), serde_json::json!([]));
        assert!(p.validate().unwrap_err().contains("guideRails"));
    }

    #[test]
    fn document_commands_refuse_invalid_loft_without_history_mutation() {
        use crate::{
            document::{
                record::{KnownOperation, Operation, OperationRecord},
                Document,
            },
            edit::{session::DocumentSession, EditCommand},
            ids::{DocumentId, RecordId},
        };
        let mut session = DocumentSession::new(Document::new(DocumentId::new()));
        let id = RecordId::new();
        let mut p = params();
        p.profiles.clear();
        let invalid =
            OperationRecord::new(id, 0, "Loft", Operation::Known(KnownOperation::Loft(p)));
        assert!(session
            .apply(EditCommand::AddOperation {
                record: invalid,
                at_cursor: true
            })
            .is_err());
        assert!(session.document().timeline.records().is_empty());
        let valid = OperationRecord::new(
            id,
            0,
            "Loft",
            Operation::Known(KnownOperation::Loft(params())),
        );
        session
            .apply(EditCommand::AddOperation {
                record: valid.clone(),
                at_cursor: true,
            })
            .unwrap();
        let mut p = params();
        p.is_solid = false;
        assert!(session
            .apply(EditCommand::UpdateOperationParams {
                record: id,
                op: Operation::Known(KnownOperation::Loft(p))
            })
            .is_err());
        assert_eq!(session.document().timeline.records(), &[valid]);
    }

    #[test]
    fn legacy_unsupported_loft_records_remain_readable_and_preserve_unknown_fields() {
        use crate::{
            document::{
                record::{KnownOperation, Operation, OperationRecord},
                Document,
            },
            edit::{session::DocumentSession, EditCommand},
            ids::{DocumentId, RecordId},
        };
        let mut legacy = params();
        legacy.is_solid = false;
        legacy.boolean_mode = BooleanMode::Cut;
        legacy.profiles[0].region = RegionId::new("");
        legacy.profiles[0].region_identity_version = None;
        legacy.extra.insert(
            "guideRails".into(),
            serde_json::json!([{"legacyRail": "rail-1"}]),
        );
        let mut record = OperationRecord::new(
            RecordId::new(),
            0,
            "Legacy unsupported Loft",
            Operation::Known(KnownOperation::Loft(legacy)),
        );
        record
            .extra
            .insert("legacyEvidence".into(), serde_json::json!({"value": 7}));
        let encoded = serde_json::to_string(&record).unwrap();
        let restored: OperationRecord = serde_json::from_str(&encoded).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), encoded);
        assert_eq!(restored, record);
        let Operation::Known(KnownOperation::Loft(params)) = &restored.op else {
            panic!("legacy Loft stays typed");
        };
        assert!(params.validate().is_err());
        let mut session = DocumentSession::new(Document::new(DocumentId::new()));
        assert!(session
            .apply(EditCommand::AddOperation {
                record: restored,
                at_cursor: true
            })
            .is_err());
        assert!(session.document().timeline.records().is_empty());
    }
}
