#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{
        DeterminismSettings, FeaturePatternLayout, FeaturePatternParams, FilletParams, HoleParams,
        HoleType, TransformBodyParams,
    };
    use crate::document::refs::{ElementKind, ElementRef, Extra, PrimaryRef};
    use crate::document::variables::Scalar;
    use crate::ids::DocumentId;
    use crate::math::Vec3;
    use uuid::Uuid;

    fn rid(n: u128) -> RecordId {
        RecordId(Uuid::from_u128(n))
    }

    fn opaque(id: u128, step: u32) -> OperationRecord {
        OperationRecord::new(
            rid(id),
            step,
            "source",
            Operation::Opaque(crate::document::record::OpaqueOperation {
                raw: serde_json::Map::from_iter([("opType".into(), json!("Alien"))]),
            }),
        )
    }

    fn pattern(ids: Vec<RecordId>) -> OperationRecord {
        OperationRecord::new(
            rid(99),
            1,
            "Pattern",
            Operation::Known(KnownOperation::FeaturePattern(FeaturePatternParams {
                source_record_ids: ids,
                layout: FeaturePatternLayout::Linear {
                    direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
                    spacing: Scalar::new(10.0),
                },
                count: 3,
                semantics_version: 1,
                extra: Extra::new(),
            })),
        )
    }

    fn transform(id: u128, body: BodyId, dx: f64) -> OperationRecord {
        let mut record = OperationRecord::new(
            rid(id),
            0,
            "move",
            Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
                targets: vec![body],
                translate: [Scalar::new(dx), Scalar::new(0.0), Scalar::new(0.0)],
                rotate: Default::default(),
                copy: false,
                extra: Extra::new(),
            })),
        );
        record.outputs.push(body);
        record
    }

    fn hole(id: u128, body: BodyId, diameter: f64) -> OperationRecord {
        let element = crate::ids::ElementId::new(format!("el_face_{id}"));
        let mut record = OperationRecord::new(
            rid(id),
            0,
            "hole",
            Operation::Known(KnownOperation::Hole(HoleParams {
                target_body: body,
                face: ElementRef {
                    primary: Some(PrimaryRef {
                        body,
                        element,
                        kind: ElementKind::Face,
                        extra: Extra::new(),
                    }),
                    intent: None,
                    anchor: None,
                    extra: Extra::new(),
                },
                point: Vec3::new(0.0, 0.0, 0.0).unwrap(),
                hole_type: HoleType::Simple,
                diameter: Scalar::new(diameter),
                depth: None,
                cb_diameter: None,
                cb_depth: None,
                cs_diameter: None,
                cs_angle_deg: None,
                thread: None,
                result_policy_version: Some(2),
                extra: Extra::new(),
            })),
        );
        record.outputs.push(body);
        record
    }

    #[test]
    fn rejects_missing_and_unsupported_sources_without_persisting_templates() {
        let source = opaque(1, 0);
        let mut records = vec![source, pattern(vec![rid(1)])];
        let errors = lower_feature_patterns(&mut records, 2);
        assert!(errors[&1].message.contains("does not support"));
        let Operation::Known(KnownOperation::FeaturePattern(p)) = &records[1].op else {
            panic!()
        };
        assert!(!p.extra.contains_key("sourceOps"));
        let _ = (DeterminismSettings::default(), DocumentId(Uuid::nil()));

        let body = BodyId(Uuid::from_u128(500));
        let mut records = vec![transform(2, body, 1.0), pattern(vec![rid(2)])];
        let errors = lower_feature_patterns(&mut records, 2);
        assert!(errors[&1].message.contains("TransformBody"));
        assert!(errors[&1].message.contains(&rid(2).to_string()));
    }

    #[test]
    fn unsupported_creator_position_reports_second_source() {
        let sketch_op: Operation = serde_json::from_value(json!({
            "opType": "Sketch",
            "params": {
                "sketchId": Uuid::from_u128(42),
                "plane": {
                    "kind": "XY",
                    "origin": [0.0, 0.0, 0.0],
                    "xAxis": [1.0, 0.0, 0.0],
                    "yAxis": [0.0, 1.0, 0.0],
                    "normal": [0.0, 0.0, 1.0]
                },
                "entities": [],
                "constraints": []
            }
        }))
        .unwrap();
        let sketch = OperationRecord::new(rid(1), 0, "sketch", sketch_op);
        let body = BodyId(Uuid::from_u128(500));
        let hole = hole(2, body, 2.0);
        let fillet = OperationRecord::new(
            rid(3),
            1,
            "fillet",
            Operation::Known(KnownOperation::Fillet(FilletParams {
                radius: Scalar::new(1.0),
                edge_ids: Vec::new(),
                edges: Vec::new(),
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Extra::new(),
            })),
        );
        for (wrong, expected) in [(&hole, "Hole"), (&fillet, "Fillet")] {
            let error = validate_executable_chain(&[&sketch, wrong]).unwrap_err();
            assert!(error.contains(&wrong.record_id.to_string()));
            assert!(error.contains(expected));
        }
    }

    #[test]
    fn rejects_noncanonical_source_order() {
        let mut records = vec![opaque(1, 0), opaque(2, 1), pattern(vec![rid(2), rid(1)])];
        let errors = lower_feature_patterns(&mut records, 3);
        assert!(errors[&2].message.contains("timeline order"));
    }

    #[test]
    fn missing_and_suppressed_sources_are_repair_failures() {
        let mut missing = vec![pattern(vec![rid(7)])];
        let errors = lower_feature_patterns(&mut missing, 1);
        assert_eq!(
            errors[&0].disposition,
            FeaturePatternFailureDisposition::NeedsRepair
        );
        assert_eq!(errors[&0].source_record_id, Some(rid(7)));

        let mut source = opaque(1, 0);
        source.suppressed = true;
        let mut suppressed = vec![source, pattern(vec![rid(1)])];
        let errors = lower_feature_patterns(&mut suppressed, 2);
        assert_eq!(
            errors[&1].disposition,
            FeaturePatternFailureDisposition::NeedsRepair
        );
        assert_eq!(errors[&1].source_record_id, Some(rid(1)));
    }

    #[test]
    fn reserved_source_ops_are_removed_even_when_lowering_fails() {
        let mut record = pattern(vec![rid(7)]);
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut record.op else {
            panic!()
        };
        params
            .extra
            .insert("sourceOps".into(), json!([{"opType":"ImportStep"}]));
        let mut records = vec![record];
        let errors = lower_feature_patterns(&mut records, 1);
        assert!(!errors.is_empty());
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &records[0].op else {
            panic!()
        };
        assert!(!params.extra.contains_key("sourceOps"));
    }

    #[test]
    fn effective_templates_follow_source_edits_without_mutating_persisted_pattern() {
        let body = BodyId(Uuid::from_u128(500));
        let stored = vec![hole(1, body, 2.0), pattern(vec![rid(1)])];
        let mut first = stored.clone();
        assert!(lower_feature_patterns(&mut first, 2).is_empty());
        let first_json = serde_json::to_value(&first[1]).unwrap();
        assert_eq!(
            first_json["params"]["sourceOps"][0]["params"]["diameter"]["value"],
            2.0
        );
        assert!(!serde_json::to_value(&stored[1]).unwrap()["params"]
            .get("sourceOps")
            .is_some());

        let mut edited = stored.clone();
        edited[0] = hole(1, body, 7.0);
        assert!(lower_feature_patterns(&mut edited, 2).is_empty());
        assert_eq!(
            serde_json::to_value(&edited[1]).unwrap()["params"]["sourceOps"][0]["params"]
                ["diameter"]["value"],
            7.0
        );
    }

    #[test]
    fn rejects_zero_layout_extent_before_worker() {
        let body = BodyId(Uuid::from_u128(500));
        let mut record = pattern(vec![rid(1)]);
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut record.op else {
            panic!()
        };
        params.layout = FeaturePatternLayout::Linear {
            direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
            spacing: Scalar::new(0.0),
        };
        let mut records = vec![transform(1, body, 2.0), record];
        assert!(lower_feature_patterns(&mut records, 2)[&1]
            .message
            .contains("nonzero"));
    }

    #[test]
    fn layout_accepts_signed_extent_and_rejects_subresolution_or_oversweep() {
        let body = BodyId(Uuid::from_u128(500));
        let mut signed = pattern(vec![rid(1)]);
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut signed.op else {
            panic!()
        };
        params.layout = FeaturePatternLayout::Linear {
            direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
            spacing: Scalar::new(-2.0),
        };
        let mut records = vec![hole(1, body, 2.0), signed];
        assert!(lower_feature_patterns(&mut records, 2).is_empty());

        let mut tiny = pattern(vec![rid(1)]);
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut tiny.op else {
            panic!()
        };
        params.layout = FeaturePatternLayout::Linear {
            direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
            spacing: Scalar::new(1.0e-10),
        };
        let mut records = vec![transform(1, body, 2.0), tiny];
        assert!(lower_feature_patterns(&mut records, 2)[&1]
            .message
            .contains("nonzero"));

        let mut sweep = pattern(vec![rid(1)]);
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut sweep.op else {
            panic!()
        };
        params.layout = FeaturePatternLayout::Circular {
            axis_origin: Vec3::new(0.0, 0.0, 0.0).unwrap(),
            axis_direction: Vec3::new(0.0, 0.0, 1.0).unwrap(),
            angle_deg: Scalar::new(-361.0),
        };
        let mut records = vec![transform(1, body, 2.0), sweep];
        assert!(lower_feature_patterns(&mut records, 2)[&1]
            .message
            .contains("angle"));
    }

    #[test]
    fn rejects_disconnected_and_multi_host_chains() {
        let a = BodyId(Uuid::from_u128(500));
        let b = BodyId(Uuid::from_u128(501));
        let disconnected = vec![
            transform(1, a, 1.0),
            transform(2, b, 1.0),
            pattern(vec![rid(1), rid(2)]),
        ];
        let mut records = disconnected;
        assert!(lower_feature_patterns(&mut records, 3)[&2]
            .message
            .contains("disconnected"));

        let first = transform(1, a, 1.0);
        let mut second = transform(2, a, 1.0);
        let Operation::Known(KnownOperation::TransformBody(params)) = &mut second.op else {
            panic!()
        };
        params.targets.push(b);
        second.inputs = second.op.derive_inputs();
        let mut records = vec![first, second, pattern(vec![rid(1), rid(2)])];
        assert!(lower_feature_patterns(&mut records, 3)[&2]
            .message
            .contains("more than one external body"));
    }
}
